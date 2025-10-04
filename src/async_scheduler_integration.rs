use crate::ast::{ResourceAccess, SystemDef, SystemParameter};
use crate::async_runtime::{
    AsyncExecutionError, AsyncResult, AsyncSystemRuntime, CompletedTaskInfo, RuntimeConfig,
    SystemFuture, TaskId, TaskPriority,
};
use crate::scheduler::{ConflictType, SystemPriority, SystemScheduler};
use crate::semantic::SemanticContext;
use crate::system_executor::{SystemStateMachineBuilder, SystemStateMachineExecutor};
use crate::table_runtime::{ColumnValue, TableRuntime};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Integrated async scheduler that combines async runtime with borrow safety analysis
pub struct AsyncSystemScheduler {
    /// Core async runtime
    async_runtime: Arc<AsyncSystemRuntime>,
    /// Borrow safety scheduler
    safety_scheduler: Arc<Mutex<SystemScheduler>>,
    /// State machine builder
    state_machine_builder: SystemStateMachineBuilder,
    /// Dependency graph for system execution order
    dependency_graph: Arc<Mutex<DependencyGraph>>,
    /// Conflict resolution cache
    conflict_cache: Arc<Mutex<HashMap<String, ConflictResolution>>>,
    /// Scheduling statistics
    stats: Arc<Mutex<SchedulingStats>>,
}

/// Dependency tracking for system execution
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// System name to tasks mapping
    system_tasks: HashMap<String, Vec<TaskId>>,
    /// Task dependencies (task -> dependencies)
    task_dependencies: HashMap<TaskId, HashSet<TaskId>>,
    /// Resource usage tracking
    resource_usage: HashMap<String, ResourceUsageInfo>,
    /// Execution order constraints
    ordering_constraints: Vec<OrderingConstraint>,
}

/// Information about resource usage
#[derive(Debug, Clone)]
pub struct ResourceUsageInfo {
    pub resource_name: String,
    pub current_readers: HashSet<TaskId>,
    pub current_writer: Option<TaskId>,
    pub pending_readers: VecDeque<TaskId>,
    pub pending_writers: VecDeque<TaskId>,
    pub access_history: Vec<ResourceAccess>,
}

/// Constraint on execution order
#[derive(Debug, Clone)]
pub struct OrderingConstraint {
    pub before_task: TaskId,
    pub after_task: TaskId,
    pub constraint_type: ConstraintType,
}

/// Types of ordering constraints
#[derive(Debug, Clone)]
pub enum ConstraintType {
    DataDependency(String), // Resource name
    CausalDependency,       // One system must complete before another starts
    MutualExclusion,        // Systems cannot run concurrently
}

/// Result of conflict resolution
#[derive(Debug, Clone)]
pub struct ConflictResolution {
    pub can_execute_concurrently: bool,
    pub execution_order: Vec<TaskId>,
    pub resource_scheduling: HashMap<String, ResourceSchedule>,
    pub estimated_completion_time: Duration,
}

/// Schedule for a specific resource
#[derive(Debug, Clone)]
pub struct ResourceSchedule {
    pub resource_name: String,
    pub time_slots: Vec<TimeSlot>,
    pub access_pattern: AccessPattern,
}

/// Time slot for resource access
#[derive(Debug, Clone)]
pub struct TimeSlot {
    pub start_time: Instant,
    pub duration: Duration,
    pub task_id: TaskId,
    pub access_type: ResourceAccess,
}

/// Pattern of resource access
#[derive(Debug, Clone)]
pub enum AccessPattern {
    Sequential,
    Concurrent,
    Batched { batch_size: usize },
}

/// Scheduling statistics
#[derive(Debug, Clone)]
pub struct SchedulingStats {
    pub total_systems_scheduled: usize,
    pub concurrent_executions: usize,
    pub resource_conflicts_resolved: usize,
    pub average_scheduling_time: Duration,
    pub successful_executions: usize,
    pub failed_executions: usize,
    pub preempted_executions: usize,
}

impl AsyncSystemScheduler {
    /// Create a new integrated async scheduler
    pub fn new(runtime_config: RuntimeConfig, semantic_context: SemanticContext) -> Self {
        let async_runtime = Arc::new(AsyncSystemRuntime::new(runtime_config));
        let safety_scheduler = Arc::new(Mutex::new(SystemScheduler::new()));
        let state_machine_builder = SystemStateMachineBuilder::new(semantic_context);

        Self {
            async_runtime,
            safety_scheduler,
            state_machine_builder,
            dependency_graph: Arc::new(Mutex::new(DependencyGraph::new())),
            conflict_cache: Arc::new(Mutex::new(HashMap::new())),
            stats: Arc::new(Mutex::new(SchedulingStats::default())),
        }
    }

    /// Schedule multiple systems for execution with conflict resolution
    pub async fn schedule_systems(
        &self,
        systems: Vec<SystemExecutionRequest>,
    ) -> Result<Vec<SystemFuture>, AsyncExecutionError> {
        let mut futures = Vec::new();

        // Analyze conflicts and dependencies
        let schedule = self.analyze_and_schedule(&systems).await?;

        // Execute systems according to the schedule
        for batch in schedule.execution_batches {
            let mut batch_futures = Vec::new();

            for request in batch.systems {
                let future = self.schedule_single_system(request).await?;
                batch_futures.push(future);
            }

            // Wait for batch completion if required
            if batch.wait_for_completion {
                self.wait_for_batch_completion(&batch_futures).await?;
            }

            futures.extend(batch_futures);
        }

        Ok(futures)
    }

    /// Schedule a single system for execution
    pub async fn schedule_single_system(
        &self,
        request: SystemExecutionRequest,
    ) -> Result<SystemFuture, AsyncExecutionError> {
        // Update statistics
        {
            let mut stats = self.stats.lock().unwrap();
            stats.total_systems_scheduled += 1;
        }

        // Check for conflicts with existing systems
        self.check_resource_conflicts(&request).await?;

        // Build state machine for the system
        let state_machine = self
            .state_machine_builder
            .build_state_machine(&request.system_def, request.parameters.clone())?;

        // Create executor for the state machine
        let mut executor = SystemStateMachineExecutor::new();

        // Register tables with executor
        for (table_name, table_runtime) in &request.table_runtimes {
            executor.register_table(table_name.clone(), table_runtime.clone());
        }

        // Submit to async runtime
        let future = self.async_runtime.submit_system(
            request.system_def.clone(),
            request.parameters.clone(),
            request.priority,
            request.timeout,
        )?;

        let task_id = future.task_id();

        // Attach execution plan so the runtime can drive the state machine
        self.async_runtime
            .attach_execution_plan(task_id, state_machine, executor)?;

        // Update dependency graph
        self.update_dependency_graph(task_id, &request).await?;

        Ok(future)
    }

    /// Analyze conflicts and create execution schedule
    async fn analyze_and_schedule(
        &self,
        systems: &[SystemExecutionRequest],
    ) -> Result<ExecutionSchedule, AsyncExecutionError> {
        let start_time = Instant::now();

        // Build conflict matrix
        let conflict_matrix = self.build_conflict_matrix(systems).await?;

        // Resolve conflicts and create batches
        let execution_batches = self
            .resolve_conflicts_and_batch(systems, &conflict_matrix)
            .await?;

        // Update statistics
        {
            let mut stats = self.stats.lock().unwrap();
            stats.average_scheduling_time = start_time.elapsed();
        }

        {
            let mut cache = self.conflict_cache.lock().unwrap();
            cache.clear();

            for batch in &execution_batches {
                for request in &batch.systems {
                    cache.insert(
                        request.system_def.name.clone(),
                        ConflictResolution {
                            can_execute_concurrently: batch.systems.len() > 1,
                            execution_order: Vec::new(),
                            resource_scheduling: HashMap::new(),
                            estimated_completion_time: batch.estimated_duration,
                        },
                    );
                }
            }
        }

        let total_time = self.estimate_total_execution_time(&execution_batches);
        Ok(ExecutionSchedule {
            execution_batches,
            total_estimated_time: total_time,
        })
    }

    /// Build conflict matrix between systems
    async fn build_conflict_matrix(
        &self,
        systems: &[SystemExecutionRequest],
    ) -> Result<ConflictMatrix, AsyncExecutionError> {
        let n = systems.len();
        let mut matrix = ConflictMatrix::new(n);

        for i in 0..n {
            for j in (i + 1)..n {
                let conflict_type = self
                    .analyze_system_conflict(&systems[i], &systems[j])
                    .await?;
                matrix.set_conflict(i, j, conflict_type);
            }
        }

        Ok(matrix)
    }

    /// Analyze conflict between two systems
    async fn analyze_system_conflict(
        &self,
        system1: &SystemExecutionRequest,
        system2: &SystemExecutionRequest,
    ) -> Result<ConflictType, AsyncExecutionError> {
        // For now, simplified conflict analysis
        // In a real implementation, this would use proper conflict detection
        let system1_resources = self.extract_system_resources(system1);
        let system2_resources = self.extract_system_resources(system2);

        for (resource, access1) in &system1_resources {
            if let Some(access2) = system2_resources.get(resource) {
                if let Some(conflict) = Self::determine_conflict_type(access1, access2) {
                    self.record_resource_conflict();
                    return Ok(conflict);
                }
            }
        }

        Ok(ConflictType::None)
    }

    /// Resolve conflicts and create execution batches
    async fn resolve_conflicts_and_batch(
        &self,
        systems: &[SystemExecutionRequest],
        conflict_matrix: &ConflictMatrix,
    ) -> Result<Vec<ExecutionBatch>, AsyncExecutionError> {
        let mut batches = Vec::new();
        let mut unscheduled: HashSet<usize> = (0..systems.len()).collect();

        while !unscheduled.is_empty() {
            let mut current_batch = Vec::new();
            let mut batch_resource_accesses: HashMap<String, Vec<ResourceAccess>> = HashMap::new();

            // Find systems that can execute concurrently
            let mut to_remove = Vec::new();

            for &i in &unscheduled {
                let system = &systems[i];
                let system_resources: Vec<(String, ResourceAccess)> =
                    self.extract_system_resources(system).into_iter().collect();

                // Check if this system conflicts with current batch
                let mut has_conflict = false;

                for &j in &current_batch {
                    if conflict_matrix.has_conflict(i, j) != ConflictType::None {
                        has_conflict = true;
                        break;
                    }
                }

                if !has_conflict {
                    for (resource, access) in &system_resources {
                        if let Some(existing) = batch_resource_accesses.get(resource) {
                            if existing.iter().any(|existing_access| {
                                Self::resource_accesses_conflict(existing_access, access)
                            }) {
                                has_conflict = true;
                                break;
                            }
                        }
                    }
                }

                if !has_conflict {
                    current_batch.push(i);
                    for (resource, access) in system_resources {
                        batch_resource_accesses
                            .entry(resource)
                            .or_insert_with(Vec::new)
                            .push(access);
                    }
                    to_remove.push(i);
                }
            }

            // Remove scheduled systems from unscheduled set
            for i in to_remove {
                unscheduled.remove(&i);
            }

            // Create execution batch
            let batch_systems: Vec<SystemExecutionRequest> = current_batch
                .into_iter()
                .map(|i| systems[i].clone())
                .collect();

            batches.push(ExecutionBatch {
                systems: batch_systems,
                wait_for_completion: true, // Wait for batch completion by default
                estimated_duration: Duration::from_secs(5), // Placeholder
            });
        }

        Ok(batches)
    }

    /// Extract resources used by a system
    fn extract_system_resources(
        &self,
        system: &SystemExecutionRequest,
    ) -> HashMap<String, ResourceAccess> {
        let mut resources = HashMap::new();

        for param in &system.system_def.parameters {
            if let SystemParameter::Resource { name, access, .. } = param {
                resources.insert(name.clone(), access.clone());
            }
        }

        resources
    }

    fn record_resource_conflict(&self) {
        let mut stats = self.stats.lock().unwrap();
        stats.resource_conflicts_resolved += 1;
    }

    fn resource_accesses_conflict(a: &ResourceAccess, b: &ResourceAccess) -> bool {
        Self::determine_conflict_type(a, b).is_some()
    }

    fn determine_conflict_type(
        access1: &ResourceAccess,
        access2: &ResourceAccess,
    ) -> Option<ConflictType> {
        match (access1, access2) {
            (ResourceAccess::Immutable, ResourceAccess::Immutable) => None,
            (ResourceAccess::Immutable, ResourceAccess::Mutable)
            | (ResourceAccess::Immutable, ResourceAccess::Owned) => Some(ConflictType::ReadWrite),
            (ResourceAccess::Mutable, ResourceAccess::Immutable)
            | (ResourceAccess::Owned, ResourceAccess::Immutable) => Some(ConflictType::WriteRead),
            (ResourceAccess::Mutable, ResourceAccess::Mutable)
            | (ResourceAccess::Mutable, ResourceAccess::Owned)
            | (ResourceAccess::Owned, ResourceAccess::Mutable)
            | (ResourceAccess::Owned, ResourceAccess::Owned) => Some(ConflictType::WriteWrite),
        }
    }

    /// Check for resource conflicts with existing systems
    async fn check_resource_conflicts(
        &self,
        request: &SystemExecutionRequest,
    ) -> Result<(), AsyncExecutionError> {
        let mut scheduler = self.safety_scheduler.lock().unwrap();
        scheduler
            .add_system(request.system_def.clone())
            .map_err(AsyncExecutionError::from)?;
        scheduler.set_system_priority(&request.system_def.name, convert_priority(request.priority));

        Ok(())
    }

    /// Update dependency graph with new task
    async fn update_dependency_graph(
        &self,
        task_id: TaskId,
        request: &SystemExecutionRequest,
    ) -> Result<(), AsyncExecutionError> {
        let mut graph = self.dependency_graph.lock().unwrap();

        // Add task to system mapping
        graph
            .system_tasks
            .entry(request.system_def.name.clone())
            .or_insert_with(Vec::new)
            .push(task_id);

        if let Some(tasks) = graph.system_tasks.get(&request.system_def.name) {
            if tasks.len() >= 2 {
                if let Some(&previous_task) = tasks.iter().rev().nth(1) {
                    graph.ordering_constraints.push(OrderingConstraint {
                        before_task: previous_task,
                        after_task: task_id,
                        constraint_type: ConstraintType::CausalDependency,
                    });
                }
            }
        }

        // Initialize dependencies
        graph.task_dependencies.insert(task_id, HashSet::new());

        // Update resource usage
        for param in &request.system_def.parameters {
            if let SystemParameter::Resource { name, access, .. } = param {
                let usage_info =
                    graph
                        .resource_usage
                        .entry(name.clone())
                        .or_insert_with(|| ResourceUsageInfo {
                            resource_name: name.clone(),
                            current_readers: HashSet::new(),
                            current_writer: None,
                            pending_readers: VecDeque::new(),
                            pending_writers: VecDeque::new(),
                            access_history: Vec::new(),
                        });

                match access {
                    ResourceAccess::Immutable => {
                        if usage_info.current_writer.is_none() {
                            usage_info.current_readers.insert(task_id);
                        } else {
                            usage_info.pending_readers.push_back(task_id);
                        }
                    }
                    ResourceAccess::Mutable | ResourceAccess::Owned => {
                        if usage_info.current_writer.is_none()
                            && usage_info.current_readers.is_empty()
                        {
                            usage_info.current_writer = Some(task_id);
                        } else {
                            usage_info.pending_writers.push_back(task_id);
                        }
                    }
                }

                usage_info.access_history.push(access.clone());
            }
        }

        Ok(())
    }

    /// Wait for batch completion
    async fn wait_for_batch_completion(
        &self,
        futures: &[SystemFuture],
    ) -> Result<(), AsyncExecutionError> {
        if futures.is_empty() {
            return Ok(());
        }

        {
            let mut stats = self.stats.lock().unwrap();
            stats.concurrent_executions = stats.concurrent_executions.max(futures.len());
        }

        // Futures are driven by the async runtime; here we just observe their state to avoid blocking.
        let _completed = futures
            .iter()
            .filter(|future| future.is_completed())
            .count();

        Ok(())
    }

    /// Estimate total execution time
    fn estimate_total_execution_time(&self, batches: &[ExecutionBatch]) -> Duration {
        batches.iter().map(|batch| batch.estimated_duration).sum()
    }

    /// Get current scheduling statistics
    pub fn get_stats(&self) -> SchedulingStats {
        self.stats.lock().unwrap().clone()
    }

    /// Register a table runtime for use by systems
    pub fn register_table(&self, name: String, table_runtime: TableRuntime) {
        self.async_runtime.register_table(name, table_runtime);
    }

    /// Run the scheduler's main loop
    pub async fn run_scheduler_loop(&self) -> Result<(), AsyncExecutionError> {
        loop {
            // Execute pending tasks
            let has_work = self.tick_runtime()?;

            if !has_work {
                // No work to do, sleep briefly
                std::thread::sleep(Duration::from_millis(10));
            }

            // Check for completed tasks and update dependencies
            self.update_completed_tasks().await?;
        }
    }

    /// Update completed tasks and resolve dependencies
    async fn update_completed_tasks(&self) -> Result<(), AsyncExecutionError> {
        let completions = self.async_runtime.drain_completed_tasks();

        if completions.is_empty() {
            return Ok(());
        }

        for completion in &completions {
            self.record_task_completion(completion);
        }

        Ok(())
    }

    /// Execute one scheduling tick, returning whether any work was performed.
    pub fn tick_runtime(&self) -> AsyncResult<bool> {
        self.async_runtime.tick()
    }

    /// Drain completed tasks from the async runtime for downstream processing.
    pub fn drain_completed_tasks(&self) -> Vec<CompletedTaskInfo> {
        self.async_runtime.drain_completed_tasks()
    }

    /// Record completion statistics and clear scheduler bookkeeping for the given task.
    pub fn record_task_completion(&self, completion: &CompletedTaskInfo) {
        {
            let mut stats = self.stats.lock().unwrap();
            if completion.result.is_ok() {
                stats.successful_executions += 1;
            } else {
                stats.failed_executions += 1;
            }
        }

        // Remove task from dependency graph bookkeeping if present.
        let mut graph = self.dependency_graph.lock().unwrap();
        graph.task_dependencies.remove(&completion.task_id);
        if let Some(system_def) = completion.system_def.as_ref().map(|def| def.name.clone()) {
            if let Some(tasks) = graph.system_tasks.get_mut(&system_def) {
                tasks.retain(|id| id != &completion.task_id);
                if tasks.is_empty() {
                    graph.system_tasks.remove(&system_def);
                }
            }
        }
    }
}

/// Request to execute a system
#[derive(Debug, Clone)]
pub struct SystemExecutionRequest {
    pub system_def: SystemDef,
    pub parameters: HashMap<String, ColumnValue>,
    pub priority: TaskPriority,
    pub timeout: Option<Duration>,
    pub table_runtimes: HashMap<String, TableRuntime>,
}

/// Complete execution schedule
#[derive(Debug)]
pub struct ExecutionSchedule {
    pub execution_batches: Vec<ExecutionBatch>,
    pub total_estimated_time: Duration,
}

/// Batch of systems that can execute concurrently
#[derive(Debug)]
pub struct ExecutionBatch {
    pub systems: Vec<SystemExecutionRequest>,
    pub wait_for_completion: bool,
    pub estimated_duration: Duration,
}

/// Matrix tracking conflicts between systems
#[derive(Debug)]
pub struct ConflictMatrix {
    size: usize,
    conflicts: Vec<Vec<ConflictType>>,
}

impl ConflictMatrix {
    pub fn new(size: usize) -> Self {
        Self {
            size,
            conflicts: vec![vec![ConflictType::None; size]; size],
        }
    }

    pub fn set_conflict(&mut self, i: usize, j: usize, conflict: ConflictType) {
        if i < self.size && j < self.size {
            self.conflicts[i][j] = conflict.clone();
            self.conflicts[j][i] = conflict; // Symmetric
        }
    }

    pub fn has_conflict(&self, i: usize, j: usize) -> ConflictType {
        if i < self.size && j < self.size {
            self.conflicts[i][j].clone()
        } else {
            ConflictType::None
        }
    }
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            system_tasks: HashMap::new(),
            task_dependencies: HashMap::new(),
            resource_usage: HashMap::new(),
            ordering_constraints: Vec::new(),
        }
    }
}

impl Default for SchedulingStats {
    fn default() -> Self {
        Self {
            total_systems_scheduled: 0,
            concurrent_executions: 0,
            resource_conflicts_resolved: 0,
            average_scheduling_time: Duration::from_secs(0),
            successful_executions: 0,
            failed_executions: 0,
            preempted_executions: 0,
        }
    }
}

fn convert_priority(priority: TaskPriority) -> SystemPriority {
    match priority {
        TaskPriority::Low => SystemPriority::Low,
        TaskPriority::Normal => SystemPriority::Normal,
        TaskPriority::High => SystemPriority::High,
        TaskPriority::Critical => SystemPriority::Critical,
    }
}
