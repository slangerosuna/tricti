//! Legacy TriCTI Standard Library Types
//!
//! This module contains the original struct-based error and result types.
//! These are kept for backward compatibility during the migration process.

/// Legacy struct-based error type
#[derive(Debug, Clone, PartialEq)]
pub struct StdError {
    pub kind: String,
    pub message: String,
    pub parameter: Option<String>,
    pub feature: Option<String>,
    pub source: Option<String>,
}

/// Legacy struct-based result type
#[derive(Debug, Clone, PartialEq)]
pub struct StdResult<T> {
    pub is_ok: bool,
    pub value: Option<T>,
    pub error: Option<StdError>,
}

/// Get the error message from a legacy StdError
pub fn std_error_message(error: &StdError) -> String {
    error.message.clone()
}

/// Get the error kind from a legacy StdError
pub fn std_error_kind(error: &StdError) -> String {
    error.kind.clone()
}

/// Create a legacy StdError with source information
pub fn std_error_with_source(kind: &str, message: &str, source: &str) -> StdError {
    StdError {
        kind: kind.to_string(),
        message: message.to_string(),
        parameter: None,
        feature: None,
        source: Some(source.to_string()),
    }
}

/// Create a legacy StdResult with an Ok value
pub fn std_ok<T>(value: T) -> StdResult<T> {
    StdResult {
        is_ok: true,
        value: Some(value),
        error: None,
    }
}

/// Create a legacy StdResult with an Err value
pub fn std_err<T>(error: StdError) -> StdResult<T> {
    StdResult {
        is_ok: false,
        value: None,
        error: Some(error),
    }
}

/// Runtime scaffolding struct
/// Currently empty, reserved for future runtime configuration
#[derive(Debug, Clone, PartialEq)]
pub struct Runtime {}

/// Runtime entry point function
/// Calls main() and returns exit code 0
pub fn start() -> i32 {
    // Call main function (placeholder - would be provided by user code)
    main();
    0
}

/// Placeholder main function for testing
/// In actual TriCTI programs, this would be user-defined
pub fn main() {
    // User main function would go here
}

/// Log level enumeration for structured logging
#[derive(Debug, Clone, PartialEq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Log a message at the specified level
/// Currently forwards to println for visibility until host exposes structured logging
pub fn log_message(level: &LogLevel, message: &str) {
    let label = match level {
        LogLevel::Trace => "[trace]",
        LogLevel::Debug => "[debug]",
        LogLevel::Info => "[info]",
        LogLevel::Warn => "[warn]",
        LogLevel::Error => "[error]",
    };
    println!("{}", label);
    println!("{}", message);
}

/// Log a message at Info level
pub fn log_info(message: &str) {
    log_message(&LogLevel::Info, message);
}

/// Log a message at Error level
pub fn log_error(message: &str) {
    log_message(&LogLevel::Error, message);
}

/// Runtime configuration options
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeConfigOptions {
    pub enable_preemption: bool,
    pub scheduling_quantum_ms: i64,
}

/// IO configuration options
#[derive(Debug, Clone, PartialEq)]
pub struct IOConfigOptions {
    pub default_timeout_ms: i64,
    pub enable_tracing: bool,
}

/// Standard configuration structure shared by stdlib modules
#[derive(Debug, Clone, PartialEq)]
pub struct StdConfig {
    pub runtime: RuntimeConfigOptions,
    pub io: IOConfigOptions,
}

/// Create default configuration for stdlib modules
pub fn std_default_config() -> StdConfig {
    StdConfig {
        runtime: RuntimeConfigOptions {
            enable_preemption: true,
            scheduling_quantum_ms: 10,
        },
        io: IOConfigOptions {
            default_timeout_ms: 5000,
            enable_tracing: false,
        },
    }
}

/// Task priority levels for async execution
#[derive(Debug, Clone, PartialEq)]
pub enum TaskPriority {
    Low,
    Normal,
    High,
    Critical,
}

/// Resource access types for concurrent resource management
#[derive(Debug, Clone, PartialEq)]
pub enum ResourceAccess {
    Immutable,
    Mutable,
    Owned,
}

/// Errors that can occur during async execution
#[derive(Debug, Clone, PartialEq)]
pub enum AsyncExecutionError {
    ResourceConflict {
        system: String,
        resource: String,
        reason: String,
    },
    SchedulingError {
        message: String,
    },
    TableError {
        message: String,
    },
    SystemError {
        system: String,
        message: String,
    },
    Timeout {
        system: String,
        duration_ms: i64,
    },
    Cancelled {
        system: String,
    },
    ResourceLifecycleError {
        resource: String,
        phase: String,
        reason: String,
    },
}

/// Points where async task execution can yield control
#[derive(Debug, Clone, PartialEq)]
pub enum YieldPoint {
    AwaitingResource {
        resource_name: String,
        access_type: ResourceAccess,
    },
    AwaitingTableQuery {
        table_name: String,
        query_type: String,
    },
    AwaitingSystemCompletion {
        system_name: String,
        task_id: u64,
    },
    AwaitingSignal {
        signal_name: String,
    },
    Sleeping {
        duration_ms: i64,
        started_at_ms: i64,
    },
}

/// States that an async task can be in during its lifecycle
#[derive(Debug, Clone, PartialEq)]
pub enum TaskState {
    Pending,
    Running {
        started_at_ms: i64,
    },
    Suspended {
        yield_point: YieldPoint,
        suspended_at_ms: i64,
        intermediate_state: Option<Vec<ParameterValue>>,
    },
    Completed {
        completed_at_ms: i64,
        result: SystemExecutionResult,
    },
    Failed {
        error: AsyncExecutionError,
    },
    Cancelled {
        cancelled_at_ms: i64,
    },
}

/// Results from system execution
#[derive(Debug, Clone, PartialEq)]
pub enum SystemExecutionResult {
    Success {
        return_value: Option<String>,
        resources_modified: Vec<String>,
        tables_modified: Vec<String>,
    },
    Partial {
        intermediate_state: Vec<ParameterValue>,
        next_yield_point: YieldPoint,
    },
}

/// Task outcome after execution attempt
#[derive(Debug, Clone, PartialEq)]
pub enum TaskOutcome {
    Completed { result: SystemExecutionResult },
    Failed { error: AsyncExecutionError },
}

/// Runtime statistics for monitoring
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeStats {
    pub total_tasks_created: i64,
    pub active_tasks: i64,
    pub queued_tasks: i64,
    pub completed_tasks: i64,
    pub failed_tasks: i64,
    pub cancelled_tasks: i64,
    pub resource_contentions: i64,
}

/// Handle to an acquired resource
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceHandle {
    pub resource_name: String,
    pub access_type: ResourceAccess,
    pub acquired_at_ms: i64,
    pub lease_duration_ms: Option<i64>,
}

/// Request for resource acquisition
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceRequest {
    pub resource_name: String,
    pub access_type: ResourceAccess,
    pub lease_duration_ms: Option<i64>,
}

/// Task waker for async notifications
#[derive(Debug, Clone, PartialEq)]
pub struct TaskWaker {
    pub task_id: u64,
    pub token: String,
}

/// Parameter value for task execution
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterValue {
    pub name: String,
    pub payload: String,
}

/// Bag of parameters for task execution
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterBag {
    pub values: Vec<ParameterValue>,
}

/// Runtime configuration for async execution
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeConfig {
    pub max_concurrent_systems: i64,
    pub default_task_timeout_ms: i64,
    pub resource_lease_timeout_ms: i64,
    pub scheduling_quantum_ms: i64,
    pub enable_preemption: bool,
}

/// Async task representation
#[derive(Debug, Clone, PartialEq)]
pub struct AsyncTask {
    pub id: u64,
    pub system_name: String,
    pub parameters: ParameterBag,
    pub state: TaskState,
    pub priority: TaskPriority,
    pub created_at_ms: i64,
    pub timeout_ms: Option<i64>,
    pub resource_handles: Vec<ResourceHandle>,
    pub dependencies: Vec<u64>,
}

/// Main async runtime state
#[derive(Debug, Clone, PartialEq)]
pub struct AsyncRuntimeState {
    pub config: RuntimeConfig,
    pub next_task_id: u64,
    pub active_tasks: Vec<AsyncTask>,
    pub completed_tasks: Vec<CompletedTaskRecord>,
    pub queued_task_ids: Vec<u64>,
    pub wakers: Vec<TaskWaker>,
    pub resource_leases: Vec<ActiveResourceLease>,
    pub resource_waiters: Vec<ResourceWaiter>,
    pub resume_buffers: Vec<TaskIntermediateState>,
    pub resource_summary: RuntimeStats,
}

/// Record of completed task
#[derive(Debug, Clone, PartialEq)]
pub struct CompletedTaskRecord {
    pub id: u64,
    pub state: TaskState,
}

/// Active resource lease
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveResourceLease {
    pub resource_name: String,
    pub task_id: u64,
    pub access_type: ResourceAccess,
    pub acquired_at_ms: i64,
    pub lease_duration_ms: Option<i64>,
}

/// Waiting task for resource
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceWaiter {
    pub resource_name: String,
    pub task_id: u64,
    pub access_type: ResourceAccess,
    pub requested_at_ms: i64,
    pub lease_duration_ms: Option<i64>,
}

/// Result of task resume operation
#[derive(Debug, Clone, PartialEq)]
pub struct TaskResumeResult {
    pub resumed: bool,
    pub intermediate_state: Option<Vec<ParameterValue>>,
}

/// Intermediate state for task resumption
#[derive(Debug, Clone, PartialEq)]
pub struct TaskIntermediateState {
    pub task_id: u64,
    pub values: Vec<ParameterValue>,
}

/// Context for task dispatch
#[derive(Debug, Clone, PartialEq)]
pub struct TaskDispatchContext {
    pub task: AsyncTask,
    pub intermediate_state: Option<Vec<ParameterValue>>,
}

/// Create default runtime configuration
pub fn default_runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        max_concurrent_systems: 100,
        default_task_timeout_ms: 30_000,
        resource_lease_timeout_ms: 5_000,
        scheduling_quantum_ms: 10,
        enable_preemption: true,
    }
}

/// Create new async runtime with optional configuration
pub fn new_async_runtime(config: Option<RuntimeConfig>) -> AsyncRuntimeState {
    let cfg = config.unwrap_or_else(default_runtime_config);

    AsyncRuntimeState {
        config: cfg,
        next_task_id: 1,
        active_tasks: Vec::new(),
        completed_tasks: Vec::new(),
        queued_task_ids: Vec::new(),
        wakers: Vec::new(),
        resource_leases: Vec::new(),
        resource_waiters: Vec::new(),
        resume_buffers: Vec::new(),
        resource_summary: empty_runtime_stats(),
    }
}

/// Submit a new task to the runtime
pub fn submit_task(
    runtime: &mut AsyncRuntimeState,
    system_name: String,
    parameters: ParameterBag,
    priority: TaskPriority,
    timeout_ms: Option<i64>,
) -> AsyncTask {
    let id = runtime.next_task_id;
    runtime.next_task_id = id + 1;

    let task = AsyncTask {
        id,
        system_name,
        parameters,
        state: TaskState::Pending,
        priority,
        created_at_ms: now_ms(),
        timeout_ms,
        resource_handles: Vec::new(),
        dependencies: Vec::new(),
    };

    runtime.active_tasks.push(task.clone());
    runtime.queued_task_ids.push(id);

    runtime.resource_summary.total_tasks_created += 1;
    runtime.resource_summary.active_tasks += 1;
    runtime.resource_summary.queued_tasks += 1;

    task
}

/// Create empty parameter bag
pub fn empty_parameter_bag() -> ParameterBag {
    ParameterBag { values: Vec::new() }
}

/// Create empty runtime statistics
pub fn empty_runtime_stats() -> RuntimeStats {
    RuntimeStats {
        total_tasks_created: 0,
        active_tasks: 0,
        queued_tasks: 0,
        completed_tasks: 0,
        failed_tasks: 0,
        cancelled_tasks: 0,
        resource_contentions: 0,
    }
}

/// Get current timestamp in milliseconds (mock implementation)
pub fn now_ms() -> i64 {
    1000
}

/// Check if resource access types conflict
pub fn resource_access_conflict(existing: &ResourceAccess, requested: &ResourceAccess) -> bool {
    match existing {
        ResourceAccess::Immutable => !matches!(requested, ResourceAccess::Immutable),
        ResourceAccess::Mutable => true,
        ResourceAccess::Owned => true,
    }
}

/// Count running tasks in a task list
pub fn count_running_tasks(tasks: &[AsyncTask]) -> i64 {
    tasks.iter().filter(|task| matches!(task.state, TaskState::Running { .. })).count() as i64
}

/// Get minimum of optional i64 values
pub fn min_option_i64(current: Option<i64>, candidate: i64) -> Option<i64> {
    match current {
        Some(existing) => Some(if candidate < existing { candidate } else { existing }),
        None => Some(candidate),
    }
}

/// Mark a task as running
pub fn mark_task_running(runtime: &mut AsyncRuntimeState, task_id: u64) {
    let task_idx = runtime.active_tasks.iter().position(|task| task.id == task_id)
        .expect("mark_task_running: unknown task id");

    let task = &mut runtime.active_tasks[task_idx];
    task.state = TaskState::Running { started_at_ms: now_ms() };
}

/// Complete a task with the given state
pub fn complete_task(runtime: &mut AsyncRuntimeState, task_id: u64, state: TaskState) {
    // Remove from queued tasks if present
    if let Some(pos) = runtime.queued_task_ids.iter().position(|&id| id == task_id) {
        runtime.queued_task_ids.remove(pos);
        runtime.resource_summary.queued_tasks = (runtime.resource_summary.queued_tasks - 1).max(0);
    }

    // Remove waker if present
    if let Some(pos) = runtime.wakers.iter().position(|waker| waker.task_id == task_id) {
        runtime.wakers.remove(pos);
    }

    let task_idx = runtime.active_tasks.iter().position(|task| task.id == task_id)
        .expect("complete_task: unknown task id");

    let task = runtime.active_tasks.remove(task_idx);
    let task_state = state.clone();

    // Clear intermediate state
    clear_task_intermediate_state(runtime, task_id);

    // Release task resources
    release_task_resources(runtime, &task);

    // Add to completed tasks
    runtime.completed_tasks.push(CompletedTaskRecord {
        id: task_id,
        state: task_state,
    });

    // Update statistics
    runtime.resource_summary.active_tasks = (runtime.resource_summary.active_tasks - 1).max(0);
    runtime.resource_summary.completed_tasks += 1;

    match state {
        TaskState::Failed { .. } => {
            runtime.resource_summary.failed_tasks += 1;
        }
        TaskState::Cancelled { .. } => {
            runtime.resource_summary.cancelled_tasks += 1;
        }
        _ => {}
    }
}

/// Suspend a task at a yield point
pub fn suspend_task(runtime: &mut AsyncRuntimeState, task_id: u64, yield_point: YieldPoint, intermediate_state: Option<Vec<ParameterValue>>) {
    let task_idx = runtime.active_tasks.iter().position(|task| task.id == task_id)
        .expect("suspend_task: unknown task id");

    let task = &mut runtime.active_tasks[task_idx];
    task.state = TaskState::Suspended {
        yield_point,
        suspended_at_ms: now_ms(),
        intermediate_state,
    };
}

/// Resume a suspended task
pub fn resume_task(runtime: &mut AsyncRuntimeState, task_id: u64) -> TaskResumeResult {
    let task_idx = runtime.active_tasks.iter().position(|task| task.id == task_id);

    let task_idx = match task_idx {
        Some(idx) => idx,
        None => return TaskResumeResult {
            resumed: false,
            intermediate_state: None,
        },
    };

    let task = &runtime.active_tasks[task_idx];
    match &task.state {
        TaskState::Suspended { intermediate_state, .. } => {
            let intermediate_state = intermediate_state.clone();
            let task = &mut runtime.active_tasks[task_idx];
            task.state = TaskState::Pending;

            // Add to queue if not already there
            if !runtime.queued_task_ids.contains(&task_id) {
                runtime.queued_task_ids.push(task_id);
                runtime.resource_summary.queued_tasks += 1;
            }

            // Store intermediate state if present
            if let Some(ref values) = intermediate_state {
                store_task_intermediate_state(runtime, task_id, values.clone());
            }

            TaskResumeResult {
                resumed: true,
                intermediate_state,
            }
        }
        TaskState::Pending => {
            // Add to queue if not already there
            if !runtime.queued_task_ids.contains(&task_id) {
                runtime.queued_task_ids.push(task_id);
                runtime.resource_summary.queued_tasks += 1;
            }

            TaskResumeResult {
                resumed: true,
                intermediate_state: None,
            }
        }
        _ => TaskResumeResult {
            resumed: false,
            intermediate_state: None,
        },
    }
}

/// Cancel a task
pub fn cancel_task(runtime: &mut AsyncRuntimeState, task_id: u64) {
    // Remove from queued tasks if present
    if let Some(pos) = runtime.queued_task_ids.iter().position(|&id| id == task_id) {
        runtime.queued_task_ids.remove(pos);
        runtime.resource_summary.queued_tasks = (runtime.resource_summary.queued_tasks - 1).max(0);
    }

    let state = TaskState::Cancelled { cancelled_at_ms: now_ms() };
    complete_task(runtime, task_id, state);
}

/// Fail a task with an error
pub fn fail_task(runtime: &mut AsyncRuntimeState, task_id: u64, error: AsyncExecutionError) {
    // Remove from queued tasks if present
    if let Some(pos) = runtime.queued_task_ids.iter().position(|&id| id == task_id) {
        runtime.queued_task_ids.remove(pos);
        runtime.resource_summary.queued_tasks = (runtime.resource_summary.queued_tasks - 1).max(0);
    }

    let state = TaskState::Failed { error };
    complete_task(runtime, task_id, state);
}

/// Complete a task successfully
pub fn complete_task_success(runtime: &mut AsyncRuntimeState, task_id: u64, result: SystemExecutionResult) {
    let state = TaskState::Completed {
        completed_at_ms: now_ms(),
        result,
    };
    complete_task(runtime, task_id, state);
}

/// Yield a task with partial execution result
pub fn yield_task(runtime: &mut AsyncRuntimeState, task_id: u64, partial: SystemExecutionResult) {
    match partial {
        SystemExecutionResult::Partial { intermediate_state, next_yield_point } => {
            suspend_task(runtime, task_id, next_yield_point, Some(intermediate_state));
        }
        _ => panic!("yield_task requires a partial execution result"),
    }
}

/// Suspend task for a specific yield point
pub fn suspend_task_for_yield_point(runtime: &mut AsyncRuntimeState, task_id: u64, yield_point: YieldPoint) {
    suspend_task(runtime, task_id, yield_point, None);
}

/// Apply task outcome to runtime state
pub fn apply_task_outcome(runtime: &mut AsyncRuntimeState, task_id: u64, outcome: TaskOutcome) {
    match outcome {
        TaskOutcome::Completed { result } => {
            match result {
                SystemExecutionResult::Partial { .. } => {
                    yield_task(runtime, task_id, result);
                }
                _ => {
                    complete_task_success(runtime, task_id, result);
                }
            }
        }
        TaskOutcome::Failed { error } => {
            fail_task(runtime, task_id, error);
        }
    }
}

/// Begin execution of next ready task
pub fn begin_next_task(runtime: &mut AsyncRuntimeState) -> Option<TaskDispatchContext> {
    let next_task = poll_next_ready_task(runtime)?;

    let task_id = next_task.id;
    mark_task_running(runtime, task_id);

    let task_idx = runtime.active_tasks.iter().position(|task| task.id == task_id)?;
    let task = runtime.active_tasks[task_idx].clone();
    let resume_state = take_task_intermediate_state(runtime, task_id);

    Some(TaskDispatchContext {
        task,
        intermediate_state: resume_state,
    })
}

/// Take a completed task record from the runtime
pub fn take_completed_task(runtime: &mut AsyncRuntimeState) -> Option<CompletedTaskRecord> {
    if runtime.completed_tasks.is_empty() {
        return None;
    }

    Some(runtime.completed_tasks.remove(0))
}

/// Take intermediate state for a task
pub fn take_task_intermediate_state(runtime: &mut AsyncRuntimeState, task_id: u64) -> Option<Vec<ParameterValue>> {
    let pos = runtime.resume_buffers.iter().position(|state| state.task_id == task_id)?;
    let record = runtime.resume_buffers.remove(pos);
    Some(record.values)
}

/// Store intermediate state for a task
pub fn store_task_intermediate_state(runtime: &mut AsyncRuntimeState, task_id: u64, values: Vec<ParameterValue>) {
    let record = TaskIntermediateState { task_id, values };

    if let Some(pos) = runtime.resume_buffers.iter().position(|state| state.task_id == task_id) {
        runtime.resume_buffers[pos] = record;
    } else {
        runtime.resume_buffers.push(record);
    }
}

/// Clear intermediate state for a task
pub fn clear_task_intermediate_state(runtime: &mut AsyncRuntimeState, task_id: u64) {
    if let Some(pos) = runtime.resume_buffers.iter().position(|state| state.task_id == task_id) {
        runtime.resume_buffers.remove(pos);
    }
}

/// Poll for the next ready task (placeholder implementation)
fn poll_next_ready_task(runtime: &mut AsyncRuntimeState) -> Option<&AsyncTask> {
    if runtime.queued_task_ids.is_empty() {
        return None;
    }

    let task_id = runtime.queued_task_ids[0];
    runtime.active_tasks.iter().find(|task| task.id == task_id)
}

/// Release resources held by a task (placeholder implementation)
fn release_task_resources(runtime: &mut AsyncRuntimeState, task: &AsyncTask) {
    // Remove resource leases for this task
    runtime.resource_leases.retain(|lease| lease.task_id != task.id);
}