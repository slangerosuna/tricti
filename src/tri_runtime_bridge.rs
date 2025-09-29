#![allow(dead_code)]

#[cfg(feature = "tri-runtime")]
use crate::async_runtime::TaskPriority as HostTaskPriority;
#[cfg(feature = "tri-runtime")]
use std::collections::{HashMap, VecDeque};
#[cfg(feature = "tri-runtime")]
use std::fmt;
#[cfg(feature = "tri-runtime")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Experimental bridge that will allow the Rust compiler host to delegate
/// runtime behavior to the TriCTI self-hosted runtime once the port is ready.
///
/// The module is guarded behind the `tri-runtime` cargo feature so we can build
/// and exercise both implementations during the migration.
#[cfg(feature = "tri-runtime")]
pub struct TriRuntimeBridge {
    adapter: Box<dyn TriRuntimeAdapter + Send>,
    initialized: bool,
}

#[cfg(feature = "tri-runtime")]
impl TriRuntimeBridge {
    /// Create a bridge backed by the in-process adapter. This adapter maintains
    /// a lightweight runtime state so the host can start exercising the Tri
    /// dispatch flow before a full interpreter or FFI surface is available.
    pub fn new() -> Self {
        Self::with_adapter(Box::<InProcessTriRuntimeAdapter>::default())
    }

    /// Create a bridge using a custom adapter implementation. The adapter is
    /// responsible for calling into the Tri runtime surface (for example via an
    /// interpreter, bytecode VM, or FFI once available).
    pub fn with_adapter(adapter: Box<dyn TriRuntimeAdapter + Send>) -> Self {
        Self {
            adapter,
            initialized: false,
        }
    }

    pub fn initialized(&self) -> bool {
        self.initialized
    }

    pub fn bootstrap(&mut self) -> Result<(), TriBridgeError> {
        if self.initialized {
            return Ok(());
        }

        self.adapter.bootstrap()?;
        self.initialized = true;
        Ok(())
    }

    pub fn submit_system_task(
        &mut self,
        request: TriSystemTaskRequest,
    ) -> Result<TriAsyncTaskSummary, TriBridgeError> {
        self.ensure_initialized()?;
        self.adapter.submit_task(request)
    }

    pub fn begin_next_task(&mut self) -> Result<Option<TriTaskDispatchContext>, TriBridgeError> {
        self.ensure_initialized()?;
        self.adapter.begin_next_task()
    }

    pub fn apply_task_outcome(
        &mut self,
        task_id: u64,
        outcome: TriTaskOutcome,
    ) -> Result<(), TriBridgeError> {
        self.ensure_initialized()?;
        self.adapter.apply_task_outcome(task_id, outcome)
    }

    pub fn take_completed_task(
        &mut self,
    ) -> Result<Option<TriCompletedTaskRecord>, TriBridgeError> {
        self.ensure_initialized()?;
        self.adapter.take_completed_task()
    }

    fn ensure_initialized(&self) -> Result<(), TriBridgeError> {
        if self.initialized {
            Ok(())
        } else {
            Err(TriBridgeError::NotInitialized)
        }
    }
}

#[cfg(feature = "tri-runtime")]
pub trait TriRuntimeAdapter {
    fn bootstrap(&mut self) -> Result<(), TriBridgeError>;
    fn submit_task(
        &mut self,
        request: TriSystemTaskRequest,
    ) -> Result<TriAsyncTaskSummary, TriBridgeError>;
    fn begin_next_task(&mut self) -> Result<Option<TriTaskDispatchContext>, TriBridgeError>;
    fn apply_task_outcome(
        &mut self,
        task_id: u64,
        outcome: TriTaskOutcome,
    ) -> Result<(), TriBridgeError>;
    fn take_completed_task(&mut self) -> Result<Option<TriCompletedTaskRecord>, TriBridgeError>;
}

#[cfg(feature = "tri-runtime")]
#[derive(Default)]
pub struct InProcessTriRuntimeAdapter {
    state: Option<InProcessTriRuntimeState>,
}

#[cfg(feature = "tri-runtime")]
impl TriRuntimeAdapter for InProcessTriRuntimeAdapter {
    fn bootstrap(&mut self) -> Result<(), TriBridgeError> {
        if self.state.is_none() {
            self.state = Some(InProcessTriRuntimeState::new());
        }
        Ok(())
    }

    fn submit_task(
        &mut self,
        request: TriSystemTaskRequest,
    ) -> Result<TriAsyncTaskSummary, TriBridgeError> {
        let state = self.state_mut()?;
        let task_id = if let Some(host_id) = request.host_task_id {
            state.next_task_id = state.next_task_id.max(host_id + 1);
            host_id
        } else {
            let id = state.next_task_id;
            state.next_task_id += 1;
            id
        };

        let created_at_ms = now_ms();
        let timeout_ms = request.timeout_ms();
        let summary = TriAsyncTaskSummary {
            id: task_id,
            system_name: request.system_name.clone(),
            parameters: request.parameters.clone(),
            priority: request.priority,
            created_at_ms,
            timeout_ms,
            state: TriTaskState::Pending,
        };

        state.queue.push_back(task_id);
        state.tasks.insert(
            task_id,
            TriTaskInternal {
                summary: summary.clone(),
            },
        );
        Ok(summary)
    }

    fn begin_next_task(&mut self) -> Result<Option<TriTaskDispatchContext>, TriBridgeError> {
        let state = self.state_mut()?;

        if state.running_task_count() >= state.config.max_concurrent_systems {
            return Ok(None);
        }

        let initial_len = state.queue.len();
        for _ in 0..initial_len {
            let Some(task_id) = state.queue.pop_front() else {
                break;
            };

            let task_state = state
                .tasks
                .get(&task_id)
                .map(|task| task.summary.state.clone());

            match task_state {
                Some(TriTaskState::Pending) => {
                    let task = state.tasks.get_mut(&task_id).expect("task exists");
                    let started_at_ms = now_ms();
                    let running_state = TriTaskState::Running { started_at_ms };
                    task.summary.state = running_state.clone();

                    let intermediate_state = state.resume_buffers.remove(&task_id);
                    let context = TriTaskDispatchContext {
                        task: task.summary.clone(),
                        intermediate_state,
                    };
                    return Ok(Some(context));
                }
                Some(_) => {
                    state.queue.push_back(task_id);
                }
                None => {
                    // Task disappeared; skip it.
                }
            }
        }

        Ok(None)
    }

    fn apply_task_outcome(
        &mut self,
        task_id: u64,
        outcome: TriTaskOutcome,
    ) -> Result<(), TriBridgeError> {
        let state = self.state_mut()?;
        state.remove_from_queue(task_id);

        match outcome {
            TriTaskOutcome::Completed { result } => match result.clone() {
                TriSystemExecutionResult::Partial {
                    intermediate_state,
                    next_yield_point,
                } => {
                    let task = state.tasks.get_mut(&task_id).ok_or_else(|| {
                        TriBridgeError::Adapter(format!("unknown task {}", task_id))
                    })?;

                    let suspended_state = TriTaskState::Suspended {
                        yield_point: next_yield_point,
                        suspended_at_ms: now_ms(),
                        intermediate_state: Some(intermediate_state.clone()),
                    };

                    task.summary.state = suspended_state;
                    state.resume_buffers.insert(task_id, intermediate_state);
                    Ok(())
                }
                _ => {
                    let task = state.remove_task(task_id)?;
                    state.resume_buffers.remove(&task_id);

                    let final_state = TriTaskState::Completed {
                        completed_at_ms: now_ms(),
                        result,
                    };
                    state.push_completed(task, final_state);
                    Ok(())
                }
            },
            TriTaskOutcome::Failed { error } => {
                let task = state.remove_task(task_id)?;
                state.resume_buffers.remove(&task_id);

                let final_state = TriTaskState::Failed { error };
                state.push_completed(task, final_state);
                Ok(())
            }
        }
    }

    fn take_completed_task(&mut self) -> Result<Option<TriCompletedTaskRecord>, TriBridgeError> {
        let state = self.state_mut()?;
        Ok(state.completed.pop_front())
    }
}

#[cfg(feature = "tri-runtime")]
impl InProcessTriRuntimeAdapter {
    fn state_mut(&mut self) -> Result<&mut InProcessTriRuntimeState, TriBridgeError> {
        self.state.as_mut().ok_or_else(|| {
            TriBridgeError::Adapter("Tri runtime adapter has not been bootstrapped".to_string())
        })
    }
}

#[cfg(feature = "tri-runtime")]
struct InProcessTriRuntimeState {
    config: InProcessRuntimeConfig,
    next_task_id: u64,
    tasks: HashMap<u64, TriTaskInternal>,
    queue: VecDeque<u64>,
    resume_buffers: HashMap<u64, Vec<TriParameterValue>>,
    completed: VecDeque<TriCompletedTaskRecord>,
}

#[cfg(feature = "tri-runtime")]
impl InProcessTriRuntimeState {
    fn new() -> Self {
        Self {
            config: InProcessRuntimeConfig::default(),
            next_task_id: 1,
            tasks: HashMap::new(),
            queue: VecDeque::new(),
            resume_buffers: HashMap::new(),
            completed: VecDeque::new(),
        }
    }

    fn running_task_count(&self) -> usize {
        self.tasks
            .values()
            .filter(|task| matches!(task.summary.state, TriTaskState::Running { .. }))
            .count()
    }

    fn remove_from_queue(&mut self, task_id: u64) {
        self.queue.retain(|queued| *queued != task_id);
    }

    fn remove_task(&mut self, task_id: u64) -> Result<TriTaskInternal, TriBridgeError> {
        self.tasks
            .remove(&task_id)
            .ok_or_else(|| TriBridgeError::Adapter(format!("unknown task {}", task_id)))
    }

    fn push_completed(&mut self, mut task: TriTaskInternal, final_state: TriTaskState) {
        let id = task.summary.id;
        task.summary.state = final_state.clone();
        self.completed.push_back(TriCompletedTaskRecord {
            id,
            state: final_state,
        });
    }
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
struct InProcessRuntimeConfig {
    max_concurrent_systems: usize,
    default_task_timeout_ms: i64,
    resource_lease_timeout_ms: i64,
    scheduling_quantum_ms: i64,
    enable_preemption: bool,
}

#[cfg(feature = "tri-runtime")]
impl Default for InProcessRuntimeConfig {
    fn default() -> Self {
        Self {
            max_concurrent_systems: 64,
            default_task_timeout_ms: 30_000,
            resource_lease_timeout_ms: 5_000,
            scheduling_quantum_ms: 10,
            enable_preemption: true,
        }
    }
}

#[cfg(feature = "tri-runtime")]
#[derive(Clone)]
struct TriTaskInternal {
    summary: TriAsyncTaskSummary,
}

#[cfg(feature = "tri-runtime")]
#[derive(Default)]
struct StubTriRuntimeAdapter;

#[cfg(feature = "tri-runtime")]
impl TriRuntimeAdapter for StubTriRuntimeAdapter {
    fn bootstrap(&mut self) -> Result<(), TriBridgeError> {
        Err(TriBridgeError::OperationUnsupported("bootstrap"))
    }

    fn submit_task(
        &mut self,
        _request: TriSystemTaskRequest,
    ) -> Result<TriAsyncTaskSummary, TriBridgeError> {
        Err(TriBridgeError::OperationUnsupported("submit_task"))
    }

    fn begin_next_task(&mut self) -> Result<Option<TriTaskDispatchContext>, TriBridgeError> {
        Err(TriBridgeError::OperationUnsupported("begin_next_task"))
    }

    fn apply_task_outcome(
        &mut self,
        _task_id: u64,
        _outcome: TriTaskOutcome,
    ) -> Result<(), TriBridgeError> {
        Err(TriBridgeError::OperationUnsupported("apply_task_outcome"))
    }

    fn take_completed_task(&mut self) -> Result<Option<TriCompletedTaskRecord>, TriBridgeError> {
        Err(TriBridgeError::OperationUnsupported("take_completed_task"))
    }
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug)]
pub enum TriBridgeError {
    NotInitialized,
    OperationUnsupported(&'static str),
    Adapter(String),
}

#[cfg(feature = "tri-runtime")]
impl fmt::Display for TriBridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TriBridgeError::NotInitialized => {
                write!(f, "Tri runtime bridge has not been bootstrapped")
            }
            TriBridgeError::OperationUnsupported(op) => {
                write!(f, "Tri runtime adapter does not support `{}`", op)
            }
            TriBridgeError::Adapter(msg) => write!(f, "Tri runtime adapter error: {}", msg),
        }
    }
}

#[cfg(feature = "tri-runtime")]
impl std::error::Error for TriBridgeError {}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
pub struct TriSystemTaskRequest {
    pub system_name: String,
    pub parameters: TriParameterBag,
    pub priority: TriTaskPriority,
    pub timeout: Option<Duration>,
    pub host_task_id: Option<u64>,
}

#[cfg(feature = "tri-runtime")]
impl TriSystemTaskRequest {
    pub fn timeout_ms(&self) -> Option<i64> {
        self.timeout.map(duration_to_millis)
    }
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
pub struct TriParameterBag {
    pub values: Vec<TriParameterValue>,
}

#[cfg(feature = "tri-runtime")]
impl TriParameterBag {
    pub fn empty() -> Self {
        Self { values: Vec::new() }
    }
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
pub struct TriParameterValue {
    pub name: String,
    pub payload: String,
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriTaskPriority {
    Low,
    Normal,
    High,
    Critical,
}

#[cfg(feature = "tri-runtime")]
impl From<HostTaskPriority> for TriTaskPriority {
    fn from(priority: HostTaskPriority) -> Self {
        match priority {
            HostTaskPriority::Low => TriTaskPriority::Low,
            HostTaskPriority::Normal => TriTaskPriority::Normal,
            HostTaskPriority::High => TriTaskPriority::High,
            HostTaskPriority::Critical => TriTaskPriority::Critical,
        }
    }
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
pub struct TriAsyncTaskSummary {
    pub id: u64,
    pub system_name: String,
    pub parameters: TriParameterBag,
    pub priority: TriTaskPriority,
    pub created_at_ms: i64,
    pub timeout_ms: Option<i64>,
    pub state: TriTaskState,
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
pub struct TriTaskDispatchContext {
    pub task: TriAsyncTaskSummary,
    pub intermediate_state: Option<Vec<TriParameterValue>>,
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
pub struct TriCompletedTaskRecord {
    pub id: u64,
    pub state: TriTaskState,
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
pub enum TriTaskState {
    Pending,
    Running {
        started_at_ms: i64,
    },
    Suspended {
        yield_point: TriYieldPoint,
        suspended_at_ms: i64,
        intermediate_state: Option<Vec<TriParameterValue>>,
    },
    Completed {
        completed_at_ms: i64,
        result: TriSystemExecutionResult,
    },
    Failed {
        error: TriAsyncExecutionError,
    },
    Cancelled {
        cancelled_at_ms: i64,
    },
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
pub enum TriTaskOutcome {
    Completed { result: TriSystemExecutionResult },
    Failed { error: TriAsyncExecutionError },
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
pub enum TriSystemExecutionResult {
    Success {
        return_value: Option<String>,
        resources_modified: Vec<String>,
        tables_modified: Vec<String>,
    },
    Partial {
        intermediate_state: Vec<TriParameterValue>,
        next_yield_point: TriYieldPoint,
    },
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
pub enum TriYieldPoint {
    AwaitingResource {
        resource_name: String,
        access_type: TriResourceAccess,
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

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriResourceAccess {
    Immutable,
    Mutable,
    Owned,
}

#[cfg(feature = "tri-runtime")]
#[derive(Debug, Clone)]
pub enum TriAsyncExecutionError {
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

#[cfg(feature = "tri-runtime")]
fn duration_to_millis(duration: Duration) -> i64 {
    let millis = duration.as_millis();
    if millis > i64::MAX as u128 {
        i64::MAX
    } else {
        millis as i64
    }
}

#[cfg(feature = "tri-runtime")]
fn now_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis().min(i64::MAX as u128) as i64,
        Err(_) => 0,
    }
}
