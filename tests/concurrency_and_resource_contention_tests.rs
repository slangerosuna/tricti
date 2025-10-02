use tricti::{
    ast::*, async_runtime::*, async_scheduler_integration::*, scheduler::*,
    semantic::SemanticContext, system_executor::*, table_runtime::*,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Focused integration tests for concurrent systems and resource contention scenarios
/// These tests verify that the async execution model properly handles:
/// 1. Concurrent system execution without conflicts
/// 2. Resource contention and borrow safety guarantees
/// 3. Proper scheduling and conflict resolution
#[cfg(test)]
mod concurrency_resource_tests {
    use super::*;

    /// Test concurrent execution of non-conflicting systems
    #[test]
    fn test_concurrent_non_conflicting_systems() {
        let runtime_config = RuntimeConfig::default();
        let semantic_context = create_test_semantic_context();
        let scheduler = AsyncSystemScheduler::new(runtime_config, semantic_context);

        // Create systems that don't share resources
        let system1 = create_system_with_resources("system1", vec!["resource_a"]);
        let system2 = create_system_with_resources("system2", vec!["resource_b"]);
        let system3 = create_system_with_resources("system3", vec!["resource_c"]);

        let requests = vec![
            create_system_request(system1),
            create_system_request(system2),
            create_system_request(system3),
        ];

        // All systems should be schedulable concurrently since no resource conflicts
        let start_time = Instant::now();
        let schedule_result = tokio_test::block_on(scheduler.schedule_systems(requests));
        let scheduling_time = start_time.elapsed();

        assert!(
            schedule_result.is_ok(),
            "Non-conflicting systems should schedule successfully"
        );
        let futures = schedule_result.unwrap();
        assert_eq!(futures.len(), 3, "All systems should be scheduled");

        // Scheduling should be fast for non-conflicting systems
        assert!(
            scheduling_time < Duration::from_millis(100),
            "Scheduling should be efficient"
        );

        let stats = scheduler.get_stats();
        assert_eq!(stats.total_systems_scheduled, 3);
        assert!(stats.concurrent_executions >= 1);
    }

    /// Test resource contention with immutable borrows
    #[test]
    fn test_immutable_borrow_contention() {
        let mut tracker = ResourceTracker::new();

        // Multiple systems requesting immutable access to same resource
        let result1 = tracker.add_access("shared_resource", "reader1", &ResourceAccess::Immutable);
        let result2 = tracker.add_access("shared_resource", "reader2", &ResourceAccess::Immutable);
        let result3 = tracker.add_access("shared_resource", "reader3", &ResourceAccess::Immutable);

        assert!(result1.is_ok(), "First immutable access should succeed");
        assert!(result2.is_ok(), "Second immutable access should succeed");
        assert!(result3.is_ok(), "Third immutable access should succeed");

        // Verify all readers are tracked
        let borrowers = tracker.get_immutable_borrowers("shared_resource").unwrap();
        assert_eq!(borrowers.len(), 3);
        assert!(borrowers.contains("reader1"));
        assert!(borrowers.contains("reader2"));
        assert!(borrowers.contains("reader3"));

        // Now try mutable access - should fail
        let mutable_result =
            tracker.add_access("shared_resource", "writer", &ResourceAccess::Mutable);
        assert!(
            mutable_result.is_err(),
            "Mutable access should fail when immutable borrows exist"
        );

        match mutable_result.unwrap_err() {
            SchedulerError::ResourceConflict { conflict_type, .. } => {
                assert_eq!(conflict_type, ConflictType::WriteRead);
            }
            _ => panic!("Expected ResourceConflict error"),
        }
    }

    /// Test resource contention with mutable borrows
    #[test]
    fn test_mutable_borrow_contention() {
        let mut tracker = ResourceTracker::new();

        // First system gets mutable access
        let result1 = tracker.add_access("exclusive_resource", "writer1", &ResourceAccess::Mutable);
        assert!(result1.is_ok(), "First mutable access should succeed");

        assert_eq!(
            tracker.get_mutable_borrower("exclusive_resource"),
            Some(&"writer1".to_string())
        );

        // Second system tries mutable access - should fail
        let result2 = tracker.add_access("exclusive_resource", "writer2", &ResourceAccess::Mutable);
        assert!(result2.is_err(), "Second mutable access should fail");

        match result2.unwrap_err() {
            SchedulerError::DuplicateMutableBorrow {
                system1,
                system2,
                resource,
            } => {
                assert_eq!(system1, "writer2");
                assert_eq!(system2, "writer1");
                assert_eq!(resource, "exclusive_resource");
            }
            _ => panic!("Expected DuplicateMutableBorrow error"),
        }

        // Third system tries immutable access - should also fail
        let result3 =
            tracker.add_access("exclusive_resource", "reader1", &ResourceAccess::Immutable);
        assert!(
            result3.is_err(),
            "Immutable access should fail when mutable borrow exists"
        );
    }

    /// Test write-after-read conflicts
    #[test]
    fn test_write_after_read_conflict() {
        let runtime_config = RuntimeConfig::default();
        let semantic_context = create_test_semantic_context();
        let scheduler = AsyncSystemScheduler::new(runtime_config, semantic_context);

        // System 1: Reader
        let reader_system = create_system_with_resource_access(
            "reader_system",
            vec![("data_resource", ResourceAccess::Immutable)],
        );

        // System 2: Writer (should conflict)
        let writer_system = create_system_with_resource_access(
            "writer_system",
            vec![("data_resource", ResourceAccess::Mutable)],
        );

        let requests = vec![
            create_system_request(reader_system),
            create_system_request(writer_system),
        ];

        let schedule_result = tokio_test::block_on(scheduler.schedule_systems(requests));
        assert!(
            schedule_result.is_ok(),
            "Scheduler should handle conflicts gracefully"
        );

        let futures = schedule_result.unwrap();
        assert_eq!(
            futures.len(),
            2,
            "Both systems should be scheduled (with ordering)"
        );

        // Verify conflict was detected and resolved
        let stats = scheduler.get_stats();
        assert!(stats.resource_conflicts_resolved >= 1);
    }

    /// Test read-after-write conflicts
    #[test]
    fn test_read_after_write_conflict() {
        let mut tracker = ResourceTracker::new();

        // Writer gets access first
        let writer_result =
            tracker.add_access("contested_resource", "writer", &ResourceAccess::Mutable);
        assert!(writer_result.is_ok());

        // Reader tries to access - should fail
        let reader_result =
            tracker.add_access("contested_resource", "reader", &ResourceAccess::Immutable);
        assert!(reader_result.is_err());

        match reader_result.unwrap_err() {
            SchedulerError::ResourceConflict { conflict_type, .. } => {
                assert_eq!(conflict_type, ConflictType::ReadWrite);
            }
            _ => panic!("Expected ReadWrite conflict"),
        }

        // After writer releases, reader should succeed
        tracker.remove_access("contested_resource", "writer", &ResourceAccess::Mutable);

        let reader_retry =
            tracker.add_access("contested_resource", "reader", &ResourceAccess::Immutable);
        assert!(
            reader_retry.is_ok(),
            "Reader should succeed after writer releases"
        );
    }

    /// Test complex resource dependency chains
    #[test]
    fn test_complex_resource_chains() {
        let mut tracker = ResourceTracker::new();

        // System A: needs resources X and Y (immutable)
        assert!(tracker
            .add_access("resource_x", "system_a", &ResourceAccess::Immutable)
            .is_ok());
        assert!(tracker
            .add_access("resource_y", "system_a", &ResourceAccess::Immutable)
            .is_ok());

        // System B: needs resource Y (immutable) and Z (mutable)
        assert!(tracker
            .add_access("resource_y", "system_b", &ResourceAccess::Immutable)
            .is_ok());
        assert!(tracker
            .add_access("resource_z", "system_b", &ResourceAccess::Mutable)
            .is_ok());

        // System C: tries to get mutable access to Y - should fail
        let conflict_result =
            tracker.add_access("resource_y", "system_c", &ResourceAccess::Mutable);
        assert!(
            conflict_result.is_err(),
            "Should not allow mutable access to shared immutable resource"
        );

        // System D: can access X (immutable) since A also has immutable access
        assert!(tracker
            .add_access("resource_x", "system_d", &ResourceAccess::Immutable)
            .is_ok());

        // System E: cannot access Z since B has mutable access
        let z_conflict = tracker.add_access("resource_z", "system_e", &ResourceAccess::Immutable);
        assert!(
            z_conflict.is_err(),
            "Should not allow any access to exclusively held resource"
        );

        // Verify resource summary
        let summary = tracker.get_resource_summary();
        assert_eq!(summary.total_resources, 3); // X, Y, Z
        assert!(summary.immutable_borrows >= 2); // X and Y have immutable borrows
        assert_eq!(summary.mutable_borrows, 1); // Z has one mutable borrow
    }

    /// Test resource release and cleanup
    #[test]
    fn test_resource_release_cleanup() {
        let mut tracker = ResourceTracker::new();

        // Setup: multiple systems with various access patterns
        assert!(tracker
            .add_access("temp_resource", "system1", &ResourceAccess::Immutable)
            .is_ok());
        assert!(tracker
            .add_access("temp_resource", "system2", &ResourceAccess::Immutable)
            .is_ok());
        assert!(tracker
            .add_access("exclusive_resource", "system3", &ResourceAccess::Mutable)
            .is_ok());

        // Verify initial state
        assert_eq!(
            tracker
                .get_immutable_borrowers("temp_resource")
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            tracker.get_mutable_borrower("exclusive_resource"),
            Some(&"system3".to_string())
        );
        assert!(!tracker.is_resource_available("temp_resource"));
        assert!(!tracker.is_resource_available("exclusive_resource"));

        // Release one immutable borrow
        tracker.remove_access("temp_resource", "system1", &ResourceAccess::Immutable);
        assert_eq!(
            tracker
                .get_immutable_borrowers("temp_resource")
                .unwrap()
                .len(),
            1
        );
        assert!(!tracker.is_resource_available("temp_resource")); // Still one borrower

        // Release second immutable borrow
        tracker.remove_access("temp_resource", "system2", &ResourceAccess::Immutable);
        assert!(tracker.get_immutable_borrowers("temp_resource").is_none());
        assert!(tracker.is_resource_available("temp_resource")); // Now available

        // Release mutable borrow
        tracker.remove_access("exclusive_resource", "system3", &ResourceAccess::Mutable);
        assert!(tracker.get_mutable_borrower("exclusive_resource").is_none());
        assert!(tracker.is_resource_available("exclusive_resource"));

        // Verify final cleanup
        let summary = tracker.get_resource_summary();
        assert_eq!(summary.total_resources, 0);
        assert_eq!(summary.immutable_borrows, 0);
        assert_eq!(summary.mutable_borrows, 0);
    }

    /// Test scheduling with priority and resource constraints
    #[test]
    fn test_priority_resource_scheduling() {
        let runtime_config = RuntimeConfig::default();
        let semantic_context = create_test_semantic_context();
        let scheduler = AsyncSystemScheduler::new(runtime_config, semantic_context);

        // High priority system needs exclusive access
        let high_priority_system = create_system_with_resource_access(
            "high_priority",
            vec![("critical_resource", ResourceAccess::Mutable)],
        );

        // Low priority systems need shared access
        let low_priority_system1 = create_system_with_resource_access(
            "low_priority_1",
            vec![("critical_resource", ResourceAccess::Immutable)],
        );
        let low_priority_system2 = create_system_with_resource_access(
            "low_priority_2",
            vec![("critical_resource", ResourceAccess::Immutable)],
        );

        let requests = vec![
            SystemExecutionRequest {
                system_def: low_priority_system1,
                parameters: HashMap::new(),
                priority: TaskPriority::Low,
                timeout: Some(Duration::from_secs(30)),
                table_runtimes: HashMap::new(),
            },
            SystemExecutionRequest {
                system_def: high_priority_system,
                parameters: HashMap::new(),
                priority: TaskPriority::Critical,
                timeout: Some(Duration::from_secs(30)),
                table_runtimes: HashMap::new(),
            },
            SystemExecutionRequest {
                system_def: low_priority_system2,
                parameters: HashMap::new(),
                priority: TaskPriority::Low,
                timeout: Some(Duration::from_secs(30)),
                table_runtimes: HashMap::new(),
            },
        ];

        let schedule_result = tokio_test::block_on(scheduler.schedule_systems(requests));
        assert!(schedule_result.is_ok(), "Priority scheduling should work");

        let futures = schedule_result.unwrap();
        assert_eq!(futures.len(), 3, "All systems should be scheduled");

        // High priority system should be scheduled preferentially
        let stats = scheduler.get_stats();
        assert!(stats.resource_conflicts_resolved >= 1);
    }

    /// Test deadlock detection and prevention
    #[test]
    fn test_deadlock_prevention() {
        let mut tracker = ResourceTracker::new();

        // System A: gets resource 1, wants resource 2
        assert!(tracker
            .add_access("resource_1", "system_a", &ResourceAccess::Mutable)
            .is_ok());

        // System B: gets resource 2, wants resource 1 (potential deadlock)
        assert!(tracker
            .add_access("resource_2", "system_b", &ResourceAccess::Mutable)
            .is_ok());

        // Try to create deadlock situation
        let deadlock_attempt1 =
            tracker.add_access("resource_2", "system_a", &ResourceAccess::Mutable);
        let deadlock_attempt2 =
            tracker.add_access("resource_1", "system_b", &ResourceAccess::Mutable);

        // Both should fail due to conflicts, preventing deadlock
        assert!(
            deadlock_attempt1.is_err(),
            "Should prevent potential deadlock"
        );
        assert!(
            deadlock_attempt2.is_err(),
            "Should prevent potential deadlock"
        );

        // Verify systems still hold their original resources
        assert_eq!(
            tracker.get_mutable_borrower("resource_1"),
            Some(&"system_a".to_string())
        );
        assert_eq!(
            tracker.get_mutable_borrower("resource_2"),
            Some(&"system_b".to_string())
        );
    }

    /// Test borrow safety guarantees under concurrent access
    #[test]
    fn test_borrow_safety_guarantees() {
        let mut tracker = ResourceTracker::new();

        // Test 1: Cannot have mutable and immutable borrows simultaneously
        assert!(tracker
            .add_access("safety_test", "reader", &ResourceAccess::Immutable)
            .is_ok());
        let safety_violation =
            tracker.add_access("safety_test", "writer", &ResourceAccess::Mutable);
        assert!(
            safety_violation.is_err(),
            "Borrow safety violated: mutable after immutable"
        );

        // Reset
        tracker.remove_access("safety_test", "reader", &ResourceAccess::Immutable);

        // Test 2: Cannot have multiple mutable borrows
        assert!(tracker
            .add_access("safety_test", "writer1", &ResourceAccess::Mutable)
            .is_ok());
        let double_mutable = tracker.add_access("safety_test", "writer2", &ResourceAccess::Mutable);
        assert!(
            double_mutable.is_err(),
            "Borrow safety violated: multiple mutable borrows"
        );

        // Reset
        tracker.remove_access("safety_test", "writer1", &ResourceAccess::Mutable);

        // Test 3: Multiple immutable borrows are OK
        assert!(tracker
            .add_access("safety_test", "reader1", &ResourceAccess::Immutable)
            .is_ok());
        assert!(tracker
            .add_access("safety_test", "reader2", &ResourceAccess::Immutable)
            .is_ok());
        assert!(tracker
            .add_access("safety_test", "reader3", &ResourceAccess::Immutable)
            .is_ok());

        let borrowers = tracker.get_immutable_borrowers("safety_test").unwrap();
        assert_eq!(
            borrowers.len(),
            3,
            "Multiple immutable borrows should be allowed"
        );
    }

    // Helper functions for test setup

    fn create_test_semantic_context() -> SemanticContext {
        // Create a minimal semantic context for testing
        SemanticContext {
            variables: HashMap::new(),
            var_scopes: Vec::new(),
            functions: HashMap::new(),
            types: HashMap::new(),
            function_generics: HashMap::new(),
            type_generics: HashMap::new(),
            current_function_return_type: None,
            in_loop: false,
            traits: HashMap::new(),
            trait_impls: HashMap::new(),
            inherent_impls: HashMap::new(),
            tables: HashMap::new(),
            modules: HashMap::new(),
            current_module_path: Vec::new(),
            use_imports: HashMap::new(),
            glob_imports: Vec::new(),
        }
    }

    fn create_system_with_resources(name: &str, resources: Vec<&str>) -> SystemDef {
        let parameters = resources
            .into_iter()
            .map(|res_name| SystemParameter::Resource {
                param_type: "resource".to_string(),
                name: res_name.to_string(),
                resource_type: Type::Identifier {
                    name: "TestResource".to_string(),
                    type_args: vec![],
                },
                access: ResourceAccess::Immutable, // Default to immutable
            })
            .collect();

        SystemDef {
            name: name.to_string(),
            parameters,
            return_type: None,
            body: vec![Statement::Expression(Expression::Literal(
                Literal::Integer(IntegerLiteral {
                    raw: "42".to_string(),
                    value: 42,
                    suffix: None,
                }),
            ))],
            is_async: true,
        }
    }

    fn create_system_with_resource_access(
        name: &str,
        resources: Vec<(&str, ResourceAccess)>,
    ) -> SystemDef {
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
            body: vec![Statement::Expression(Expression::Literal(
                Literal::Integer(IntegerLiteral {
                    raw: "42".to_string(),
                    value: 42,
                    suffix: None,
                }),
            ))],
            is_async: true,
        }
    }

    fn create_system_request(system_def: SystemDef) -> SystemExecutionRequest {
        SystemExecutionRequest {
            system_def,
            parameters: HashMap::new(),
            priority: TaskPriority::Normal,
            timeout: Some(Duration::from_secs(30)),
            table_runtimes: HashMap::new(),
        }
    }
}

// Add tokio test utilities as a separate module since we can't use async/await in regular tests
#[cfg(test)]
mod tokio_test {
    use std::future::Future;

    pub fn block_on<F: Future>(future: F) -> F::Output {
        // Simple blocking executor for tests
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(future)
    }
}
