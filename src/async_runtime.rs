use crate::ast::{ResourceAccess, SystemDef, SystemParameter};
use crate::scheduler::{ResourceTracker, SchedulerError};
use crate::system_executor::{ExecutionStepResult, SystemStateMachine, SystemStateMachineExecutor};
use crate::table_runtime::{ColumnValue, TableError, TableRuntime};
#[cfg(feature = "tri-runtime")]
use crate::tri_runtime_bridge::{
    TriAsyncExecutionError, TriBridgeError, TriCompletedTaskRecord, TriParameterBag,
    TriParameterValue, TriResourceAccess, TriRuntimeBridge, TriSystemExecutionResult,
    TriSystemTaskRequest, TriTaskOutcome, TriTaskPriority, TriTaskState, TriYieldPoint,
};
#[cfg(feature = "tri-runtime")]
use std::collections::HashSet;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

/// Result type for async system execution
pub type AsyncResult<T> = Result<T, AsyncExecutionError>;

/// Errors that can occur during async execution
#[derive(Debug, Clone)]
pub enum AsyncExecutionError {
    ResourceConflict {
        system: String,
        resource: String,
        reason: String,
    },
    SchedulingError(SchedulerError),
    TableError(TableError),
    SystemError {
        system: String,
        message: String,
    },
    Timeout {
        system: String,
        duration: Duration,
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

impl From<SchedulerError> for AsyncExecutionError {
    fn from(err: SchedulerError) -> Self {
        AsyncExecutionError::SchedulingError(err)
    }
}

impl From<TableError> for AsyncExecutionError {
    fn from(err: TableError) -> Self {
        AsyncExecutionError::TableError(err)
    }
}

/// Unique identifier for async system execution tasks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(u64);

impl TaskId {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        TaskId(COUNTER.fetch_add(1, Ordering::SeqCst))
    }
}

/// Future representing the completion of an async system execution
pub struct SystemFuture {
    task_id: TaskId,
    priority: TaskPriority,
    completed_tasks: Arc<Mutex<HashMap<TaskId, AsyncResult<SystemExecutionResult>>>>,
    wakers: Arc<Mutex<HashMap<TaskId, Waker>>>,
}

impl SystemFuture {
    pub fn new(
        task_id: TaskId,
        priority: TaskPriority,
        completed_tasks: Arc<Mutex<HashMap<TaskId, AsyncResult<SystemExecutionResult>>>>,
        wakers: Arc<Mutex<HashMap<TaskId, Waker>>>,
    ) -> Self {
        Self {
            task_id,
            priority,
            completed_tasks,
            wakers,
        }
    }

    pub fn task_id(&self) -> TaskId {
        self.task_id
    }

    pub fn priority(&self) -> TaskPriority {
        self.priority
    }

    pub fn is_completed(&self) -> bool {
        self.completed_tasks
            .lock()
            .map(|completed| completed.contains_key(&self.task_id))
            .unwrap_or(false)
    }

    fn try_get_result(&self) -> Option<AsyncResult<SystemExecutionResult>> {
        self.completed_tasks
            .lock()
            .ok()
            .and_then(|completed| completed.get(&self.task_id).cloned())
    }
}

impl Future for SystemFuture {
    type Output = AsyncResult<SystemExecutionResult>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(result) = self.try_get_result() {
            return Poll::Ready(result);
        }

        if let Ok(mut wakers) = self.wakers.lock() {
            wakers.insert(self.task_id, cx.waker().clone());
        }

        if let Some(result) = self.try_get_result() {
            return Poll::Ready(result);
        }

        Poll::Pending
    }
}

/// State of an async system execution
#[derive(Debug, Clone)]
pub enum SystemExecutionState {
    Pending,
    Running {
        started_at: Instant,
    },
    Suspended {
        yield_point: YieldPoint,
        suspended_at: Instant,
    },
    Completed {
        result: SystemExecutionResult,
        completed_at: Instant,
    },
    Failed {
        error: AsyncExecutionError,
        failed_at: Instant,
    },
    Cancelled {
        cancelled_at: Instant,
    },
}

/// Points where async systems can yield control
#[derive(Debug, Clone)]
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
        task_id: TaskId,
    },
    AwaitingSignal {
        signal_name: String,
    },
    Sleeping {
        duration: Duration,
        started_at: Instant,
    },
}

/// Result of system execution
#[derive(Debug, Clone)]
pub enum SystemExecutionResult {
    Success {
        return_value: Option<ColumnValue>,
        resources_modified: Vec<String>,
        tables_modified: Vec<String>,
    },
    Partial {
        intermediate_state: HashMap<String, ColumnValue>,
        next_yield_point: YieldPoint,
    },
}

/// Task representing an async system execution
#[derive(Debug)]
pub struct AsyncSystemTask {
    pub task_id: TaskId,
    pub system_def: SystemDef,
    pub parameters: HashMap<String, ColumnValue>,
    pub state: SystemExecutionState,
    pub resource_handles: HashMap<String, ResourceHandle>,
    pub priority: TaskPriority,
    pub timeout: Option<Duration>,
    pub created_at: Instant,
    pub dependencies: Vec<TaskId>,
}

#[derive(Debug, Clone)]
pub struct CompletedTaskInfo {
    pub task_id: TaskId,
    pub system_def: Option<SystemDef>,
    pub parameters: Option<HashMap<String, ColumnValue>>,
    pub priority: TaskPriority,
    pub result: AsyncResult<SystemExecutionResult>,
}

#[derive(Debug)]
struct SystemExecutionHandle {
    state_machine: SystemStateMachine,
    executor: SystemStateMachineExecutor,
}

#[cfg_attr(not(feature = "tri-runtime"), allow(dead_code))]
enum TaskStepOutcome {
    NeedsContinuation,
    Completed(SystemExecutionResult),
    Waiting(Option<YieldPoint>),
}

/// Priority levels for task scheduling
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Handle to a resource acquired by an async system
#[derive(Debug, Clone)]
pub struct ResourceHandle {
    pub resource_name: String,
    pub access_type: ResourceAccess,
    pub acquired_at: Instant,
    pub lease_duration: Option<Duration>,
}

/// Async system runtime for executing systems concurrently
pub struct AsyncSystemRuntime {
    /// Task queue organized by priority
    task_queue: Arc<Mutex<VecDeque<TaskId>>>,
    /// All active tasks
    active_tasks: Arc<RwLock<HashMap<TaskId, AsyncSystemTask>>>,
    /// Resource tracker for borrow safety
    resource_tracker: Arc<Mutex<ResourceTracker>>,
    /// Table runtime for async queries
    table_runtimes: Arc<RwLock<HashMap<String, Arc<Mutex<TableRuntime>>>>>,
    /// Completed task results
    completed_tasks: Arc<Mutex<HashMap<TaskId, AsyncResult<SystemExecutionResult>>>>,
    /// Wakers for pending futures
    wakers: Arc<Mutex<HashMap<TaskId, Waker>>>,
    /// Registered execution plans for tasks
    execution_handles: Arc<Mutex<HashMap<TaskId, SystemExecutionHandle>>>,
    #[cfg(feature = "tri-runtime")]
    /// Track tasks that have already been acknowledged by the host so the bridge doesn't double-report.
    tri_synced_tasks: Arc<Mutex<HashSet<TaskId>>>,
    #[cfg(feature = "tri-runtime")]
    /// Tasks that have been submitted but are waiting for their execution plan before the Tri runtime activates them.
    tri_pending_activation: Arc<Mutex<HashMap<TaskId, TriSystemTaskRequest>>>,
    /// Runtime configuration
    config: RuntimeConfig,
    #[cfg(feature = "tri-runtime")]
    tri_bridge: Option<Arc<Mutex<TriRuntimeBridge>>>,
}

/// Configuration for the async runtime
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub max_concurrent_systems: usize,
    pub default_task_timeout: Duration,
    pub resource_lease_timeout: Duration,
    pub scheduling_quantum: Duration,
    pub enable_preemption: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_concurrent_systems: 100,
            default_task_timeout: Duration::from_secs(30),
            resource_lease_timeout: Duration::from_secs(5),
            scheduling_quantum: Duration::from_millis(10),
            enable_preemption: true,
        }
    }
}

impl AsyncSystemRuntime {
    /// Create a new async system runtime
    pub fn new(config: RuntimeConfig) -> Self {
        #[cfg(feature = "tri-runtime")]
        let tri_bridge = {
            let mut bridge = TriRuntimeBridge::new();
            if let Err(err) = bridge.bootstrap() {
                panic!("Failed to bootstrap Tri runtime bridge: {}", err);
            }
            Some(Arc::new(Mutex::new(bridge)))
        };

        Self {
            task_queue: Arc::new(Mutex::new(VecDeque::new())),
            active_tasks: Arc::new(RwLock::new(HashMap::new())),
            resource_tracker: Arc::new(Mutex::new(ResourceTracker::new())),
            table_runtimes: Arc::new(RwLock::new(HashMap::new())),
            completed_tasks: Arc::new(Mutex::new(HashMap::new())),
            wakers: Arc::new(Mutex::new(HashMap::new())),
            execution_handles: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "tri-runtime")]
            tri_synced_tasks: Arc::new(Mutex::new(HashSet::new())),
            #[cfg(feature = "tri-runtime")]
            tri_pending_activation: Arc::new(Mutex::new(HashMap::new())),
            config,
            #[cfg(feature = "tri-runtime")]
            tri_bridge,
        }
    }

    /// Register a table runtime for async query execution
    pub fn register_table(&self, name: String, table_runtime: TableRuntime) {
        let mut tables = self.table_runtimes.write().unwrap();
        tables.insert(name, Arc::new(Mutex::new(table_runtime)));
    }

    fn enqueue_local_task(&self, task_id: TaskId) {
        let mut queue = self.task_queue.lock().unwrap();
        queue.push_back(task_id);
    }

    #[cfg(feature = "tri-runtime")]
    fn activate_tri_task(&self, task_id: TaskId) -> AsyncResult<()> {
        let request = {
            let pending = self.tri_pending_activation.lock().unwrap();
            pending.get(&task_id).cloned()
        };

        if let Some(request) = request {
            if let Some(bridge) = &self.tri_bridge {
                let activation_result = {
                    let mut bridge = bridge.lock().unwrap();
                    bridge.submit_system_task(request.clone())
                };

                if let Err(err) = activation_result {
                    return Err(map_bridge_error("submit_system_task", err));
                }
            } else {
                self.enqueue_local_task(task_id);
            }

            if let Ok(mut pending) = self.tri_pending_activation.lock() {
                pending.remove(&task_id);
            }
        }

        Ok(())
    }

    #[cfg(feature = "tri-runtime")]
    fn clear_tri_activation(&self, task_id: TaskId) {
        if let Ok(mut pending) = self.tri_pending_activation.lock() {
            pending.remove(&task_id);
        }
    }

    /// Attach the lowered execution plan for a task so the runtime can drive it.
    pub fn attach_execution_plan(
        &self,
        task_id: TaskId,
        state_machine: SystemStateMachine,
        executor: SystemStateMachineExecutor,
    ) -> AsyncResult<()> {
        let mut handles = self.execution_handles.lock().unwrap();
        if handles.contains_key(&task_id) {
            return Err(AsyncExecutionError::SystemError {
                system: "async_runtime".to_string(),
                message: format!("Execution plan already registered for task {:?}", task_id),
            });
        }

        handles.insert(
            task_id,
            SystemExecutionHandle {
                state_machine,
                executor,
            },
        );

        #[cfg(feature = "tri-runtime")]
        if let Err(err) = self.activate_tri_task(task_id) {
            let mut handles = self.execution_handles.lock().unwrap();
            handles.remove(&task_id);
            return Err(err);
        }

        Ok(())
    }

    /// Submit a system for async execution
    pub fn submit_system(
        &self,
        system_def: SystemDef,
        parameters: HashMap<String, ColumnValue>,
        priority: TaskPriority,
        timeout: Option<Duration>,
    ) -> AsyncResult<SystemFuture> {
        let task_id = TaskId::new();

        // Validate system can be executed
        self.validate_system_execution(&system_def, &parameters)?;

        let task_timeout = timeout.unwrap_or(self.config.default_task_timeout);

        #[cfg(feature = "tri-runtime")]
        let tri_parameters = convert_parameters_to_tri(&parameters);

        // Create task
        let task = AsyncSystemTask {
            task_id,
            system_def: system_def.clone(),
            parameters,
            state: SystemExecutionState::Pending,
            resource_handles: HashMap::new(),
            priority,
            timeout: Some(task_timeout),
            created_at: Instant::now(),
            dependencies: Vec::new(),
        };

        // Register task
        {
            let mut active_tasks = self.active_tasks.write().unwrap();
            active_tasks.insert(task_id, task);
        }

        // Inform the Tri runtime bridge when enabled; otherwise queue locally
        #[cfg(feature = "tri-runtime")]
        {
            if self.tri_bridge.is_some() {
                let request = TriSystemTaskRequest {
                    system_name: system_def.name.clone(),
                    parameters: tri_parameters,
                    priority: TriTaskPriority::from(priority),
                    timeout: Some(task_timeout),
                    host_task_id: Some(task_id.0),
                };

                if let Ok(mut pending) = self.tri_pending_activation.lock() {
                    pending.insert(task_id, request);
                }
            } else {
                self.enqueue_local_task(task_id);
            }
        }

        #[cfg(not(feature = "tri-runtime"))]
        {
            self.enqueue_local_task(task_id);
        }

        // Create future
        let future = SystemFuture::new(
            task_id,
            priority,
            Arc::clone(&self.completed_tasks),
            Arc::clone(&self.wakers),
        );
        Ok(future)
    }

    /// Cancel a running task
    pub fn cancel_task(&self, task_id: TaskId) -> AsyncResult<()> {
        let mut active_tasks = self.active_tasks.write().unwrap();

        if let Some(task) = active_tasks.get_mut(&task_id) {
            // Release any held resources
            self.release_task_resources(task)?;

            task.state = SystemExecutionState::Cancelled {
                cancelled_at: Instant::now(),
            };

            #[cfg(feature = "tri-runtime")]
            self.clear_tri_activation(task_id);

            // Complete the future
            if let Ok(mut completed) = self.completed_tasks.lock() {
                completed.insert(
                    task_id,
                    Err(AsyncExecutionError::Cancelled {
                        system: task.system_def.name.clone(),
                    }),
                );
            }

            // Wake up waiting future
            if let Ok(mut wakers) = self.wakers.lock() {
                if let Some(waker) = wakers.remove(&task_id) {
                    waker.wake();
                }
            }

            Ok(())
        } else {
            Err(AsyncExecutionError::SystemError {
                system: "unknown".to_string(),
                message: format!("Task {:?} not found", task_id),
            })
        }
    }

    /// Execute the next ready task
    pub fn tick(&self) -> AsyncResult<bool> {
        self.sync_tri_runtime()?;

        #[cfg(feature = "tri-runtime")]
        if let Some(bridge) = &self.tri_bridge {
            let dispatch = {
                let mut bridge = bridge.lock().unwrap();
                bridge
                    .begin_next_task()
                    .map_err(|err| map_bridge_error("begin_next_task", err))?
            };

            if let Some(context) = dispatch {
                let task_id = TaskId(context.task.id);

                loop {
                    match self.execute_task_step(task_id) {
                        Ok(TaskStepOutcome::NeedsContinuation) => {
                            continue;
                        }
                        Ok(TaskStepOutcome::Completed(result)) => {
                            let tri_result = convert_system_execution_result(&result);
                            let mut bridge = bridge.lock().unwrap();
                            bridge
                                .apply_task_outcome(
                                    task_id.0,
                                    TriTaskOutcome::Completed { result: tri_result },
                                )
                                .map_err(|err| map_bridge_error("apply_task_outcome", err))?;
                            break;
                        }
                        Ok(TaskStepOutcome::Waiting(yield_point)) => {
                            if let Some(yield_point) = yield_point {
                                let tri_result = TriSystemExecutionResult::Partial {
                                    intermediate_state: Vec::new(),
                                    next_yield_point: convert_yield_point(&yield_point),
                                };
                                let mut bridge = bridge.lock().unwrap();
                                bridge
                                    .apply_task_outcome(
                                        task_id.0,
                                        TriTaskOutcome::Completed { result: tri_result },
                                    )
                                    .map_err(|err| map_bridge_error("apply_task_outcome", err))?;
                            }
                            break;
                        }
                        Err(error) => {
                            let tri_error = convert_async_error(&error);
                            self.fail_task(task_id, error.clone())?;
                            let mut bridge = bridge.lock().unwrap();
                            bridge
                                .apply_task_outcome(
                                    task_id.0,
                                    TriTaskOutcome::Failed { error: tri_error },
                                )
                                .map_err(|err| map_bridge_error("apply_task_outcome", err))?;
                            break;
                        }
                    }
                }

                return Ok(true);
            }
        }

        // Get next ready task
        let task_id = {
            let mut queue = self.task_queue.lock().unwrap();
            queue.pop_front()
        };

        let Some(task_id) = task_id else {
            return Ok(false); // No tasks ready
        };

        match self.execute_task_step(task_id) {
            Ok(TaskStepOutcome::NeedsContinuation) => {
                let mut queue = self.task_queue.lock().unwrap();
                queue.push_back(task_id);
                Ok(true)
            }
            Ok(TaskStepOutcome::Completed(_)) => Ok(true),
            Ok(TaskStepOutcome::Waiting(_)) => Ok(true),
            Err(err) => {
                self.fail_task(task_id, err.clone())?;
                Ok(true)
            }
        }
    }

    fn sync_tri_runtime(&self) -> AsyncResult<()> {
        #[cfg(feature = "tri-runtime")]
        {
            if let Some(bridge) = &self.tri_bridge {
                loop {
                    let record = {
                        let mut bridge = bridge.lock().unwrap();
                        bridge
                            .take_completed_task()
                            .map_err(|err| map_bridge_error("take_completed_task", err))?
                    };

                    let Some(record) = record else {
                        break;
                    };

                    self.process_tri_completion(record)?;
                }
            }
        }

        Ok(())
    }

    /// Execute a single step of a task
    fn execute_task_step(&self, task_id: TaskId) -> AsyncResult<TaskStepOutcome> {
        let mut completion_to_record: Option<SystemExecutionResult> = None;

        let outcome =
            {
                let mut active_tasks = self.active_tasks.write().unwrap();
                let task = active_tasks.get_mut(&task_id).ok_or_else(|| {
                    AsyncExecutionError::SystemError {
                        system: "async_runtime".to_string(),
                        message: format!("Task {:?} not found", task_id),
                    }
                })?;

                if let Some(timeout) = task.timeout {
                    if task.created_at.elapsed() > timeout {
                        return Err(AsyncExecutionError::Timeout {
                            system: task.system_def.name.clone(),
                            duration: timeout,
                        });
                    }
                }

                let system_name = task.system_def.name.clone();

                match &mut task.state {
                    SystemExecutionState::Pending => {
                        self.acquire_task_resources(task)?;
                        task.state = SystemExecutionState::Running {
                            started_at: Instant::now(),
                        };
                        TaskStepOutcome::NeedsContinuation
                    }
                    SystemExecutionState::Running { .. } => {
                        let execution_result = {
                            let mut handles = self.execution_handles.lock().unwrap();
                            let handle = handles.get_mut(&task_id).ok_or_else(|| {
                                AsyncExecutionError::SystemError {
                                    system: system_name.clone(),
                                    message: format!(
                                        "No execution plan registered for task {:?}",
                                        task_id
                                    ),
                                }
                            })?;

                            handle.executor.execute_step(&mut handle.state_machine)?
                        };

                        match execution_result {
                            ExecutionStepResult::Continue => TaskStepOutcome::NeedsContinuation,
                            ExecutionStepResult::Yield(yield_point) => {
                                let yield_point_clone = yield_point.clone();
                                task.state = SystemExecutionState::Suspended {
                                    yield_point: yield_point_clone.clone(),
                                    suspended_at: Instant::now(),
                                };
                                TaskStepOutcome::Waiting(Some(yield_point_clone))
                            }
                            ExecutionStepResult::Completed(result) => {
                                let result_clone = result.clone();
                                task.state = SystemExecutionState::Completed {
                                    result: result_clone.clone(),
                                    completed_at: Instant::now(),
                                };
                                completion_to_record = Some(result_clone.clone());
                                TaskStepOutcome::Completed(result_clone)
                            }
                        }
                    }
                    SystemExecutionState::Suspended { yield_point, .. } => {
                        if self.can_resume_from_yield_point(yield_point)? {
                            task.state = SystemExecutionState::Running {
                                started_at: Instant::now(),
                            };
                            TaskStepOutcome::NeedsContinuation
                        } else {
                            TaskStepOutcome::Waiting(Some(yield_point.clone()))
                        }
                    }
                    SystemExecutionState::Completed { result, .. } => {
                        TaskStepOutcome::Completed(result.clone())
                    }
                    SystemExecutionState::Failed { .. }
                    | SystemExecutionState::Cancelled { .. } => TaskStepOutcome::Waiting(None),
                }
            };

        if let Some(result) = completion_to_record {
            self.complete_task(task_id, Ok(result))?;
        }

        Ok(outcome)
    }

    /// Validate that a system can be executed with given parameters
    fn validate_system_execution(
        &self,
        system_def: &SystemDef,
        parameters: &HashMap<String, ColumnValue>,
    ) -> AsyncResult<()> {
        // Check parameter compatibility
        for param in &system_def.parameters {
            match param {
                SystemParameter::Regular { name, .. } => {
                    if !parameters.contains_key(name) {
                        return Err(AsyncExecutionError::SystemError {
                            system: system_def.name.clone(),
                            message: format!("Missing required parameter: {}", name),
                        });
                    }
                }
                SystemParameter::Resource { name, .. } => {
                    if !parameters.contains_key(name) {
                        return Err(AsyncExecutionError::SystemError {
                            system: system_def.name.clone(),
                            message: format!("Missing required resource: {}", name),
                        });
                    }
                }
                SystemParameter::Query { name, .. } => {
                    // Query parameters are handled differently
                    if !parameters.contains_key(name) {
                        return Err(AsyncExecutionError::SystemError {
                            system: system_def.name.clone(),
                            message: format!("Missing required query parameter: {}", name),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// Acquire resources needed for task execution
    fn acquire_task_resources(&self, task: &mut AsyncSystemTask) -> AsyncResult<()> {
        let mut resource_tracker = self.resource_tracker.lock().unwrap();

        for param in &task.system_def.parameters {
            if let SystemParameter::Resource { name, access, .. } = param {
                // Check if resource can be acquired
                resource_tracker.can_access_resource(name, &task.system_def.name, access)?;

                // Acquire the resource
                let handle = ResourceHandle {
                    resource_name: name.clone(),
                    access_type: access.clone(),
                    acquired_at: Instant::now(),
                    lease_duration: Some(self.config.resource_lease_timeout),
                };

                task.resource_handles.insert(name.clone(), handle);

                // Update resource tracker
                match access {
                    ResourceAccess::Immutable => {
                        resource_tracker.add_access(
                            name,
                            &task.system_def.name,
                            &ResourceAccess::Immutable,
                        )?;
                    }
                    ResourceAccess::Mutable => {
                        resource_tracker.add_access(
                            name,
                            &task.system_def.name,
                            &ResourceAccess::Mutable,
                        )?;
                    }
                    ResourceAccess::Owned => {
                        resource_tracker.add_access(
                            name,
                            &task.system_def.name,
                            &ResourceAccess::Owned,
                        )?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Release resources held by a task
    fn release_task_resources(&self, task: &AsyncSystemTask) -> AsyncResult<()> {
        let mut resource_tracker = self.resource_tracker.lock().unwrap();

        for (resource_name, handle) in &task.resource_handles {
            match handle.access_type {
                ResourceAccess::Immutable => {
                    resource_tracker.remove_access(
                        resource_name,
                        &task.system_def.name,
                        &ResourceAccess::Immutable,
                    );
                }
                ResourceAccess::Mutable => {
                    resource_tracker.remove_access(
                        resource_name,
                        &task.system_def.name,
                        &ResourceAccess::Mutable,
                    );
                }
                ResourceAccess::Owned => {
                    resource_tracker.remove_access(
                        resource_name,
                        &task.system_def.name,
                        &ResourceAccess::Owned,
                    );
                }
            }
        }

        Ok(())
    }

    /// Check if a task can resume from a yield point
    fn can_resume_from_yield_point(&self, yield_point: &YieldPoint) -> AsyncResult<bool> {
        match yield_point {
            YieldPoint::AwaitingResource {
                resource_name,
                access_type,
            } => {
                let resource_tracker = self.resource_tracker.lock().unwrap();
                // Check if resource is available
                Ok(resource_tracker
                    .can_access_resource(resource_name, "temp", access_type)
                    .is_ok())
            }
            YieldPoint::AwaitingTableQuery { .. } => {
                // For now, assume table queries complete immediately
                Ok(true)
            }
            YieldPoint::AwaitingSystemCompletion { task_id, .. } => {
                let completed = self.completed_tasks.lock().unwrap();
                Ok(completed.contains_key(task_id))
            }
            YieldPoint::Sleeping {
                duration,
                started_at,
            } => Ok(started_at.elapsed() >= *duration),
            _ => Ok(true), // Other yield points can resume immediately for now
        }
    }

    /// Complete a task with a result
    fn complete_task(
        &self,
        task_id: TaskId,
        result: AsyncResult<SystemExecutionResult>,
    ) -> AsyncResult<()> {
        // Store the result
        {
            let mut completed = self.completed_tasks.lock().unwrap();
            completed.insert(task_id, result);
        }

        #[cfg(feature = "tri-runtime")]
        self.clear_tri_activation(task_id);

        #[cfg(feature = "tri-runtime")]
        if let Ok(mut synced) = self.tri_synced_tasks.lock() {
            synced.insert(task_id);
        }

        // Wake up the future
        if let Ok(mut wakers) = self.wakers.lock() {
            if let Some(waker) = wakers.remove(&task_id) {
                waker.wake();
            }
        }

        // Release resources
        if let Ok(active_tasks) = self.active_tasks.read() {
            if let Some(task) = active_tasks.get(&task_id) {
                self.release_task_resources(task)?;
            }
        }

        {
            let mut handles = self.execution_handles.lock().unwrap();
            handles.remove(&task_id);
        }

        Ok(())
    }

    /// Mark a task as failed
    fn fail_task(&self, task_id: TaskId, error: AsyncExecutionError) -> AsyncResult<()> {
        let mut active_tasks = self.active_tasks.write().unwrap();

        if let Some(task) = active_tasks.get_mut(&task_id) {
            task.state = SystemExecutionState::Failed {
                error: error.clone(),
                failed_at: Instant::now(),
            };

            // Release resources
            self.release_task_resources(task)?;
        }

        // Complete with error
        self.complete_task(task_id, Err(error))?;
        Ok(())
    }

    #[cfg(feature = "tri-runtime")]
    fn process_tri_completion(&self, record: TriCompletedTaskRecord) -> AsyncResult<()> {
        use TriTaskState::*;

        let task_id = TaskId(record.id);

        if self
            .tri_synced_tasks
            .lock()
            .map(|synced| synced.contains(&task_id))
            .unwrap_or(false)
        {
            return Ok(());
        }

        match record.state {
            Completed { result, .. } => {
                let host_result = convert_tri_system_result(result);

                {
                    let mut active_tasks = self.active_tasks.write().unwrap();
                    if let Some(task) = active_tasks.get_mut(&task_id) {
                        task.state = SystemExecutionState::Completed {
                            result: host_result.clone(),
                            completed_at: Instant::now(),
                        };
                    }
                }

                self.complete_task(task_id, Ok(host_result))?;
            }
            Failed { error } => {
                let host_error = convert_tri_error(error);
                self.fail_task(task_id, host_error)?;
            }
            Cancelled { .. } => {
                let system_name = self
                    .active_tasks
                    .read()
                    .ok()
                    .and_then(|tasks| tasks.get(&task_id).map(|task| task.system_def.name.clone()))
                    .unwrap_or_else(|| "tri_runtime".to_string());

                self.fail_task(
                    task_id,
                    AsyncExecutionError::Cancelled {
                        system: system_name,
                    },
                )?;
            }
            Suspended { yield_point, .. } => {
                let converted = convert_tri_yield_point(yield_point);
                let mut active_tasks = self.active_tasks.write().unwrap();
                if let Some(task) = active_tasks.get_mut(&task_id) {
                    task.state = SystemExecutionState::Suspended {
                        yield_point: converted,
                        suspended_at: Instant::now(),
                    };
                }
            }
            Running { .. } | Pending => {
                // Nothing to do for transient states.
            }
        }

        Ok(())
    }

    /// Drain completed task results along with metadata for higher-level managers.
    pub fn drain_completed_tasks(&self) -> Vec<CompletedTaskInfo> {
        let _ = self.sync_tri_runtime();

        let mut completed = self.completed_tasks.lock().unwrap();
        if completed.is_empty() {
            return Vec::new();
        }

        let drained_capacity = completed.len();
        let mut active_tasks = self.active_tasks.write().unwrap();
        let mut drained = Vec::with_capacity(drained_capacity);

        for (task_id, result) in completed.drain() {
            if let Some(task) = active_tasks.remove(&task_id) {
                let AsyncSystemTask {
                    system_def,
                    parameters,
                    priority,
                    ..
                } = task;

                #[cfg(feature = "tri-runtime")]
                if let Ok(mut synced) = self.tri_synced_tasks.lock() {
                    synced.remove(&task_id);
                }

                drained.push(CompletedTaskInfo {
                    task_id,
                    system_def: Some(system_def),
                    parameters: Some(parameters),
                    priority,
                    result,
                });
            } else {
                #[cfg(feature = "tri-runtime")]
                if let Ok(mut synced) = self.tri_synced_tasks.lock() {
                    synced.remove(&task_id);
                }

                drained.push(CompletedTaskInfo {
                    task_id,
                    system_def: None,
                    parameters: None,
                    priority: TaskPriority::Normal,
                    result,
                });
            }
        }

        drained
    }

    /// Get statistics about the runtime
    pub fn get_stats(&self) -> RuntimeStats {
        let active_tasks = self.active_tasks.read().unwrap();
        let completed_tasks = self.completed_tasks.lock().unwrap();
        let queue = self.task_queue.lock().unwrap();

        let mut stats = RuntimeStats {
            total_tasks_created: active_tasks.len() + completed_tasks.len(),
            active_tasks: active_tasks.len(),
            queued_tasks: queue.len(),
            completed_tasks: completed_tasks.len(),
            failed_tasks: 0,
            cancelled_tasks: 0,
            resource_contentions: 0,
        };

        // Count task states
        for task in active_tasks.values() {
            match task.state {
                SystemExecutionState::Failed { .. } => stats.failed_tasks += 1,
                SystemExecutionState::Cancelled { .. } => stats.cancelled_tasks += 1,
                _ => {}
            }
        }

        stats
    }
}

/// Required for cloning the runtime (simplified)
impl Clone for AsyncSystemRuntime {
    fn clone(&self) -> Self {
        Self {
            task_queue: Arc::new(Mutex::new(VecDeque::new())),
            active_tasks: Arc::new(RwLock::new(HashMap::new())),
            resource_tracker: Arc::new(Mutex::new(ResourceTracker::new())),
            table_runtimes: Arc::new(RwLock::new(HashMap::new())),
            completed_tasks: Arc::new(Mutex::new(HashMap::new())),
            wakers: Arc::new(Mutex::new(HashMap::new())),
            execution_handles: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "tri-runtime")]
            tri_synced_tasks: Arc::new(Mutex::new(HashSet::new())),
            #[cfg(feature = "tri-runtime")]
            tri_pending_activation: Arc::new(Mutex::new(HashMap::new())),
            config: self.config.clone(),
            #[cfg(feature = "tri-runtime")]
            tri_bridge: self.tri_bridge.clone(),
        }
    }
}

/// Runtime statistics
#[derive(Debug, Clone)]
pub struct RuntimeStats {
    pub total_tasks_created: usize,
    pub active_tasks: usize,
    pub queued_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub cancelled_tasks: usize,
    pub resource_contentions: usize,
}

// These methods have been removed as they bypass borrow safety guarantees
// Use ResourceTracker::add_access and ResourceTracker::remove_access instead

#[cfg(feature = "tri-runtime")]
fn map_bridge_error(operation: &str, err: TriBridgeError) -> AsyncExecutionError {
    AsyncExecutionError::SystemError {
        system: format!("tri_runtime_bridge::{}", operation),
        message: err.to_string(),
    }
}

#[cfg(feature = "tri-runtime")]
fn convert_parameters_to_tri(parameters: &HashMap<String, ColumnValue>) -> TriParameterBag {
    let values = parameters
        .iter()
        .map(|(name, value)| TriParameterValue {
            name: name.clone(),
            payload: column_value_to_string(value),
        })
        .collect();

    TriParameterBag { values }
}

#[cfg(feature = "tri-runtime")]
fn convert_system_execution_result(result: &SystemExecutionResult) -> TriSystemExecutionResult {
    match result {
        SystemExecutionResult::Success {
            return_value,
            resources_modified,
            tables_modified,
        } => TriSystemExecutionResult::Success {
            return_value: return_value
                .as_ref()
                .map(|value| column_value_to_string(value)),
            resources_modified: resources_modified.clone(),
            tables_modified: tables_modified.clone(),
        },
        SystemExecutionResult::Partial {
            intermediate_state,
            next_yield_point,
        } => {
            let intermediate_state = intermediate_state
                .iter()
                .map(|(name, value)| TriParameterValue {
                    name: name.clone(),
                    payload: column_value_to_string(value),
                })
                .collect();

            TriSystemExecutionResult::Partial {
                intermediate_state,
                next_yield_point: convert_yield_point(next_yield_point),
            }
        }
    }
}

#[cfg(feature = "tri-runtime")]
fn convert_async_error(error: &AsyncExecutionError) -> TriAsyncExecutionError {
    match error {
        AsyncExecutionError::ResourceConflict {
            system,
            resource,
            reason,
        } => TriAsyncExecutionError::ResourceConflict {
            system: system.clone(),
            resource: resource.clone(),
            reason: reason.clone(),
        },
        AsyncExecutionError::SchedulingError(err) => TriAsyncExecutionError::SchedulingError {
            message: format!("{:?}", err),
        },
        AsyncExecutionError::TableError(err) => TriAsyncExecutionError::TableError {
            message: format!("{:?}", err),
        },
        AsyncExecutionError::SystemError { system, message } => {
            TriAsyncExecutionError::SystemError {
                system: system.clone(),
                message: message.clone(),
            }
        }
        AsyncExecutionError::Timeout { system, duration } => TriAsyncExecutionError::Timeout {
            system: system.clone(),
            duration_ms: duration_to_millis_i64(*duration),
        },
        AsyncExecutionError::Cancelled { system } => TriAsyncExecutionError::Cancelled {
            system: system.clone(),
        },
        AsyncExecutionError::ResourceLifecycleError {
            resource,
            phase,
            reason,
        } => TriAsyncExecutionError::ResourceLifecycleError {
            resource: resource.clone(),
            phase: phase.clone(),
            reason: reason.clone(),
        },
    }
}

#[cfg(feature = "tri-runtime")]
fn convert_yield_point(yield_point: &YieldPoint) -> TriYieldPoint {
    match yield_point {
        YieldPoint::AwaitingResource {
            resource_name,
            access_type,
        } => TriYieldPoint::AwaitingResource {
            resource_name: resource_name.clone(),
            access_type: convert_resource_access(access_type),
        },
        YieldPoint::AwaitingTableQuery {
            table_name,
            query_type,
        } => TriYieldPoint::AwaitingTableQuery {
            table_name: table_name.clone(),
            query_type: query_type.clone(),
        },
        YieldPoint::AwaitingSystemCompletion {
            system_name,
            task_id,
        } => TriYieldPoint::AwaitingSystemCompletion {
            system_name: system_name.clone(),
            task_id: task_id.0,
        },
        YieldPoint::AwaitingSignal { signal_name } => TriYieldPoint::AwaitingSignal {
            signal_name: signal_name.clone(),
        },
        YieldPoint::Sleeping {
            duration,
            started_at,
        } => TriYieldPoint::Sleeping {
            duration_ms: duration_to_millis_i64(*duration),
            started_at_ms: instant_to_millis_since_start(*started_at),
        },
    }
}

#[cfg(feature = "tri-runtime")]
fn convert_resource_access(access: &ResourceAccess) -> TriResourceAccess {
    match access {
        ResourceAccess::Immutable => TriResourceAccess::Immutable,
        ResourceAccess::Mutable => TriResourceAccess::Mutable,
        ResourceAccess::Owned => TriResourceAccess::Owned,
    }
}

#[cfg(feature = "tri-runtime")]
fn convert_tri_system_result(result: TriSystemExecutionResult) -> SystemExecutionResult {
    match result {
        TriSystemExecutionResult::Success {
            return_value,
            resources_modified,
            tables_modified,
        } => SystemExecutionResult::Success {
            return_value: return_value.map(string_to_column_value),
            resources_modified,
            tables_modified,
        },
        TriSystemExecutionResult::Partial {
            intermediate_state,
            next_yield_point,
        } => SystemExecutionResult::Partial {
            intermediate_state: convert_tri_parameter_values(intermediate_state),
            next_yield_point: convert_tri_yield_point(next_yield_point),
        },
    }
}

#[cfg(feature = "tri-runtime")]
fn convert_tri_parameter_values(values: Vec<TriParameterValue>) -> HashMap<String, ColumnValue> {
    values
        .into_iter()
        .map(|param| (param.name, string_to_column_value(param.payload)))
        .collect()
}

#[cfg(feature = "tri-runtime")]
fn convert_tri_error(error: TriAsyncExecutionError) -> AsyncExecutionError {
    match error {
        TriAsyncExecutionError::ResourceConflict {
            system,
            resource,
            reason,
        } => AsyncExecutionError::ResourceConflict {
            system,
            resource,
            reason,
        },
        TriAsyncExecutionError::SchedulingError { message } => {
            AsyncExecutionError::SchedulingError(SchedulerError::SchedulingFailure {
                reason: message,
            })
        }
        TriAsyncExecutionError::TableError { message } => AsyncExecutionError::SystemError {
            system: "tri_runtime".to_string(),
            message,
        },
        TriAsyncExecutionError::SystemError { system, message } => {
            AsyncExecutionError::SystemError { system, message }
        }
        TriAsyncExecutionError::Timeout {
            system,
            duration_ms,
        } => AsyncExecutionError::Timeout {
            system,
            duration: tri_duration_to_duration(duration_ms),
        },
        TriAsyncExecutionError::Cancelled { system } => AsyncExecutionError::Cancelled { system },
        TriAsyncExecutionError::ResourceLifecycleError {
            resource,
            phase,
            reason,
        } => AsyncExecutionError::ResourceLifecycleError {
            resource,
            phase,
            reason,
        },
    }
}

#[cfg(feature = "tri-runtime")]
fn convert_tri_yield_point(yield_point: TriYieldPoint) -> YieldPoint {
    match yield_point {
        TriYieldPoint::AwaitingResource {
            resource_name,
            access_type,
        } => YieldPoint::AwaitingResource {
            resource_name,
            access_type: convert_tri_resource_access(access_type),
        },
        TriYieldPoint::AwaitingTableQuery {
            table_name,
            query_type,
        } => YieldPoint::AwaitingTableQuery {
            table_name,
            query_type,
        },
        TriYieldPoint::AwaitingSystemCompletion {
            system_name,
            task_id,
        } => YieldPoint::AwaitingSystemCompletion {
            system_name,
            task_id: TaskId(task_id),
        },
        TriYieldPoint::AwaitingSignal { signal_name } => YieldPoint::AwaitingSignal { signal_name },
        TriYieldPoint::Sleeping {
            duration_ms,
            started_at_ms: _,
        } => {
            let duration = tri_duration_to_duration(duration_ms);
            let now = Instant::now();
            let started_at = now.checked_sub(duration).unwrap_or(now);

            YieldPoint::Sleeping {
                duration,
                started_at,
            }
        }
    }
}

#[cfg(feature = "tri-runtime")]
fn convert_tri_resource_access(access: TriResourceAccess) -> ResourceAccess {
    match access {
        TriResourceAccess::Immutable => ResourceAccess::Immutable,
        TriResourceAccess::Mutable => ResourceAccess::Mutable,
        TriResourceAccess::Owned => ResourceAccess::Owned,
    }
}

#[cfg(feature = "tri-runtime")]
fn string_to_column_value(payload: String) -> ColumnValue {
    ColumnValue::String(payload)
}

#[cfg(feature = "tri-runtime")]
fn tri_duration_to_duration(duration_ms: i64) -> Duration {
    if duration_ms <= 0 {
        Duration::from_millis(0)
    } else {
        Duration::from_millis(duration_ms as u64)
    }
}

#[cfg(feature = "tri-runtime")]
fn column_value_to_string(value: &ColumnValue) -> String {
    match value {
        ColumnValue::U64(v) => v.to_string(),
        ColumnValue::String(v) => v.clone(),
        ColumnValue::Bool(v) => v.to_string(),
        ColumnValue::F64(bits) => f64::from_bits(*bits).to_string(),
    }
}

#[cfg(feature = "tri-runtime")]
fn duration_to_millis_i64(duration: Duration) -> i64 {
    duration.as_millis().min(i64::MAX as u128) as i64
}

#[cfg(feature = "tri-runtime")]
fn instant_to_millis_since_start(started_at: Instant) -> i64 {
    started_at.elapsed().as_millis().min(i64::MAX as u128) as i64
}
