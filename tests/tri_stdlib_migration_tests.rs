// Rust tests for migrated TriCTI logic from stdlib/prelude.tri
// These tests follow TDD: implement the test first, then the logic.

use peano::stdlib::legacy::*;
use peano::stdlib::modern::{StdError as AdvancedStdError, StdResult as AdvancedStdResult, std_error_message as advanced_std_error_message, std_ok as advanced_std_ok, std_err as advanced_std_err};

#[derive(Debug, Clone, PartialEq)]
pub enum StdErrorKind {
    Message,
    Panic,
    InvalidArgument,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StdError {
    pub kind: StdErrorKind,
    pub message: String,
    pub parameter: Option<String>,
    pub feature: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StdResult<T> {
    pub is_ok: bool,
    pub value: Option<T>,
    pub error: Option<StdError>,
}

pub fn std_error_message(error: &StdError) -> String {
    error.message.clone()
}

pub fn std_error_kind(error: &StdError) -> StdErrorKind {
    error.kind.clone()
}

pub fn std_error_with_source(
    kind: StdErrorKind,
    message: String,
    source: Option<String>,
) -> StdError {
    StdError {
        kind,
        message,
        parameter: None,
        feature: None,
        source,
    }
}

pub fn std_error_invalid_argument(parameter: String, message: String) -> StdError {
    StdError {
        kind: StdErrorKind::InvalidArgument,
        message,
        parameter: Some(parameter),
        feature: None,
        source: None,
    }
}

pub fn std_error_unsupported(feature: String) -> StdError {
    StdError {
        kind: StdErrorKind::Unsupported,
        message: feature.clone(),
        parameter: None,
        feature: Some(feature),
        source: None,
    }
}

pub fn std_ok<T>(value: T) -> StdResult<T> {
    StdResult {
        is_ok: true,
        value: Some(value),
        error: None,
    }
}

pub fn std_err<T>(error: StdError) -> StdResult<T> {
    StdResult {
        is_ok: false,
        value: None,
        error: Some(error),
    }
}

pub fn std_result_is_ok<T>(result: &StdResult<T>) -> bool {
    result.is_ok
}

pub fn std_result_unwrap<T: Clone>(result: &StdResult<T>) -> T {
    if !result.is_ok {
        panic!("attempted to unwrap error result")
    }
    match &result.value {
        Some(value) => value.clone(),
        None => panic!("missing value for ok result"),
    }
}

pub fn std_result_error<T>(result: &StdResult<T>) -> Option<StdError> {
    result.error.clone()
}

pub fn clamp_i64(value: i64, min_value: i64, max_value: i64) -> i64 {
    if value < min_value {
        min_value
    } else if value > max_value {
        max_value
    } else {
        value
    }
}

pub fn sign_i64(value: i64) -> i64 {
    if value < 0 {
        -1
    } else if value > 0 {
        1
    } else {
        0
    }
}

pub fn is_even_i64(value: i64) -> bool {
    value % 2 == 0
}

pub fn is_odd_i64(value: i64) -> bool {
    value % 2 != 0
}

pub fn array_append<T: Clone>(items: &[T], value: T) -> Vec<T> {
    let mut output = items.to_vec();
    output.push(value);
    output
}

pub fn array_set_at<T: Clone>(items: &[T], idx: usize, value: T) -> Vec<T> {
    let mut output = items.to_vec();
    if idx < output.len() {
        output[idx] = value;
    }
    output
}

pub fn array_remove_at<T: Clone>(items: &[T], remove_idx: usize) -> Vec<T> {
    let mut output = items.to_vec();
    if remove_idx < output.len() {
        output.remove(remove_idx);
    }
    output
}

pub fn array_remove_where<T: Clone, F>(items: &[T], predicate: F) -> Vec<T>
where
    F: Fn(&T) -> bool,
{
    items.iter().filter(|item| !predicate(item)).cloned().collect()
}

pub fn array_filter<T: Clone, F>(items: &[T], predicate: F) -> Vec<T>
where
    F: Fn(&T) -> bool,
{
    items.iter().filter(|item| predicate(item)).cloned().collect()
}

pub fn array_find_index<T, F>(items: &[T], predicate: F) -> Option<usize>
where
    F: Fn(&T) -> bool,
{
    items.iter().position(predicate)
}

pub fn array_contains<T, F>(items: &[T], predicate: F) -> bool
where
    F: Fn(&T) -> bool,
{
    items.iter().any(predicate)
}

pub fn array_map<T: Clone, F>(items: &[T], transform: F) -> Vec<T>
where
    F: Fn(&T) -> T,
{
    items.iter().map(transform).collect()
}

pub fn array_all<T, F>(items: &[T], predicate: F) -> bool
where
    F: Fn(&T) -> bool,
{
    items.iter().all(predicate)
}

pub fn array_any<T, F>(items: &[T], predicate: F) -> bool
where
    F: Fn(&T) -> bool,
{
    items.iter().any(predicate)
}

pub fn array_fold<T: Clone, F>(items: &[T], initial: T, reducer: F) -> T
where
    F: Fn(T, &T) -> T,
{
    items.iter().fold(initial, |acc, item| reducer(acc, item))
}

pub fn rem_i64(a: i64, b: i64) -> i64 {
    a % b
}

pub fn abs_i64(x: i64) -> i64 {
    x.abs()
}

pub fn min_i64(a: i64, b: i64) -> i64 {
    a.min(b)
}

pub fn max_i64(a: i64, b: i64) -> i64 {
    a.max(b)
}

pub fn id(x: i64) -> i64 {
    x
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

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

pub fn log_info(message: &str) {
    log_message(&LogLevel::Info, message);
}

pub fn log_error(message: &str) {
    log_message(&LogLevel::Error, message);
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeConfigOptions {
    pub enable_preemption: bool,
    pub scheduling_quantum_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IOConfigOptions {
    pub default_timeout_ms: i64,
    pub enable_tracing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StdConfig {
    pub runtime: RuntimeConfigOptions,
    pub io: IOConfigOptions,
}

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

// Slice utilities (using Rust slices)
pub fn slice_len<T>(s: &[T]) -> usize {
    s.len()
}

pub fn slice_is_empty<T>(s: &[T]) -> bool {
    s.is_empty()
}

pub fn slice_get<T: Clone>(s: &[T], idx: usize) -> Option<T> {
    s.get(idx).cloned()
}

pub fn slice_get_bool(s: &[bool], idx: usize) -> Option<bool> {
    slice_get(s, idx)
}

pub fn slice_is_empty_bool(s: &[bool]) -> bool {
    slice_is_empty(s)
}

// Manual vector implementation (mimicking TriCTI's Vec_i64)
#[derive(Debug, Clone)]
pub struct VecI64 {
    data: Vec<i64>,
}

pub fn new_vec_i64() -> VecI64 {
    VecI64 { data: Vec::new() }
}

pub fn vec_push_i64(v: &mut VecI64, val: i64) {
    v.data.push(val);
}

pub fn vec_pop_i64(v: &mut VecI64) -> Option<i64> {
    v.data.pop()
}

pub fn vec_len_i64(v: &VecI64) -> usize {
    v.data.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_std_error_message() {
        let err = StdError {
            kind: StdErrorKind::Message,
            message: "msg".to_string(),
            parameter: None,
            feature: None,
            source: None,
        };
        assert_eq!(std_error_message(&err), "msg");
    }

    #[test]
    fn test_std_error_kind() {
        let err = StdError {
            kind: StdErrorKind::Panic,
            message: "fail".to_string(),
            parameter: None,
            feature: None,
            source: None,
        };
        assert_eq!(std_error_kind(&err), StdErrorKind::Panic);
    }

    #[test]
    fn test_std_error_with_source() {
        let err = std_error_with_source(
            StdErrorKind::InvalidArgument,
            "bad arg".to_string(),
            Some("src".to_string()),
        );
        assert_eq!(err.kind, StdErrorKind::InvalidArgument);
        assert_eq!(err.message, "bad arg");
        assert_eq!(err.source, Some("src".to_string()));
    }

    #[test]
    fn test_std_error_invalid_argument() {
        let err = std_error_invalid_argument("foo".to_string(), "bad foo".to_string());
        assert_eq!(err.kind, StdErrorKind::InvalidArgument);
        assert_eq!(err.parameter, Some("foo".to_string()));
        assert_eq!(err.message, "bad foo");
    }

    #[test]
    fn test_std_error_unsupported() {
        let err = std_error_unsupported("fancy".to_string());
        assert_eq!(err.kind, StdErrorKind::Unsupported);
        assert_eq!(err.feature, Some("fancy".to_string()));
    }

    #[test]
    fn test_std_ok_and_err() {
        let ok: StdResult<i32> = std_ok(42);
        assert!(ok.is_ok);
        assert_eq!(ok.value, Some(42));
        assert!(ok.error.is_none());

        let err = StdError {
            kind: StdErrorKind::Message,
            message: "fail".to_string(),
            parameter: None,
            feature: None,
            source: None,
        };
        let res: StdResult<i32> = std_err(err.clone());
        assert!(!res.is_ok);
        assert!(res.value.is_none());
        assert_eq!(res.error, Some(err));
    }

    #[test]
    fn test_std_result_is_ok() {
        let ok: StdResult<i32> = std_ok(1);
        let err: StdResult<i32> = std_err(StdError {
            kind: StdErrorKind::Panic,
            message: "fail".to_string(),
            parameter: None,
            feature: None,
            source: None,
        });
        assert!(std_result_is_ok(&ok));
        assert!(!std_result_is_ok(&err));
    }

    #[test]
    #[should_panic]
    fn test_std_result_unwrap_panic() {
        let err: StdResult<i32> = std_err(StdError {
            kind: StdErrorKind::Panic,
            message: "fail".to_string(),
            parameter: None,
            feature: None,
            source: None,
        });
        std_result_unwrap(&err);
    }

    #[test]
    fn test_std_result_unwrap_ok() {
        let ok: StdResult<i32> = std_ok(99);
        assert_eq!(std_result_unwrap(&ok), 99);
    }

    #[test]
    fn test_std_result_error() {
        let err = StdError {
            kind: StdErrorKind::Panic,
            message: "fail".to_string(),
            parameter: None,
            feature: None,
            source: None,
        };
        let res: StdResult<i32> = std_err(err.clone());
        assert_eq!(std_result_error(&res), Some(err));
    }

    #[test]
    fn test_clamp_i64() {
        assert_eq!(clamp_i64(-5, 0, 10), 0);
        assert_eq!(clamp_i64(5, 0, 10), 5);
        assert_eq!(clamp_i64(15, 0, 10), 10);
    }

    #[test]
    fn test_sign_i64() {
        assert_eq!(sign_i64(-42), -1);
        assert_eq!(sign_i64(0), 0);
        assert_eq!(sign_i64(11), 1);
    }

    #[test]
    fn test_is_even_i64() {
        assert!(is_even_i64(12));
        assert!(!is_even_i64(13));
    }

    #[test]
    fn test_is_odd_i64() {
        assert!(!is_odd_i64(12));
        assert!(is_odd_i64(13));
    }

    #[test]
    fn test_array_append() {
        let arr = vec![1, 2, 3];
        let result = array_append(&arr, 4);
        assert_eq!(result, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_array_set_at() {
        let arr = vec![1, 2, 3];
        let result = array_set_at(&arr, 1, 99);
        assert_eq!(result, vec![1, 99, 3]);
        
        // Test out of bounds
        let result = array_set_at(&arr, 10, 99);
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn test_array_remove_at() {
        let arr = vec![1, 2, 3, 4];
        let result = array_remove_at(&arr, 1);
        assert_eq!(result, vec![1, 3, 4]);
        
        // Test out of bounds
        let result = array_remove_at(&arr, 10);
        assert_eq!(result, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_array_remove_where() {
        let arr = vec![1, 2, 3, 4, 5];
        let result = array_remove_where(&arr, |&x| x % 2 == 0);
        assert_eq!(result, vec![1, 3, 5]);
    }

    #[test]
    fn test_array_filter() {
        let arr = vec![1, 2, 3, 4, 5];
        let result = array_filter(&arr, |&x| x % 2 == 0);
        assert_eq!(result, vec![2, 4]);
    }

    #[test]
    fn test_array_find_index() {
        let arr = vec![1, 2, 3, 4, 5];
        assert_eq!(array_find_index(&arr, |&x| x == 3), Some(2));
        assert_eq!(array_find_index(&arr, |&x| x == 99), None);
    }

    #[test]
    fn test_array_contains() {
        let arr = vec![1, 2, 3, 4, 5];
        assert!(array_contains(&arr, |&x| x == 3));
        assert!(!array_contains(&arr, |&x| x == 99));
    }

    #[test]
    fn test_array_map() {
        let arr = vec![1, 2, 3];
        let result = array_map(&arr, |&x| x * 2);
        assert_eq!(result, vec![2, 4, 6]);
    }

    #[test]
    fn test_array_all() {
        let arr = vec![2, 4, 6, 8];
        assert!(array_all(&arr, |&x| x % 2 == 0));
        
        let arr = vec![2, 3, 6, 8];
        assert!(!array_all(&arr, |&x| x % 2 == 0));
    }

    #[test]
    fn test_array_any() {
        let arr = vec![1, 3, 5, 7];
        assert!(!array_any(&arr, |&x| x % 2 == 0));
        
        let arr = vec![1, 2, 5, 7];
        assert!(array_any(&arr, |&x| x % 2 == 0));
    }

    #[test]
    fn test_array_fold() {
        let arr = vec![1, 2, 3, 4];
        let result = array_fold(&arr, 0, |acc, &x| acc + x);
        assert_eq!(result, 10);
        
        let result = array_fold(&arr, 1, |acc, &x| acc * x);
        assert_eq!(result, 24);
    }

    #[test]
    fn test_rem_i64() {
        assert_eq!(rem_i64(7, 3), 1);
        assert_eq!(rem_i64(10, 2), 0);
        assert_eq!(rem_i64(8, 5), 3);
    }

    #[test]
    fn test_abs_i64() {
        assert_eq!(abs_i64(-5), 5);
        assert_eq!(abs_i64(5), 5);
        assert_eq!(abs_i64(0), 0);
    }

    #[test]
    fn test_min_i64() {
        assert_eq!(min_i64(5, 3), 3);
        assert_eq!(min_i64(-1, 1), -1);
        assert_eq!(min_i64(10, 10), 10);
    }

    #[test]
    fn test_max_i64() {
        assert_eq!(max_i64(5, 3), 5);
        assert_eq!(max_i64(-1, 1), 1);
        assert_eq!(max_i64(10, 10), 10);
    }

    #[test]
    fn test_id() {
        assert_eq!(id(42), 42);
        assert_eq!(id(-10), -10);
        assert_eq!(id(0), 0);
    }

    #[test]
    fn test_log_message() {
        // Test that log_message doesn't panic
        log_message(&LogLevel::Info, "test message");
        log_message(&LogLevel::Error, "error message");
    }

    #[test]
    fn test_log_info() {
        log_info("info message");
    }

    #[test]
    fn test_log_error() {
        log_error("error message");
    }

    #[test]
    fn test_std_default_config() {
        let config = std_default_config();
        assert_eq!(config.runtime.enable_preemption, true);
        assert_eq!(config.runtime.scheduling_quantum_ms, 10);
        assert_eq!(config.io.default_timeout_ms, 5000);
        assert_eq!(config.io.enable_tracing, false);
    }

    #[test]
    fn test_slice_len() {
        let arr = vec![1, 2, 3];
        assert_eq!(slice_len(&arr), 3);
        
        let empty: Vec<i32> = vec![];
        assert_eq!(slice_len(&empty), 0);
    }

    #[test]
    fn test_slice_is_empty() {
        let arr = vec![1, 2, 3];
        assert!(!slice_is_empty(&arr));
        
        let empty: Vec<i32> = vec![];
        assert!(slice_is_empty(&empty));
    }

    #[test]
    fn test_slice_get() {
        let arr = vec![10, 20, 30];
        assert_eq!(slice_get(&arr, 0), Some(10));
        assert_eq!(slice_get(&arr, 1), Some(20));
        assert_eq!(slice_get(&arr, 2), Some(30));
        assert_eq!(slice_get(&arr, 3), None);
    }

    #[test]
    fn test_slice_get_bool() {
        let arr = vec![true, false, true];
        assert_eq!(slice_get_bool(&arr, 0), Some(true));
        assert_eq!(slice_get_bool(&arr, 1), Some(false));
        assert_eq!(slice_get_bool(&arr, 2), Some(true));
        assert_eq!(slice_get_bool(&arr, 3), None);
    }

    #[test]
    fn test_slice_is_empty_bool() {
        let arr = vec![true, false];
        assert!(!slice_is_empty_bool(&arr));
        let empty: Vec<bool> = vec![];
        assert!(slice_is_empty_bool(&empty));
    }

    #[test]
    fn test_vec_i64() {
        let mut v = new_vec_i64();
        assert_eq!(vec_len_i64(&v), 0);

        vec_push_i64(&mut v, 10);
        assert_eq!(vec_len_i64(&v), 1);

        vec_push_i64(&mut v, 20);
        assert_eq!(vec_len_i64(&v), 2);

        assert_eq!(vec_pop_i64(&mut v), Some(20));
        assert_eq!(vec_len_i64(&v), 1);

        assert_eq!(vec_pop_i64(&mut v), Some(10));
        assert_eq!(vec_len_i64(&v), 0);

        assert_eq!(vec_pop_i64(&mut v), None);
    }
}

// Tests for async runtime types and functions from stdlib/runtime_async.tri

#[derive(Debug, Clone, PartialEq)]
pub enum TaskPriority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResourceAccess {
    Immutable,
    Mutable,
    Owned,
}

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

#[derive(Debug, Clone, PartialEq)]
pub enum TaskState {
    Pending,
    Running { started_at_ms: i64 },
    Suspended {
        yield_point: YieldPoint,
        suspended_at_ms: i64,
        intermediate_state: Option<Vec<ParameterValue>>,
    },
    Completed {
        completed_at_ms: i64,
        result: SystemExecutionResult,
    },
    Failed { error: AsyncExecutionError },
    Cancelled { cancelled_at_ms: i64 },
}

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

#[derive(Debug, Clone, PartialEq)]
pub enum TaskOutcome {
    Completed { result: SystemExecutionResult },
    Failed { error: AsyncExecutionError },
}

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

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceHandle {
    pub resource_name: String,
    pub access_type: ResourceAccess,
    pub acquired_at_ms: i64,
    pub lease_duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceRequest {
    pub resource_name: String,
    pub access_type: ResourceAccess,
    pub lease_duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskWaker {
    pub task_id: u64,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParameterValue {
    pub name: String,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParameterBag {
    pub values: Vec<ParameterValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeConfig {
    pub max_concurrent_systems: i64,
    pub default_task_timeout_ms: i64,
    pub resource_lease_timeout_ms: i64,
    pub scheduling_quantum_ms: i64,
    pub enable_preemption: bool,
}

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

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedTaskRecord {
    pub id: u64,
    pub state: TaskState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveResourceLease {
    pub resource_name: String,
    pub task_id: u64,
    pub access_type: ResourceAccess,
    pub acquired_at_ms: i64,
    pub lease_duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceWaiter {
    pub resource_name: String,
    pub task_id: u64,
    pub access_type: ResourceAccess,
    pub requested_at_ms: i64,
    pub lease_duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskResumeResult {
    pub resumed: bool,
    pub intermediate_state: Option<Vec<ParameterValue>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskIntermediateState {
    pub task_id: u64,
    pub values: Vec<ParameterValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskDispatchContext {
    pub task: AsyncTask,
    pub intermediate_state: Option<Vec<ParameterValue>>,
}

pub fn default_runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        max_concurrent_systems: 100,
        default_task_timeout_ms: 30_000,
        resource_lease_timeout_ms: 5_000,
        scheduling_quantum_ms: 10,
        enable_preemption: true,
    }
}

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

pub fn mark_task_running(runtime: &mut AsyncRuntimeState, task_id: u64) {
    if let Some(idx) = runtime.active_tasks.iter().position(|task| task.id == task_id) {
        let task = &mut runtime.active_tasks[idx];
        task.state = TaskState::Running {
            started_at_ms: now_ms(),
        };
    } else {
        panic!("mark_task_running: unknown task id");
    }
}

pub fn complete_task(runtime: &mut AsyncRuntimeState, task_id: u64, state: TaskState) {
    if let Some(pos) = runtime.queued_task_ids.iter().position(|&id| id == task_id) {
        runtime.queued_task_ids.remove(pos);
        runtime.resource_summary.queued_tasks = std::cmp::max(runtime.resource_summary.queued_tasks - 1, 0);
    }

    if let Some(pos) = runtime.wakers.iter().position(|waker| waker.task_id == task_id) {
        runtime.wakers.remove(pos);
    }

    if let Some(idx) = runtime.active_tasks.iter().position(|task| task.id == task_id) {
        let task = runtime.active_tasks.remove(idx);
        clear_task_intermediate_state(runtime, task_id);
        release_task_resources(runtime, &task);

        runtime.completed_tasks.push(CompletedTaskRecord {
            id: task_id,
            state: state.clone(),
        });

        runtime.resource_summary.active_tasks = std::cmp::max(runtime.resource_summary.active_tasks - 1, 0);
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
    } else {
        panic!("complete_task: unknown task id");
    }
}

pub fn suspend_task(
    runtime: &mut AsyncRuntimeState,
    task_id: u64,
    yield_point: YieldPoint,
    intermediate_state: Option<Vec<ParameterValue>>,
) {
    if let Some(idx) = runtime.active_tasks.iter().position(|task| task.id == task_id) {
        let task = &mut runtime.active_tasks[idx];
        task.state = TaskState::Suspended {
            yield_point,
            suspended_at_ms: now_ms(),
            intermediate_state: intermediate_state.clone(),
        };
    } else {
        panic!("suspend_task: unknown task id");
    }
}

pub fn resume_task(runtime: &mut AsyncRuntimeState, task_id: u64) -> TaskResumeResult {
    if let Some(idx) = runtime.active_tasks.iter().position(|task| task.id == task_id) {
        let intermediate_state = match &runtime.active_tasks[idx].state {
            TaskState::Suspended { intermediate_state, .. } => intermediate_state.clone(),
            TaskState::Pending => {
                if !runtime.queued_task_ids.contains(&task_id) {
                    runtime.queued_task_ids.push(task_id);
                    runtime.resource_summary.queued_tasks += 1;
                }
                return TaskResumeResult {
                    resumed: true,
                    intermediate_state: None,
                };
            }
            _ => {
                return TaskResumeResult {
                    resumed: false,
                    intermediate_state: None,
                };
            }
        };

        // Now modify the task state
        let task = &mut runtime.active_tasks[idx];
        task.state = TaskState::Pending;

        if !runtime.queued_task_ids.contains(&task_id) {
            runtime.queued_task_ids.push(task_id);
            runtime.resource_summary.queued_tasks += 1;
        }

        if let Some(values) = &intermediate_state {
            store_task_intermediate_state(runtime, task_id, values.clone());
        }

        TaskResumeResult {
            resumed: true,
            intermediate_state,
        }
    } else {
        TaskResumeResult {
            resumed: false,
            intermediate_state: None,
        }
    }
}

pub fn cancel_task(runtime: &mut AsyncRuntimeState, task_id: u64) {
    if let Some(pos) = runtime.queued_task_ids.iter().position(|&id| id == task_id) {
        runtime.queued_task_ids.remove(pos);
        runtime.resource_summary.queued_tasks = std::cmp::max(runtime.resource_summary.queued_tasks - 1, 0);
    }

    let state = TaskState::Cancelled {
        cancelled_at_ms: now_ms(),
    };

    complete_task(runtime, task_id, state);
}

pub fn fail_task(runtime: &mut AsyncRuntimeState, task_id: u64, error: AsyncExecutionError) {
    if let Some(pos) = runtime.queued_task_ids.iter().position(|&id| id == task_id) {
        runtime.queued_task_ids.remove(pos);
        runtime.resource_summary.queued_tasks = std::cmp::max(runtime.resource_summary.queued_tasks - 1, 0);
    }

    let state = TaskState::Failed { error };

    complete_task(runtime, task_id, state);
}

pub fn complete_task_success(runtime: &mut AsyncRuntimeState, task_id: u64, result: SystemExecutionResult) {
    let state = TaskState::Completed {
        completed_at_ms: now_ms(),
        result,
    };

    complete_task(runtime, task_id, state);
}

pub fn yield_task(runtime: &mut AsyncRuntimeState, task_id: u64, partial: SystemExecutionResult) {
    match partial {
        SystemExecutionResult::Partial { intermediate_state, next_yield_point } => {
            suspend_task(runtime, task_id, next_yield_point, Some(intermediate_state));
        }
        _ => {
            panic!("yield_task requires a partial execution result");
        }
    }
}

pub fn suspend_task_for_yield_point(runtime: &mut AsyncRuntimeState, task_id: u64, yield_point: YieldPoint) {
    suspend_task(runtime, task_id, yield_point, None);
}

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

pub fn begin_next_task(runtime: &mut AsyncRuntimeState) -> Option<TaskDispatchContext> {
    runtime_housekeeping(runtime);

    let running = count_running_tasks(&runtime.active_tasks);
    if running >= runtime.config.max_concurrent_systems {
        return None;
    }

    next_ready_task(runtime).map(|task| {
        let task_id = task.id;
        mark_task_running(runtime, task_id);

        let resume_state = take_task_intermediate_state(runtime, task_id);

        TaskDispatchContext {
            task,
            intermediate_state: resume_state,
        }
    })
}

pub fn take_completed_task(runtime: &mut AsyncRuntimeState) -> Option<CompletedTaskRecord> {
    if runtime.completed_tasks.is_empty() {
        return None;
    }

    Some(runtime.completed_tasks.remove(0))
}

pub fn take_task_intermediate_state(runtime: &mut AsyncRuntimeState, task_id: u64) -> Option<Vec<ParameterValue>> {
    if let Some(idx) = runtime.resume_buffers.iter().position(|state| state.task_id == task_id) {
        let record = runtime.resume_buffers.remove(idx);
        Some(record.values)
    } else {
        None
    }
}

pub fn store_task_intermediate_state(runtime: &mut AsyncRuntimeState, task_id: u64, values: Vec<ParameterValue>) {
    let record = TaskIntermediateState {
        task_id,
        values,
    };

    if let Some(idx) = runtime.resume_buffers.iter().position(|state| state.task_id == task_id) {
        runtime.resume_buffers[idx] = record;
    } else {
        runtime.resume_buffers.push(record);
    }
}

pub fn clear_task_intermediate_state(runtime: &mut AsyncRuntimeState, task_id: u64) {
    if let Some(idx) = runtime.resume_buffers.iter().position(|state| state.task_id == task_id) {
        runtime.resume_buffers.remove(idx);
    }
}

pub fn acquire_resource_for_task(runtime: &mut AsyncRuntimeState, task_id: u64, request: ResourceRequest) -> bool {
    try_acquire_resource(runtime, task_id, &request.resource_name, request.access_type, request.lease_duration_ms, true)
}

pub fn acquire_resources_for_task(runtime: &mut AsyncRuntimeState, task_id: u64, requests: Vec<ResourceRequest>) -> bool {
    if requests.is_empty() {
        return true;
    }

    let mut acquired_names: Vec<String> = Vec::new();

    for request in &requests {
        let success = try_acquire_resource(runtime, task_id, &request.resource_name, request.access_type.clone(), request.lease_duration_ms, true);
        if !success {
            for name in &acquired_names {
                release_resource_for_task(runtime, task_id, name);
            }
            return false;
        }
        acquired_names.push(request.resource_name.clone());
    }

    true
}

pub fn release_resource_for_task(runtime: &mut AsyncRuntimeState, task_id: u64, resource_name: &str) {
    runtime.resource_leases.retain(|lease| !(lease.task_id == task_id && lease.resource_name == resource_name));
    wake_waiters_for_resource(runtime, resource_name);

    if let Some(task_idx) = runtime.active_tasks.iter().position(|task| task.id == task_id) {
        let task = &mut runtime.active_tasks[task_idx];
        if let Some(handle_idx) = task.resource_handles.iter().position(|handle| handle.resource_name == resource_name) {
            task.resource_handles.remove(handle_idx);
        }
    }
}

pub fn release_task_resources(runtime: &mut AsyncRuntimeState, task: &AsyncTask) {
    for handle in &task.resource_handles {
        runtime.resource_leases.retain(|lease| !(lease.task_id == task.id && lease.resource_name == handle.resource_name));
        wake_waiters_for_resource(runtime, &handle.resource_name);
    }
}

pub fn try_acquire_resource(
    runtime: &mut AsyncRuntimeState,
    task_id: u64,
    resource_name: &str,
    access_type: ResourceAccess,
    lease_duration_ms: Option<i64>,
    enqueue_on_conflict: bool,
) -> bool {
    let task_idx = match runtime.active_tasks.iter().position(|task| task.id == task_id) {
        Some(idx) => idx,
        None => return false,
    };

    let task = &runtime.active_tasks[task_idx];

    // Check if task already has this resource
    if task.resource_handles.iter().any(|handle| handle.resource_name == resource_name) {
        return true;
    }

    // Check for conflicts with existing leases
    let conflict = runtime.resource_leases.iter().any(|lease| {
        lease.resource_name == resource_name
            && lease.task_id != task_id
            && resource_access_conflict(&lease.access_type, &access_type)
    });

    if conflict {
        if enqueue_on_conflict {
            let existing_waiter = runtime.resource_waiters.iter().any(|waiter| waiter.task_id == task_id && waiter.resource_name == resource_name);
            if !existing_waiter {
                let waiter = ResourceWaiter {
                    resource_name: resource_name.to_string(),
                    task_id,
                    access_type: access_type.clone(),
                    requested_at_ms: now_ms(),
                    lease_duration_ms,
                };
                runtime.resource_waiters.push(waiter);
                runtime.resource_summary.resource_contentions += 1;
            }
        }
        return false;
    }

    let timestamp = now_ms();

    let handle = ResourceHandle {
        resource_name: resource_name.to_string(),
        access_type: access_type.clone(),
        acquired_at_ms: timestamp,
        lease_duration_ms,
    };

    let task = &mut runtime.active_tasks[task_idx];
    task.resource_handles.push(handle);

    let lease_record = ActiveResourceLease {
        resource_name: resource_name.to_string(),
        task_id,
        access_type,
        acquired_at_ms: timestamp,
        lease_duration_ms,
    };

    runtime.resource_leases.push(lease_record);

    true
}

pub fn wake_waiters_for_resource(runtime: &mut AsyncRuntimeState, resource_name: &str) {
    let mut idx = 0;
    while idx < runtime.resource_waiters.len() {
        let waiter_resource_name = runtime.resource_waiters[idx].resource_name.clone();
        let waiter_task_id = runtime.resource_waiters[idx].task_id;
        let waiter_access_type = runtime.resource_waiters[idx].access_type.clone();
        let waiter_lease_duration_ms = runtime.resource_waiters[idx].lease_duration_ms;

        if waiter_resource_name != resource_name {
            idx += 1;
            continue;
        }

        let acquired = try_acquire_resource(runtime, waiter_task_id, &waiter_resource_name, waiter_access_type, waiter_lease_duration_ms, false);
        if acquired {
            runtime.resource_waiters.remove(idx);
            let _ = resume_task(runtime, waiter_task_id);
        } else {
            idx += 1;
        }
    }
}

pub fn register_task_waker(runtime: &mut AsyncRuntimeState, task_id: u64, token: String) {
    let waker = TaskWaker { task_id, token };

    if let Some(idx) = runtime.wakers.iter().position(|existing| existing.task_id == task_id) {
        runtime.wakers[idx] = waker;
    } else {
        runtime.wakers.push(waker);
    }
}

pub fn take_task_waker(runtime: &mut AsyncRuntimeState, task_id: u64) -> Option<TaskWaker> {
    if let Some(idx) = runtime.wakers.iter().position(|waker| waker.task_id == task_id) {
        Some(runtime.wakers.remove(idx))
    } else {
        None
    }
}

pub fn wake_task(runtime: &mut AsyncRuntimeState, task_id: u64) -> bool {
    let _ = take_task_waker(runtime, task_id);
    let result = resume_task(runtime, task_id);
    result.resumed
}

pub fn runtime_housekeeping(runtime: &mut AsyncRuntimeState) {
    let now = now_ms();
    wake_sleeping_tasks(runtime, now);
    expire_resource_leases(runtime, now);
    check_task_timeouts(runtime, now);
}

pub fn poll_next_ready_task(runtime: &mut AsyncRuntimeState) -> Option<AsyncTask> {
    runtime_housekeeping(runtime);

    let running = count_running_tasks(&runtime.active_tasks);
    if running >= runtime.config.max_concurrent_systems {
        return None;
    }

    next_ready_task(runtime)
}

pub fn wake_sleeping_tasks(runtime: &mut AsyncRuntimeState, now_ms: i64) {
    let task_ids_to_resume: Vec<u64> = runtime.active_tasks.iter().filter_map(|task| {
        if let TaskState::Suspended { yield_point: YieldPoint::Sleeping { duration_ms, started_at_ms }, .. } = &task.state {
            if now_ms >= started_at_ms + duration_ms {
                Some(task.id)
            } else {
                None
            }
        } else {
            None
        }
    }).collect();

    for task_id in task_ids_to_resume {
        let _ = resume_task(runtime, task_id);
    }
}

pub fn expire_resource_leases(runtime: &mut AsyncRuntimeState, now_ms: i64) {
    let leases_to_expire: Vec<(u64, String)> = runtime.resource_leases.iter().filter_map(|lease| {
        if let Some(duration_ms) = lease.lease_duration_ms {
            if now_ms >= lease.acquired_at_ms + duration_ms {
                Some((lease.task_id, lease.resource_name.clone()))
            } else {
                None
            }
        } else {
            None
        }
    }).collect();

    for (task_id, resource_name) in leases_to_expire {
        release_resource_for_task(runtime, task_id, &resource_name);
    }
}

pub fn check_task_timeouts(runtime: &mut AsyncRuntimeState, now_ms: i64) {
    let tasks_to_timeout: Vec<(u64, i64)> = runtime.active_tasks.iter().filter_map(|task| {
        if let Some(timeout_ms) = task.timeout_ms {
            if now_ms >= task.created_at_ms + timeout_ms {
                Some((task.id, timeout_ms))
            } else {
                None
            }
        } else {
            None
        }
    }).collect();

    for (task_id, timeout_value) in tasks_to_timeout {
        let error = AsyncExecutionError::Timeout {
            system: runtime.active_tasks.iter().find(|t| t.id == task_id).unwrap().system_name.clone(),
            duration_ms: timeout_value,
        };
        fail_task(runtime, task_id, error);
    }
}

pub fn runtime_next_deadline_ms(runtime: &AsyncRuntimeState) -> Option<i64> {
    let mut deadline = None;

    for task in &runtime.active_tasks {
        if let Some(timeout_ms) = task.timeout_ms {
            let candidate = task.created_at_ms + timeout_ms;
            deadline = min_option_i64(deadline, candidate);
        }

        if let TaskState::Suspended { yield_point: YieldPoint::Sleeping { duration_ms, started_at_ms }, .. } = &task.state {
            let candidate = started_at_ms + duration_ms;
            deadline = min_option_i64(deadline, candidate);
        }
    }

    for lease in &runtime.resource_leases {
        if let Some(duration_ms) = lease.lease_duration_ms {
            let candidate = lease.acquired_at_ms + duration_ms;
            deadline = min_option_i64(deadline, candidate);
        }
    }

    deadline
}

pub fn next_ready_task(runtime: &mut AsyncRuntimeState) -> Option<AsyncTask> {
    if runtime.queued_task_ids.is_empty() {
        return None;
    }

    let task_id = runtime.queued_task_ids.remove(0);
    runtime.resource_summary.queued_tasks = std::cmp::max(runtime.resource_summary.queued_tasks - 1, 0);

    runtime.active_tasks.iter().find(|task| task.id == task_id).cloned()
}

pub fn empty_parameter_bag() -> ParameterBag {
    ParameterBag { values: Vec::new() }
}

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

pub fn now_ms() -> i64 {
    // Mock implementation for testing
    1000
}

pub fn resource_access_conflict(existing: &ResourceAccess, requested: &ResourceAccess) -> bool {
    match existing {
        ResourceAccess::Immutable => !matches!(requested, ResourceAccess::Immutable),
        ResourceAccess::Mutable => true,
        ResourceAccess::Owned => true,
    }
}

pub fn count_running_tasks(tasks: &[AsyncTask]) -> i64 {
    tasks.iter().filter(|task| matches!(task.state, TaskState::Running { .. })).count() as i64
}

pub fn min_option_i64(current: Option<i64>, candidate: i64) -> Option<i64> {
    match current {
        Some(existing) => Some(if candidate < existing { candidate } else { existing }),
        None => Some(candidate),
    }
}

#[cfg(test)]
mod async_runtime_tests {
    use super::*;

    // Mock now_ms for testing
    fn mock_now_ms() -> i64 {
        1000
    }

    #[test]
    fn test_default_runtime_config() {
        let config = default_runtime_config();
        assert_eq!(config.max_concurrent_systems, 100);
        assert_eq!(config.default_task_timeout_ms, 30_000);
        assert_eq!(config.resource_lease_timeout_ms, 5_000);
        assert_eq!(config.scheduling_quantum_ms, 10);
        assert_eq!(config.enable_preemption, true);
    }

    #[test]
    fn test_new_async_runtime() {
        let config = default_runtime_config();
        let runtime = new_async_runtime(Some(config.clone()));

        assert_eq!(runtime.config, config);
        assert_eq!(runtime.next_task_id, 1);
        assert!(runtime.active_tasks.is_empty());
        assert!(runtime.completed_tasks.is_empty());
        assert!(runtime.queued_task_ids.is_empty());
        assert!(runtime.wakers.is_empty());
        assert!(runtime.resource_leases.is_empty());
        assert!(runtime.resource_waiters.is_empty());
        assert!(runtime.resume_buffers.is_empty());
        assert_eq!(runtime.resource_summary, empty_runtime_stats());
    }

    #[test]
    fn test_submit_task() {
        let mut runtime = new_async_runtime(None);
        let params = empty_parameter_bag();

        let task = submit_task(&mut runtime, "test_system".to_string(), params, TaskPriority::Normal, Some(5000));

        assert_eq!(task.id, 1);
        assert_eq!(task.system_name, "test_system");
        assert_eq!(task.priority, TaskPriority::Normal);
        assert_eq!(task.timeout_ms, Some(5000));
        assert_eq!(task.state, TaskState::Pending);

        assert_eq!(runtime.next_task_id, 2);
        assert_eq!(runtime.active_tasks.len(), 1);
        assert_eq!(runtime.queued_task_ids, vec![1]);
        assert_eq!(runtime.resource_summary.total_tasks_created, 1);
        assert_eq!(runtime.resource_summary.active_tasks, 1);
        assert_eq!(runtime.resource_summary.queued_tasks, 1);
    }

    #[test]
    fn test_mark_task_running() {
        let mut runtime = new_async_runtime(None);
        let params = empty_parameter_bag();
        let task = submit_task(&mut runtime, "test".to_string(), params, TaskPriority::Normal, None);

        mark_task_running(&mut runtime, task.id);

        let active_task = &runtime.active_tasks[0];
        assert!(matches!(active_task.state, TaskState::Running { .. }));
    }

    #[test]
    fn test_complete_task_success() {
        let mut runtime = new_async_runtime(None);
        let params = empty_parameter_bag();
        let task = submit_task(&mut runtime, "test".to_string(), params, TaskPriority::Normal, None);
        mark_task_running(&mut runtime, task.id);

        let result = SystemExecutionResult::Success {
            return_value: Some("done".to_string()),
            resources_modified: vec!["res1".to_string()],
            tables_modified: vec!["table1".to_string()],
        };

        complete_task_success(&mut runtime, task.id, result.clone());

        assert!(runtime.active_tasks.is_empty());
        assert_eq!(runtime.completed_tasks.len(), 1);
        assert_eq!(runtime.completed_tasks[0].id, task.id);
        assert!(matches!(runtime.completed_tasks[0].state, TaskState::Completed { .. }));
        assert_eq!(runtime.resource_summary.completed_tasks, 1);
        assert_eq!(runtime.resource_summary.active_tasks, 0);
    }

    #[test]
    fn test_suspend_and_resume_task() {
        let mut runtime = new_async_runtime(None);
        let params = empty_parameter_bag();
        let task = submit_task(&mut runtime, "test".to_string(), params, TaskPriority::Normal, None);
        mark_task_running(&mut runtime, task.id);

        let yield_point = YieldPoint::Sleeping {
            duration_ms: 1000,
            started_at_ms: 1000,
        };

        suspend_task(&mut runtime, task.id, yield_point, None);

        let active_task = &runtime.active_tasks[0];
        assert!(matches!(active_task.state, TaskState::Suspended { .. }));

        let resume_result = resume_task(&mut runtime, task.id);
        assert!(resume_result.resumed);
        assert_eq!(resume_result.intermediate_state, None);

        let active_task = &runtime.active_tasks[0];
        assert_eq!(active_task.state, TaskState::Pending);
        assert_eq!(runtime.queued_task_ids, vec![task.id]);
    }

    #[test]
    fn test_cancel_task() {
        let mut runtime = new_async_runtime(None);
        let params = empty_parameter_bag();
        let task = submit_task(&mut runtime, "test".to_string(), params, TaskPriority::Normal, None);

        cancel_task(&mut runtime, task.id);

        assert!(runtime.active_tasks.is_empty());
        assert_eq!(runtime.completed_tasks.len(), 1);
        assert!(matches!(runtime.completed_tasks[0].state, TaskState::Cancelled { .. }));
        assert_eq!(runtime.resource_summary.cancelled_tasks, 1);
    }

    #[test]
    fn test_fail_task() {
        let mut runtime = new_async_runtime(None);
        let params = empty_parameter_bag();
        let task = submit_task(&mut runtime, "test".to_string(), params, TaskPriority::Normal, None);

        let error = AsyncExecutionError::SystemError {
            system: "test".to_string(),
            message: "test error".to_string(),
        };

        fail_task(&mut runtime, task.id, error.clone());

        assert!(runtime.active_tasks.is_empty());
        assert_eq!(runtime.completed_tasks.len(), 1);
        assert!(matches!(runtime.completed_tasks[0].state, TaskState::Failed { .. }));
        assert_eq!(runtime.resource_summary.failed_tasks, 1);
    }

    #[test]
    fn test_yield_task() {
        let mut runtime = new_async_runtime(None);
        let params = empty_parameter_bag();
        let task = submit_task(&mut runtime, "test".to_string(), params, TaskPriority::Normal, None);
        mark_task_running(&mut runtime, task.id);

        let intermediate_state = vec![ParameterValue {
            name: "state".to_string(),
            payload: "partial".to_string(),
        }];

        let yield_point = YieldPoint::Sleeping {
            duration_ms: 1000,
            started_at_ms: 1000,
        };

        let partial_result = SystemExecutionResult::Partial {
            intermediate_state: intermediate_state.clone(),
            next_yield_point: yield_point.clone(),
        };

        yield_task(&mut runtime, task.id, partial_result);

        let active_task = &runtime.active_tasks[0];
        if let TaskState::Suspended { yield_point: yp, intermediate_state: istate, .. } = &active_task.state {
            assert_eq!(yp, &yield_point);
            assert_eq!(istate, &Some(intermediate_state));
        } else {
            panic!("Task should be suspended");
        }
    }

    #[test]
    fn test_apply_task_outcome() {
        let mut runtime = new_async_runtime(None);
        let params = empty_parameter_bag();
        let task = submit_task(&mut runtime, "test".to_string(), params, TaskPriority::Normal, None);
        mark_task_running(&mut runtime, task.id);

        let result = SystemExecutionResult::Success {
            return_value: Some("done".to_string()),
            resources_modified: vec![],
            tables_modified: vec![],
        };

        let outcome = TaskOutcome::Completed { result: result.clone() };
        apply_task_outcome(&mut runtime, task.id, outcome);

        assert!(runtime.active_tasks.is_empty());
        assert_eq!(runtime.completed_tasks.len(), 1);
    }

    #[test]
    fn test_begin_next_task() {
        let mut runtime = new_async_runtime(None);
        let params = empty_parameter_bag();
        let task = submit_task(&mut runtime, "test".to_string(), params, TaskPriority::Normal, None);

        let dispatch_context = begin_next_task(&mut runtime);

        assert!(dispatch_context.is_some());
        let ctx = dispatch_context.unwrap();
        assert_eq!(ctx.task.id, task.id);
        assert_eq!(ctx.intermediate_state, None);

        let active_task = &runtime.active_tasks[0];
        assert!(matches!(active_task.state, TaskState::Running { .. }));
    }

    #[test]
    fn test_take_completed_task() {
        let mut runtime = new_async_runtime(None);
        let params = empty_parameter_bag();
        let task = submit_task(&mut runtime, "test".to_string(), params, TaskPriority::Normal, None);
        mark_task_running(&mut runtime, task.id);

        let result = SystemExecutionResult::Success {
            return_value: None,
            resources_modified: vec![],
            tables_modified: vec![],
        };

        complete_task_success(&mut runtime, task.id, result);

        let completed = take_completed_task(&mut runtime);
        assert!(completed.is_some());
        assert_eq!(completed.unwrap().id, task.id);

        assert!(runtime.completed_tasks.is_empty());
    }

    #[test]
    fn test_task_intermediate_state() {
        let mut runtime = new_async_runtime(None);

        let values = vec![ParameterValue {
            name: "test".to_string(),
            payload: "data".to_string(),
        }];

        store_task_intermediate_state(&mut runtime, 1, values.clone());

        let retrieved = take_task_intermediate_state(&mut runtime, 1);
        assert_eq!(retrieved, Some(values));

        let empty = take_task_intermediate_state(&mut runtime, 1);
        assert_eq!(empty, None);
    }

    #[test]
    fn test_resource_access_conflict() {
        assert!(!resource_access_conflict(&ResourceAccess::Immutable, &ResourceAccess::Immutable));
        assert!(resource_access_conflict(&ResourceAccess::Immutable, &ResourceAccess::Mutable));
        assert!(resource_access_conflict(&ResourceAccess::Immutable, &ResourceAccess::Owned));
        assert!(resource_access_conflict(&ResourceAccess::Mutable, &ResourceAccess::Immutable));
        assert!(resource_access_conflict(&ResourceAccess::Mutable, &ResourceAccess::Mutable));
        assert!(resource_access_conflict(&ResourceAccess::Owned, &ResourceAccess::Immutable));
    }

    #[test]
    fn test_count_running_tasks() {
        let mut runtime = new_async_runtime(None);
        let params = empty_parameter_bag();

        let task1 = submit_task(&mut runtime, "test".to_string(), params.clone(), TaskPriority::Normal, None);
        let task2 = submit_task(&mut runtime, "test".to_string(), params, TaskPriority::Normal, None);

        mark_task_running(&mut runtime, task1.id);

        assert_eq!(count_running_tasks(&runtime.active_tasks), 1);

        mark_task_running(&mut runtime, task2.id);

        assert_eq!(count_running_tasks(&runtime.active_tasks), 2);
    }

    #[test]
    fn test_min_option_i64() {
        assert_eq!(min_option_i64(None, 5), Some(5));
        assert_eq!(min_option_i64(Some(10), 5), Some(5));
        assert_eq!(min_option_i64(Some(5), 10), Some(5));
    }

    #[test]
    fn test_empty_parameter_bag() {
        let bag = empty_parameter_bag();
        assert!(bag.values.is_empty());
    }

    #[test]
    fn test_empty_runtime_stats() {
        let stats = empty_runtime_stats();
        assert_eq!(stats.total_tasks_created, 0);
        assert_eq!(stats.active_tasks, 0);
        assert_eq!(stats.queued_tasks, 0);
        assert_eq!(stats.completed_tasks, 0);
        assert_eq!(stats.failed_tasks, 0);
        assert_eq!(stats.cancelled_tasks, 0);
        assert_eq!(stats.resource_contentions, 0);
    }
}

// Tests for advanced enum-based error and result types from stdlib/core/

#[cfg(test)]
mod advanced_core_tests {
    use super::*;

    #[test]
    fn test_advanced_std_error_message() {
        let message_err = AdvancedStdError::Message {
            message: "test message".to_string(),
        };
        assert_eq!(advanced_std_error_message(&message_err), "test message");

        let panic_err = AdvancedStdError::Panic {
            message: "panic occurred".to_string(),
            source: Some("some_source".to_string()),
        };
        assert_eq!(advanced_std_error_message(&panic_err), "panic occurred");

        let invalid_arg_err = AdvancedStdError::InvalidArgument {
            parameter: "param1".to_string(),
            message: "invalid value".to_string(),
        };
        assert_eq!(advanced_std_error_message(&invalid_arg_err), "invalid value");

        let unsupported_err = AdvancedStdError::Unsupported {
            feature: "advanced_feature".to_string(),
        };
        assert_eq!(advanced_std_error_message(&unsupported_err), "advanced_feature");
    }

    #[test]
    fn test_advanced_std_ok() {
        let result: AdvancedStdResult<i32> = advanced_std_ok(42);
        match result {
            AdvancedStdResult::Ok { value } => assert_eq!(value, 42),
            AdvancedStdResult::Err { .. } => panic!("Expected Ok variant"),
        }
    }

    #[test]
    fn test_advanced_std_err() {
        let error = AdvancedStdError::Message {
            message: "test error".to_string(),
        };
        let result: AdvancedStdResult<i32> = advanced_std_err(error.clone());
        match result {
            AdvancedStdResult::Ok { .. } => panic!("Expected Err variant"),
            AdvancedStdResult::Err { error: err } => assert_eq!(err, error),
        }
    }

    #[test]
    fn test_advanced_std_error_variants() {
        // Test Message variant
        let message_err = AdvancedStdError::Message {
            message: "simple message".to_string(),
        };
        assert_eq!(advanced_std_error_message(&message_err), "simple message");

        // Test Panic variant with source
        let panic_with_source = AdvancedStdError::Panic {
            message: "panic with source".to_string(),
            source: Some("source_location".to_string()),
        };
        assert_eq!(advanced_std_error_message(&panic_with_source), "panic with source");

        // Test Panic variant without source
        let panic_no_source = AdvancedStdError::Panic {
            message: "panic no source".to_string(),
            source: None,
        };
        assert_eq!(advanced_std_error_message(&panic_no_source), "panic no source");

        // Test InvalidArgument variant
        let invalid_arg = AdvancedStdError::InvalidArgument {
            parameter: "input_param".to_string(),
            message: "must be positive".to_string(),
        };
        assert_eq!(advanced_std_error_message(&invalid_arg), "must be positive");

        // Test Unsupported variant
        let unsupported = AdvancedStdError::Unsupported {
            feature: "experimental_api".to_string(),
        };
        assert_eq!(advanced_std_error_message(&unsupported), "experimental_api");
    }

    #[test]
    fn test_advanced_std_result_pattern_matching() {
        // Test Ok pattern matching
        let ok_result: AdvancedStdResult<String> = advanced_std_ok("success".to_string());
        if let AdvancedStdResult::Ok { value } = ok_result {
            assert_eq!(value, "success");
        } else {
            panic!("Expected Ok result");
        }

        // Test Err pattern matching
        let err_result: AdvancedStdResult<String> = advanced_std_err(AdvancedStdError::Message {
            message: "failure".to_string(),
        });
        if let AdvancedStdResult::Err { error } = err_result {
            assert_eq!(advanced_std_error_message(&error), "failure");
        } else {
            panic!("Expected Err result");
        }
    }

    #[test]
    fn test_advanced_std_result_with_complex_types() {
        // Test with Vec
        let vec_result: AdvancedStdResult<Vec<i32>> = advanced_std_ok(vec![1, 2, 3]);
        match vec_result {
            AdvancedStdResult::Ok { value } => assert_eq!(value, vec![1, 2, 3]),
            AdvancedStdResult::Err { .. } => panic!("Expected Ok result"),
        }

        // Test with custom struct
        #[derive(Debug, Clone, PartialEq)]
        struct TestStruct {
            id: i32,
            name: String,
        }

        let struct_result: AdvancedStdResult<TestStruct> = advanced_std_ok(TestStruct {
            id: 42,
            name: "test".to_string(),
        });
        match struct_result {
            AdvancedStdResult::Ok { value } => {
                assert_eq!(value.id, 42);
                assert_eq!(value.name, "test");
            }
            AdvancedStdResult::Err { .. } => panic!("Expected Ok result"),
        }
    }
}
