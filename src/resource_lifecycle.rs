use crate::async_runtime::{AsyncExecutionError, TaskId};
use crate::ast::{ResourceAccess, SystemDef};
use crate::scheduler::SchedulerError;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

/// Resource lifecycle manager for async context with borrow safety
pub struct ResourceLifecycleManager {
    /// Active resource leases
    active_leases: Arc<RwLock<HashMap<String, ResourceLease>>>,
    /// Resource dependencies and hierarchy
    resource_graph: Arc<RwLock<ResourceDependencyGraph>>,
    /// Lease timeout manager
    timeout_manager: Arc<Mutex<LeaseTimeoutManager>>,
    /// Resource pools for efficient allocation
    resource_pools: Arc<RwLock<HashMap<String, ResourcePool>>>,
    /// Lifecycle policies
    policies: ResourceLifecyclePolicies,
}

/// Lease on a resource with lifecycle tracking
#[derive(Debug, Clone)]
pub struct ResourceLease {
    pub resource_name: String,
    pub task_id: TaskId,
    pub access_type: ResourceAccess,
    pub acquired_at: Instant,
    pub lease_duration: Duration,
    pub auto_release: bool,
    pub reference_count: u32,
    pub dependencies: Vec<String>, // Other resources this lease depends on
}

/// Dependency graph for resource management
#[derive(Debug)]
pub struct ResourceDependencyGraph {
    /// Resource dependencies (resource -> dependencies)
    dependencies: HashMap<String, HashSet<String>>,
    /// Reverse dependencies (resource -> dependents)
    dependents: HashMap<String, HashSet<String>>,
    /// Resource hierarchy levels
    levels: HashMap<String, u32>,
}

/// Timeout manager for resource leases
#[derive(Debug)]
pub struct LeaseTimeoutManager {
    timeouts: VecDeque<LeaseTimeout>,
}

/// Scheduled lease timeout
#[derive(Debug, Clone)]
pub struct LeaseTimeout {
    pub resource_name: String,
    pub task_id: TaskId,
    pub timeout_at: Instant,
}

/// Resource pool for efficient allocation
#[derive(Debug)]
pub struct ResourcePool {
    pub pool_name: String,
    pub resource_type: String,
    pub available_resources: VecDeque<String>,
    pub allocated_resources: HashMap<String, TaskId>,
    pub max_size: usize,
    pub allocation_strategy: AllocationStrategy,
}

/// Allocation strategies for resource pools
#[derive(Debug, Clone)]
pub enum AllocationStrategy {
    FirstAvailable,
    LeastRecentlyUsed,
    RoundRobin,
    LoadBalanced,
}

/// Lifecycle policies for resource management
#[derive(Debug, Clone)]
pub struct ResourceLifecyclePolicies {
    pub default_lease_duration: Duration,
    pub max_lease_duration: Duration,
    pub auto_release_on_timeout: bool,
    pub deadlock_detection_enabled: bool,
    pub resource_cleanup_interval: Duration,
    pub max_dependency_depth: u32,
}

/// Result of resource acquisition
#[derive(Debug)]
pub enum AcquisitionResult {
    Acquired(ResourceLease),
    WaitRequired {
        estimated_wait_time: Duration,
        blocking_tasks: Vec<TaskId>,
    },
    Denied {
        reason: String,
        retry_after: Option<Duration>,
    },
}

/// Resource lifecycle events
#[derive(Debug, Clone)]
pub enum LifecycleEvent {
    ResourceAcquired {
        resource_name: String,
        task_id: TaskId,
        access_type: ResourceAccess,
    },
    ResourceReleased {
        resource_name: String,
        task_id: TaskId,
        reason: ReleaseReason,
    },
    LeaseExpired {
        resource_name: String,
        task_id: TaskId,
    },
    DeadlockDetected {
        cycle: Vec<String>,
        involved_tasks: Vec<TaskId>,
    },
    ResourceCreated {
        resource_name: String,
        resource_type: String,
    },
    ResourceDestroyed {
        resource_name: String,
        reason: String,
    },
}

/// Reasons for resource release
#[derive(Debug, Clone)]
pub enum ReleaseReason {
    TaskCompleted,
    TaskCancelled,
    LeaseExpired,
    ExplicitRelease,
    DeadlockResolution,
    SystemShutdown,
}

impl ResourceLifecycleManager {
    /// Create a new resource lifecycle manager
    pub fn new(policies: ResourceLifecyclePolicies) -> Self {
        Self {
            active_leases: Arc::new(RwLock::new(HashMap::new())),
            resource_graph: Arc::new(RwLock::new(ResourceDependencyGraph::new())),
            timeout_manager: Arc::new(Mutex::new(LeaseTimeoutManager::new())),
            resource_pools: Arc::new(RwLock::new(HashMap::new())),
            policies,
        }
    }

    /// Acquire a resource for a task
    pub async fn acquire_resource(
        &self,
        resource_name: String,
        task_id: TaskId,
        access_type: ResourceAccess,
        lease_duration: Option<Duration>,
    ) -> Result<AcquisitionResult, AsyncExecutionError> {
        let lease_duration = lease_duration.unwrap_or(self.policies.default_lease_duration);
        
        // Check if resource can be acquired
        let can_acquire = self.check_acquisition_feasibility(&resource_name, &access_type).await?;
        
        if !can_acquire {
            return Ok(self.calculate_wait_time(&resource_name, &access_type).await?);
        }

        // Check for potential deadlocks
        if self.policies.deadlock_detection_enabled {
            if let Some(cycle) = self.detect_deadlock(&resource_name, task_id).await? {
                return Err(AsyncExecutionError::ResourceConflict {
                    system: format!("task_{:?}", task_id),
                    resource: resource_name,
                    reason: format!("Deadlock detected: {:?}", cycle),
                });
            }
        }

        // Create the lease
        let lease = ResourceLease {
            resource_name: resource_name.clone(),
            task_id,
            access_type: access_type.clone(),
            acquired_at: Instant::now(),
            lease_duration,
            auto_release: true,
            reference_count: 1,
            dependencies: Vec::new(),
        };

        // Record the lease
        {
            let mut leases = self.active_leases.write().unwrap();
            leases.insert(resource_name.clone(), lease.clone());
        }

        // Schedule timeout
        if lease.auto_release {
            let mut timeout_manager = self.timeout_manager.lock().unwrap();
            timeout_manager.schedule_timeout(resource_name.clone(), task_id, lease_duration);
        }

        // Update dependency graph
        self.update_resource_dependencies(&resource_name, task_id).await?;

        Ok(AcquisitionResult::Acquired(lease))
    }

    /// Release a resource
    pub async fn release_resource(
        &self,
        resource_name: String,
        task_id: TaskId,
        reason: ReleaseReason,
    ) -> Result<(), AsyncExecutionError> {
        // Remove the lease
        let lease = {
            let mut leases = self.active_leases.write().unwrap();
            leases.remove(&resource_name)
        };

        let Some(lease) = lease else {
            return Err(AsyncExecutionError::ResourceLifecycleError {
                resource: resource_name,
                phase: "release".to_string(),
                reason: "Resource not found or already released".to_string(),
            });
        };

        // Verify task ownership
        if lease.task_id != task_id {
            return Err(AsyncExecutionError::ResourceLifecycleError {
                resource: resource_name,
                phase: "release".to_string(),
                reason: format!("Task {:?} does not own resource", task_id),
            });
        }

        // Cancel timeout
        {
            let mut timeout_manager = self.timeout_manager.lock().unwrap();
            timeout_manager.cancel_timeout(&resource_name, task_id);
        }

        // Update dependency graph
        self.remove_resource_dependencies(&resource_name, task_id).await?;

        // Emit lifecycle event
        self.emit_lifecycle_event(LifecycleEvent::ResourceReleased {
            resource_name,
            task_id,
            reason,
        });

        Ok(())
    }

    /// Release all resources held by a task
    pub async fn release_all_task_resources(
        &self,
        task_id: TaskId,
        reason: ReleaseReason,
    ) -> Result<Vec<String>, AsyncExecutionError> {
        let resources_to_release: Vec<String> = {
            let leases = self.active_leases.read().unwrap();
            leases.values()
                .filter(|lease| lease.task_id == task_id)
                .map(|lease| lease.resource_name.clone())
                .collect()
        };

        for resource_name in &resources_to_release {
            self.release_resource(resource_name.clone(), task_id, reason.clone()).await?;
        }

        Ok(resources_to_release)
    }

    /// Check if a resource acquisition is feasible
    async fn check_acquisition_feasibility(
        &self,
        resource_name: &str,
        access_type: &ResourceAccess,
    ) -> Result<bool, AsyncExecutionError> {
        let leases = self.active_leases.read().unwrap();
        
        if let Some(existing_lease) = leases.get(resource_name) {
            match (&existing_lease.access_type, access_type) {
                (ResourceAccess::Immutable, ResourceAccess::Immutable) => Ok(true),
                (ResourceAccess::Immutable, ResourceAccess::Mutable) => Ok(false),
                (ResourceAccess::Mutable, _) => Ok(false),
                (ResourceAccess::Owned, _) => Ok(false),
                (_, ResourceAccess::Owned) => Ok(false),
            }
        } else {
            Ok(true)
        }
    }

    /// Calculate wait time for resource acquisition
    async fn calculate_wait_time(
        &self,
        resource_name: &str,
        access_type: &ResourceAccess,
    ) -> Result<AcquisitionResult, AsyncExecutionError> {
        let leases = self.active_leases.read().unwrap();
        
        if let Some(existing_lease) = leases.get(resource_name) {
            let remaining_time = existing_lease.lease_duration
                .saturating_sub(existing_lease.acquired_at.elapsed());
            
            Ok(AcquisitionResult::WaitRequired {
                estimated_wait_time: remaining_time,
                blocking_tasks: vec![existing_lease.task_id],
            })
        } else {
            Ok(AcquisitionResult::Denied {
                reason: "Resource unavailable".to_string(),
                retry_after: Some(Duration::from_secs(1)),
            })
        }
    }

    /// Detect potential deadlocks
    async fn detect_deadlock(
        &self,
        resource_name: &str,
        task_id: TaskId,
    ) -> Result<Option<Vec<String>>, AsyncExecutionError> {
        // Simplified deadlock detection using cycle detection
        let graph = self.resource_graph.read().unwrap();
        
        // Check if adding this resource would create a cycle
        if let Some(dependencies) = graph.dependencies.get(resource_name) {
            for dep in dependencies {
                if let Some(cycle) = self.find_cycle_to_task(dep, task_id, &graph, &mut HashSet::new()) {
                    return Ok(Some(cycle));
                }
            }
        }

        Ok(None)
    }

    /// Find cycle in dependency graph leading to task
    fn find_cycle_to_task(
        &self,
        current_resource: &str,
        target_task: TaskId,
        graph: &ResourceDependencyGraph,
        visited: &mut HashSet<String>,
    ) -> Option<Vec<String>> {
        if visited.contains(current_resource) {
            return Some(vec![current_resource.to_string()]);
        }

        visited.insert(current_resource.to_string());

        if let Some(dependencies) = graph.dependencies.get(current_resource) {
            for dep in dependencies {
                if let Some(mut cycle) = self.find_cycle_to_task(dep, target_task, graph, visited) {
                    cycle.push(current_resource.to_string());
                    return Some(cycle);
                }
            }
        }

        visited.remove(current_resource);
        None
    }

    /// Update resource dependencies
    async fn update_resource_dependencies(
        &self,
        resource_name: &str,
        task_id: TaskId,
    ) -> Result<(), AsyncExecutionError> {
        let mut graph = self.resource_graph.write().unwrap();
        
        // Add resource to graph if not present
        graph.dependencies.entry(resource_name.to_string()).or_insert_with(HashSet::new);
        graph.dependents.entry(resource_name.to_string()).or_insert_with(HashSet::new);

        Ok(())
    }

    /// Remove resource dependencies
    async fn remove_resource_dependencies(
        &self,
        resource_name: &str,
        task_id: TaskId,
    ) -> Result<(), AsyncExecutionError> {
        let mut graph = self.resource_graph.write().unwrap();
        
        // Remove dependencies
        if let Some(dependencies) = graph.dependencies.get_mut(resource_name) {
            dependencies.clear();
        }

        // Remove from dependents
        for dependents in graph.dependents.values_mut() {
            dependents.remove(resource_name);
        }

        Ok(())
    }

    /// Process expired leases
    pub async fn process_expired_leases(&self) -> Result<Vec<LifecycleEvent>, AsyncExecutionError> {
        let mut events = Vec::new();
        let expired_leases: Vec<(String, TaskId)> = {
            let mut timeout_manager = self.timeout_manager.lock().unwrap();
            timeout_manager.get_expired_timeouts()
        };

        for (resource_name, task_id) in expired_leases {
            if self.policies.auto_release_on_timeout {
                self.release_resource(resource_name.clone(), task_id, ReleaseReason::LeaseExpired).await?;
                events.push(LifecycleEvent::LeaseExpired {
                    resource_name,
                    task_id,
                });
            }
        }

        Ok(events)
    }

    /// Emit a lifecycle event
    fn emit_lifecycle_event(&self, event: LifecycleEvent) {
        // This would integrate with the event loop manager
        // For now, just log the event
        println!("Lifecycle event: {:?}", event);
    }

    /// Get resource utilization statistics
    pub fn get_resource_stats(&self) -> ResourceStats {
        let leases = self.active_leases.read().unwrap();
        let graph = self.resource_graph.read().unwrap();

        ResourceStats {
            active_leases: leases.len(),
            total_resources: graph.dependencies.len(),
            average_lease_duration: self.calculate_average_lease_duration(&leases),
            resource_contention_count: self.calculate_contention_count(&leases),
        }
    }

    /// Calculate average lease duration
    fn calculate_average_lease_duration(&self, leases: &HashMap<String, ResourceLease>) -> Duration {
        if leases.is_empty() {
            return Duration::from_secs(0);
        }

        let total_duration: Duration = leases.values()
            .map(|lease| lease.acquired_at.elapsed())
            .sum();

        total_duration / leases.len() as u32
    }

    /// Calculate resource contention count
    fn calculate_contention_count(&self, leases: &HashMap<String, ResourceLease>) -> usize {
        // This would count resources with multiple waiting tasks
        // Simplified for now
        0
    }
}

impl ResourceDependencyGraph {
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
            levels: HashMap::new(),
        }
    }

    /// Add a dependency relationship
    pub fn add_dependency(&mut self, resource: String, dependency: String) {
        self.dependencies.entry(resource.clone())
            .or_insert_with(HashSet::new)
            .insert(dependency.clone());
        
        self.dependents.entry(dependency)
            .or_insert_with(HashSet::new)
            .insert(resource);
    }

    /// Remove a dependency relationship
    pub fn remove_dependency(&mut self, resource: &str, dependency: &str) {
        if let Some(deps) = self.dependencies.get_mut(resource) {
            deps.remove(dependency);
        }
        
        if let Some(dependents) = self.dependents.get_mut(dependency) {
            dependents.remove(resource);
        }
    }

    /// Check if there's a cycle in the dependency graph
    pub fn has_cycle(&self) -> bool {
        let mut visited = HashSet::new();
        let mut recursion_stack = HashSet::new();

        for resource in self.dependencies.keys() {
            if !visited.contains(resource) {
                if self.has_cycle_util(resource, &mut visited, &mut recursion_stack) {
                    return true;
                }
            }
        }

        false
    }

    /// Utility function for cycle detection
    fn has_cycle_util(
        &self,
        resource: &str,
        visited: &mut HashSet<String>,
        recursion_stack: &mut HashSet<String>,
    ) -> bool {
        visited.insert(resource.to_string());
        recursion_stack.insert(resource.to_string());

        if let Some(dependencies) = self.dependencies.get(resource) {
            for dep in dependencies {
                if !visited.contains(dep) {
                    if self.has_cycle_util(dep, visited, recursion_stack) {
                        return true;
                    }
                } else if recursion_stack.contains(dep) {
                    return true;
                }
            }
        }

        recursion_stack.remove(resource);
        false
    }
}

impl LeaseTimeoutManager {
    pub fn new() -> Self {
        Self {
            timeouts: VecDeque::new(),
        }
    }

    /// Schedule a lease timeout
    pub fn schedule_timeout(&mut self, resource_name: String, task_id: TaskId, duration: Duration) {
        let timeout = LeaseTimeout {
            resource_name,
            task_id,
            timeout_at: Instant::now() + duration,
        };

        // Insert in sorted order
        let mut index = 0;
        for existing_timeout in &self.timeouts {
            if timeout.timeout_at < existing_timeout.timeout_at {
                break;
            }
            index += 1;
        }
        
        self.timeouts.insert(index, timeout);
    }

    /// Cancel a timeout
    pub fn cancel_timeout(&mut self, resource_name: &str, task_id: TaskId) {
        self.timeouts.retain(|timeout| {
            !(timeout.resource_name == resource_name && timeout.task_id == task_id)
        });
    }

    /// Get expired timeouts
    pub fn get_expired_timeouts(&mut self) -> Vec<(String, TaskId)> {
        let now = Instant::now();
        let mut expired = Vec::new();

        while let Some(timeout) = self.timeouts.front() {
            if timeout.timeout_at <= now {
                let timeout = self.timeouts.pop_front().unwrap();
                expired.push((timeout.resource_name, timeout.task_id));
            } else {
                break;
            }
        }

        expired
    }
}

/// Resource utilization statistics
#[derive(Debug, Clone)]
pub struct ResourceStats {
    pub active_leases: usize,
    pub total_resources: usize,
    pub average_lease_duration: Duration,
    pub resource_contention_count: usize,
}

impl Default for ResourceLifecyclePolicies {
    fn default() -> Self {
        Self {
            default_lease_duration: Duration::from_secs(30),
            max_lease_duration: Duration::from_secs(300),
            auto_release_on_timeout: true,
            deadlock_detection_enabled: true,
            resource_cleanup_interval: Duration::from_secs(60),
            max_dependency_depth: 10,
        }
    }
}