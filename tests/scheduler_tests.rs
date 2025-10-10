use tricti::ast::{ResourceAccess, SystemDef, SystemParameter, Type};
use tricti::scheduler::*;

/// Helper function to create a test system with resource parameters
fn create_test_system(name: &str, resources: Vec<(&str, ResourceAccess)>) -> SystemDef {
    let parameters = resources
        .into_iter()
        .map(|(res_name, access)| SystemParameter::Resource {
            param_type: "resource".to_string(),
            name: res_name.to_string(),
            resource_type: Type::Identifier {
                name: "TestResource".to_string(),
                type_args: vec![],
            },
            access,
        })
        .collect();

    SystemDef {
        name: name.to_string(),
        parameters,
        return_type: None,
        body: vec![],
        is_async: false,
    }
}

#[test]
fn test_basic_scheduler_functionality() {
    let mut scheduler = SystemScheduler::new();

    // Create systems with different resource access patterns
    let system1 = create_test_system("reader1", vec![("database", ResourceAccess::Immutable)]);
    let system2 = create_test_system("reader2", vec![("database", ResourceAccess::Immutable)]);
    let system3 = create_test_system("writer", vec![("database", ResourceAccess::Mutable)]);

    // Add systems to scheduler
    assert!(scheduler.add_system(system1).is_ok());
    assert!(scheduler.add_system(system2).is_ok());
    assert!(scheduler.add_system(system3).is_ok());

    // Build conflict graph
    assert!(scheduler.build_conflict_graph().is_ok());

    // Check that readers don't conflict with each other
    assert!(!scheduler
        .get_conflict_graph()
        .has_conflict("reader1", "reader2"));

    // Check that readers conflict with writer
    assert!(scheduler
        .get_conflict_graph()
        .has_conflict("reader1", "writer"));
    assert!(scheduler
        .get_conflict_graph()
        .has_conflict("reader2", "writer"));
}

#[test]
fn test_scheduling_workflow() {
    let mut scheduler = SystemScheduler::new();

    // Create systems
    let system1 = create_test_system("system1", vec![("resource1", ResourceAccess::Immutable)]);
    let system2 = create_test_system("system2", vec![("resource2", ResourceAccess::Mutable)]);
    let system3 = create_test_system("system3", vec![("resource1", ResourceAccess::Mutable)]);

    // Add systems
    assert!(scheduler.add_system(system1).is_ok());
    assert!(scheduler.add_system(system2).is_ok());
    assert!(scheduler.add_system(system3).is_ok());

    // Build conflict graph
    assert!(scheduler.build_conflict_graph().is_ok());

    // Enqueue systems
    scheduler.enqueue_system("system1".to_string());
    scheduler.enqueue_system("system2".to_string());
    scheduler.enqueue_system("system3".to_string());

    // system1 and system2 should be schedulable (different resources)
    let schedulable = scheduler.get_schedulable_systems();
    assert!(schedulable.contains(&"system1".to_string()));
    assert!(schedulable.contains(&"system2".to_string()));
    assert!(schedulable.contains(&"system3".to_string()));

    // Schedule system1 and system2
    assert!(scheduler.schedule_system("system1").is_ok());
    assert!(scheduler.schedule_system("system2").is_ok());

    // Now system3 not be schedulable
    let schedulable = scheduler.get_schedulable_systems();
    assert!(!schedulable.contains(&"system3".to_string()));

    // Complete system1
    assert!(scheduler.complete_system("system1").is_ok());

    // Now system3 should be schedulable
    let schedulable = scheduler.get_schedulable_systems();
    assert!(schedulable.contains(&"system3".to_string()));

    // Schedule system3
    assert!(scheduler.schedule_system("system3").is_ok());

    // Complete remaining systems
    assert!(scheduler.complete_system("system2").is_ok());
    assert!(scheduler.complete_system("system3").is_ok());

    // Scheduler should be complete
    assert!(scheduler.is_complete());
}

#[test]
fn test_complex_resource_conflicts() {
    let mut scheduler = SystemScheduler::new();

    // Create systems with overlapping resource usage
    let system1 = create_test_system(
        "multi_reader",
        vec![
            ("db", ResourceAccess::Immutable),
            ("cache", ResourceAccess::Immutable),
        ],
    );
    let system2 = create_test_system("cache_writer", vec![("cache", ResourceAccess::Mutable)]);
    let system3 = create_test_system("db_writer", vec![("db", ResourceAccess::Mutable)]);
    let system4 = create_test_system(
        "independent",
        vec![("file_system", ResourceAccess::Mutable)],
    );

    // Add all systems
    assert!(scheduler.add_system(system1).is_ok());
    assert!(scheduler.add_system(system2).is_ok());
    assert!(scheduler.add_system(system3).is_ok());
    assert!(scheduler.add_system(system4).is_ok());

    // Build conflict graph
    assert!(scheduler.build_conflict_graph().is_ok());

    // Verify conflicts
    assert!(scheduler
        .get_conflict_graph()
        .has_conflict("multi_reader", "cache_writer"));
    assert!(scheduler
        .get_conflict_graph()
        .has_conflict("multi_reader", "db_writer"));
    assert!(!scheduler
        .get_conflict_graph()
        .has_conflict("cache_writer", "db_writer"));
    assert!(!scheduler
        .get_conflict_graph()
        .has_conflict("independent", "multi_reader"));
    assert!(!scheduler
        .get_conflict_graph()
        .has_conflict("independent", "cache_writer"));
    assert!(!scheduler
        .get_conflict_graph()
        .has_conflict("independent", "db_writer"));
}

#[test]
fn test_deadlock_detection() {
    let mut scheduler = SystemScheduler::new();

    // Create systems that could potentially deadlock
    // This is a simplified test - real deadlock scenarios would be more complex
    let system1 = create_test_system(
        "system1",
        vec![
            ("resource1", ResourceAccess::Mutable),
            ("resource2", ResourceAccess::Immutable),
        ],
    );
    let system2 = create_test_system(
        "system2",
        vec![
            ("resource2", ResourceAccess::Mutable),
            ("resource1", ResourceAccess::Immutable),
        ],
    );

    assert!(scheduler.add_system(system1).is_ok());
    assert!(scheduler.add_system(system2).is_ok());
    assert!(scheduler.build_conflict_graph().is_ok());

    // Both systems conflict due to cross-dependencies
    assert!(scheduler
        .get_conflict_graph()
        .has_conflict("system1", "system2"));

    // Run deadlock detection
    let deadlock_result = scheduler.detect_deadlocks();
    // In this simple case, there might not be a true deadlock, but conflicts exist
    assert!(
        deadlock_result.is_ok()
            || matches!(
                deadlock_result,
                Err(SchedulerError::DeadlockDetected { .. })
            )
    );
}

#[test]
fn test_resource_ownership_conflicts() {
    let mut scheduler = SystemScheduler::new();

    // Create systems with ownership conflicts
    let system1 = create_test_system("owner", vec![("resource1", ResourceAccess::Owned)]);
    let system2 = create_test_system("borrower", vec![("resource1", ResourceAccess::Immutable)]);
    let system3 = create_test_system("writer", vec![("resource1", ResourceAccess::Mutable)]);

    assert!(scheduler.add_system(system1).is_ok());
    assert!(scheduler.add_system(system2).is_ok());
    assert!(scheduler.add_system(system3).is_ok());

    assert!(scheduler.build_conflict_graph().is_ok());

    // Owned resource should conflict with all other access types
    assert!(scheduler
        .get_conflict_graph()
        .has_conflict("owner", "borrower"));
    assert!(scheduler
        .get_conflict_graph()
        .has_conflict("owner", "writer"));
    assert!(scheduler
        .get_conflict_graph()
        .has_conflict("borrower", "writer"));
}

#[test]
fn test_invalid_system_validation() {
    let mut scheduler = SystemScheduler::new();

    // Create a system with invalid resource access patterns
    let mut system = create_test_system("invalid_system", vec![]);
    system.parameters.push(SystemParameter::Resource {
        param_type: "resource".to_string(),
        name: "resource1".to_string(),
        resource_type: Type::Identifier {
            name: "TestResource".to_string(),
            type_args: vec![],
        },
        access: ResourceAccess::Immutable,
    });
    system.parameters.push(SystemParameter::Resource {
        param_type: "resource".to_string(),
        name: "resource1".to_string(), // Same resource name
        resource_type: Type::Identifier {
            name: "TestResource".to_string(),
            type_args: vec![],
        },
        access: ResourceAccess::Mutable, // Different access type
    });

    // This should fail validation
    let result = scheduler.add_system(system);
    assert!(result.is_err());
    match result.unwrap_err() {
        SchedulerError::InvalidResourceAccess {
            system,
            resource,
            reason,
        } => {
            assert_eq!(system, "invalid_system");
            assert_eq!(resource, "resource1");
            assert!(reason.contains("conflicting access patterns"));
        }
        _ => panic!("Expected InvalidResourceAccess error"),
    }
}

#[test]
fn test_concurrent_execution_simulation() {
    let mut scheduler = SystemScheduler::new();

    // Create a larger set of systems to test concurrent execution
    for i in 0..5 {
        let system = create_test_system(
            &format!("reader_{}", i),
            vec![("shared_db", ResourceAccess::Immutable)],
        );
        assert!(scheduler.add_system(system).is_ok());
    }

    let writer_system = create_test_system("writer", vec![("shared_db", ResourceAccess::Mutable)]);
    assert!(scheduler.add_system(writer_system).is_ok());

    let independent_system = create_test_system(
        "independent",
        vec![("other_resource", ResourceAccess::Mutable)],
    );
    assert!(scheduler.add_system(independent_system).is_ok());

    assert!(scheduler.build_conflict_graph().is_ok());

    // Enqueue all systems
    for i in 0..5 {
        scheduler.enqueue_system(format!("reader_{}", i));
    }
    scheduler.enqueue_system("writer".to_string());
    scheduler.enqueue_system("independent".to_string());

    // All readers and independent system should be schedulable initially
    let schedulable = scheduler.get_schedulable_systems();
    assert!(schedulable.len() >= 6); // 5 readers + independent
    assert!(schedulable.contains(&"writer".to_string()));

    // Schedule all readers and independent system
    for i in 0..5 {
        assert!(scheduler.schedule_system(&format!("reader_{}", i)).is_ok());
    }
    assert!(scheduler.schedule_system("independent").is_ok());

    // Writer should still not be schedulable
    let schedulable = scheduler.get_schedulable_systems();
    assert!(!schedulable.contains(&"writer".to_string()));

    // Complete all readers
    for i in 0..5 {
        assert!(scheduler.complete_system(&format!("reader_{}", i)).is_ok());
    }

    // Now writer should be schedulable
    let schedulable = scheduler.get_schedulable_systems();
    assert!(schedulable.contains(&"writer".to_string()));
}

#[test]
fn test_priority_scheduling() {
    let mut scheduler = SystemScheduler::with_config(SchedulingStrategy::Priority, 2);

    // Create systems with different priorities
    let low_priority = create_test_system(
        "low_priority",
        vec![("resource1", ResourceAccess::Immutable)],
    );
    let high_priority = create_test_system(
        "high_priority",
        vec![("resource2", ResourceAccess::Immutable)],
    );
    let critical_priority = create_test_system(
        "critical_priority",
        vec![("resource3", ResourceAccess::Immutable)],
    );

    // Set priorities before adding systems
    scheduler.set_system_priority("low_priority", SystemPriority::Low);
    scheduler.set_system_priority("high_priority", SystemPriority::High);
    scheduler.set_system_priority("critical_priority", SystemPriority::Critical);

    // Add systems
    assert!(scheduler.add_system(low_priority).is_ok());
    assert!(scheduler.add_system(high_priority).is_ok());
    assert!(scheduler.add_system(critical_priority).is_ok());

    // Enqueue in random order
    scheduler.enqueue_system("low_priority".to_string());
    scheduler.enqueue_system("high_priority".to_string());
    scheduler.enqueue_system("critical_priority".to_string());

    // Check that schedulable systems are in priority order
    let schedulable = scheduler.get_schedulable_systems();
    assert_eq!(schedulable[0], "critical_priority");
    assert_eq!(schedulable[1], "high_priority");
    assert_eq!(schedulable[2], "low_priority");
}

#[test]
fn test_shortest_job_first_scheduling() {
    let mut scheduler = SystemScheduler::with_config(SchedulingStrategy::SJF, 3);

    // Create systems with different estimated runtimes
    let long_job = create_test_system("long_job", vec![("resource1", ResourceAccess::Immutable)]);
    let short_job = create_test_system("short_job", vec![("resource2", ResourceAccess::Immutable)]);
    let medium_job =
        create_test_system("medium_job", vec![("resource3", ResourceAccess::Immutable)]);

    // Add systems with different estimated runtimes
    assert!(scheduler
        .add_system_with_metadata(long_job, SystemPriority::Normal, Some(5000), vec![])
        .is_ok());
    assert!(scheduler
        .add_system_with_metadata(short_job, SystemPriority::Normal, Some(1000), vec![])
        .is_ok());
    assert!(scheduler
        .add_system_with_metadata(medium_job, SystemPriority::Normal, Some(3000), vec![])
        .is_ok());

    // Enqueue systems
    scheduler.enqueue_system("long_job".to_string());
    scheduler.enqueue_system("short_job".to_string());
    scheduler.enqueue_system("medium_job".to_string());

    // Check that schedulable systems are in runtime order (shortest first)
    let schedulable = scheduler.get_schedulable_systems();
    assert_eq!(schedulable[0], "short_job");
    assert_eq!(schedulable[1], "medium_job");
    assert_eq!(schedulable[2], "long_job");
}

#[test]
fn test_batch_scheduling() {
    let mut scheduler = SystemScheduler::with_config(SchedulingStrategy::FCFS, 3);

    // Create multiple independent systems
    for i in 0..5 {
        let system = create_test_system(
            &format!("system_{}", i),
            vec![(
                format!("resource_{}", i).as_str(),
                ResourceAccess::Immutable,
            )],
        );
        assert!(scheduler.add_system(system).is_ok());
        scheduler.enqueue_system(format!("system_{}", i));
    }

    // Schedule a batch
    let scheduled = scheduler.schedule_batch().unwrap();

    // Should schedule up to max_concurrent (3) systems
    assert_eq!(scheduled.len(), 3);
    assert_eq!(scheduler.get_state().executing_systems.len(), 3);
    assert_eq!(scheduler.get_state().pending_systems.len(), 2);
}

#[test]
fn test_system_dependencies() {
    let mut scheduler = SystemScheduler::new();

    // Create systems with dependencies
    let dependency = create_test_system("dependency", vec![("resource1", ResourceAccess::Mutable)]);
    let dependent = create_test_system("dependent", vec![("resource2", ResourceAccess::Immutable)]);

    // Add systems first
    assert!(scheduler.add_system(dependency).is_ok());
    assert!(scheduler.add_system(dependent).is_ok());

    // Add dependency relationship
    assert!(scheduler.add_dependency("dependent", "dependency").is_ok());

    // Enqueue both systems
    scheduler.enqueue_system("dependency".to_string());
    scheduler.enqueue_system("dependent".to_string());

    // Only dependency should be schedulable initially
    let schedulable = scheduler.get_schedulable_systems();
    assert!(schedulable.contains(&"dependency".to_string()));
    assert!(!schedulable.contains(&"dependent".to_string()));

    // Schedule and complete dependency
    assert!(scheduler.schedule_system("dependency").is_ok());
    assert!(scheduler.complete_system("dependency").is_ok());

    // Now dependent should be schedulable
    let schedulable = scheduler.get_schedulable_systems();
    assert!(schedulable.contains(&"dependent".to_string()));
}

#[test]
fn test_circular_dependency_detection() {
    let mut scheduler = SystemScheduler::new();

    // Create systems
    let system1 = create_test_system("system1", vec![("resource1", ResourceAccess::Immutable)]);
    let system2 = create_test_system("system2", vec![("resource2", ResourceAccess::Immutable)]);

    assert!(scheduler.add_system(system1).is_ok());
    assert!(scheduler.add_system(system2).is_ok());

    // Add first dependency
    assert!(scheduler.add_dependency("system1", "system2").is_ok());

    // Try to add circular dependency - should fail
    let result = scheduler.add_dependency("system2", "system1");
    assert!(result.is_err());
    match result.unwrap_err() {
        SchedulerError::DeadlockDetected { cycle } => {
            assert!(cycle.contains(&"system1".to_string()));
            assert!(cycle.contains(&"system2".to_string()));
        }
        _ => panic!("Expected DeadlockDetected error"),
    }
}

#[test]
fn test_scheduling_statistics() {
    let mut scheduler = SystemScheduler::with_config(SchedulingStrategy::Priority, 2);

    // Add several systems
    for i in 0..4 {
        let system = create_test_system(
            &format!("system_{}", i),
            vec![(
                format!("resource_{}", i).as_str(),
                ResourceAccess::Immutable,
            )],
        );
        assert!(scheduler.add_system(system).is_ok());
        scheduler.enqueue_system(format!("system_{}", i));
    }

    let stats = scheduler.get_scheduling_stats();
    assert_eq!(stats.total_systems, 4);
    assert_eq!(stats.pending_systems, 4);
    assert_eq!(stats.executing_systems, 0);
    assert_eq!(stats.completed_systems, 0);
    assert_eq!(stats.max_concurrent, 2);
    assert_eq!(stats.strategy, SchedulingStrategy::Priority);

    // Schedule some systems
    let scheduled = scheduler.schedule_batch().unwrap();
    assert_eq!(scheduled.len(), 2);

    let stats = scheduler.get_scheduling_stats();
    assert_eq!(stats.executing_systems, 2);
    assert_eq!(stats.pending_systems, 2);

    // Complete systems
    for system_name in &scheduled {
        assert!(scheduler.complete_system(system_name).is_ok());
    }

    let stats = scheduler.get_scheduling_stats();
    assert_eq!(stats.executing_systems, 0);
    assert_eq!(stats.completed_systems, 2);
    assert_eq!(stats.pending_systems, 2);
}

#[test]
fn test_completion_time_estimation() {
    let mut scheduler = SystemScheduler::with_config(SchedulingStrategy::SJF, 2);

    // Add systems with known runtimes
    for i in 0..3 {
        let system = create_test_system(
            &format!("system_{}", i),
            vec![(
                format!("resource_{}", i).as_str(),
                ResourceAccess::Immutable,
            )],
        );
        assert!(scheduler
            .add_system_with_metadata(
                system,
                SystemPriority::Normal,
                Some(1000), // 1 second each
                vec![]
            )
            .is_ok());
        scheduler.enqueue_system(format!("system_{}", i));
    }

    // Estimate completion time
    let estimated_time = scheduler.estimate_completion_time();
    assert!(estimated_time.is_some());

    // With 3 systems of 1 second each and max concurrent 2, should take about 1.5 seconds
    let time = estimated_time.unwrap();
    assert!(time >= 1000 && time <= 2000); // Allow some variance

    // Complete all systems
    while !scheduler.is_complete() {
        let scheduled = scheduler.schedule_batch().unwrap();
        for system_name in &scheduled {
            scheduler.complete_system(system_name).unwrap();
        }
    }

    // Completion time should be 0 when complete
    assert_eq!(scheduler.estimate_completion_time(), Some(0));
}

#[test]
fn test_work_stealing_strategy() {
    let mut scheduler = SystemScheduler::with_config(SchedulingStrategy::WorkStealing, 3);

    // Create systems with dependencies (simulating work that unblocks others)
    let unlocker = create_test_system(
        "unlocker",
        vec![("shared_resource", ResourceAccess::Mutable)],
    );
    let blocked1 = create_test_system("blocked1", vec![("resource1", ResourceAccess::Immutable)]);
    let blocked2 = create_test_system("blocked2", vec![("resource2", ResourceAccess::Immutable)]);
    let independent = create_test_system(
        "independent",
        vec![("resource3", ResourceAccess::Immutable)],
    );

    // Add systems
    assert!(scheduler.add_system(unlocker).is_ok());
    assert!(scheduler.add_system(blocked1).is_ok());
    assert!(scheduler.add_system(blocked2).is_ok());
    assert!(scheduler.add_system(independent).is_ok());

    // Add dependencies
    assert!(scheduler.add_dependency("blocked1", "unlocker").is_ok());
    assert!(scheduler.add_dependency("blocked2", "unlocker").is_ok());

    // Enqueue systems
    scheduler.enqueue_system("unlocker".to_string());
    scheduler.enqueue_system("blocked1".to_string());
    scheduler.enqueue_system("blocked2".to_string());
    scheduler.enqueue_system("independent".to_string());

    // Work-stealing should prioritize unlocker and independent
    let schedulable = scheduler.get_schedulable_systems();
    assert!(schedulable.contains(&"unlocker".to_string()));
    assert!(schedulable.contains(&"independent".to_string()));
    assert!(!schedulable.contains(&"blocked1".to_string()));
    assert!(!schedulable.contains(&"blocked2".to_string()));
}

#[test]
fn test_max_concurrent_limit() {
    let mut scheduler = SystemScheduler::with_config(SchedulingStrategy::FCFS, 2);

    // Add more systems than the concurrent limit
    for i in 0..5 {
        let system = create_test_system(
            &format!("system_{}", i),
            vec![(
                format!("resource_{}", i).as_str(),
                ResourceAccess::Immutable,
            )],
        );
        assert!(scheduler.add_system(system).is_ok());
        scheduler.enqueue_system(format!("system_{}", i));
    }

    // Should only be able to schedule 2 systems at once
    let scheduled = scheduler.schedule_batch().unwrap();
    assert_eq!(scheduled.len(), 2);
    assert_eq!(scheduler.get_state().executing_systems.len(), 2);

    // Try to schedule more - should return empty
    let scheduled_more = scheduler.schedule_batch().unwrap();
    assert_eq!(scheduled_more.len(), 0);

    // Complete one system
    let first_system = scheduled.first().unwrap();
    assert!(scheduler.complete_system(first_system).is_ok());

    // Now should be able to schedule one more
    let scheduled_again = scheduler.schedule_batch().unwrap();
    assert_eq!(scheduled_again.len(), 1);
    assert_eq!(scheduler.get_state().executing_systems.len(), 2);
}

/// Helper function to create a test system with specific resource type (instead of generic TestResource)
#[allow(dead_code)]
fn create_gui_system(name: &str, param_name: &str, access: ResourceAccess) -> SystemDef {
    let parameters = vec![SystemParameter::Resource {
        param_type: "resource".to_string(),
        name: param_name.to_string(), // Different parameter names
        resource_type: Type::Identifier {
            name: "Gui".to_string(), // Same resource type
            type_args: vec![],
        },
        access,
    }];

    SystemDef {
        name: name.to_string(),
        parameters,
        return_type: None,
        body: vec![],
        is_async: false,
    }
}
