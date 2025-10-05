// Rust tests for migrated TriCTI logic from stdlib/prelude.tri
// These tests ensure the legacy helper semantics remain available while the
// modern stdlib evolves toward !-based error handling.

use tricti::stdlib::modern::{
    std_err as advanced_std_err,
    std_error_message as advanced_std_error_message,
    std_ok as advanced_std_ok,
    StdError as AdvancedStdError,
    StdResult as AdvancedStdResult,
};

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
    if result.is_ok {
        result
            .value
            .as_ref()
            .cloned()
            .expect("std_ok results always carry a value")
    } else {
        panic!("std_result_unwrap called on error result");
    }
}

pub fn std_result_error<T>(result: &StdResult<T>) -> Option<StdError> {
    result.error.clone()
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
    items
        .iter()
        .filter(|item| !predicate(item))
        .cloned()
        .collect()
}

pub fn array_filter<T: Clone, F>(items: &[T], predicate: F) -> Vec<T>
where
    F: Fn(&T) -> bool,
{
    items
        .iter()
        .filter(|item| predicate(item))
        .cloned()
        .collect()
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
    if value > 0 {
        1
    } else if value < 0 {
        -1
    } else {
        0
    }
}

pub fn array_fold<T, F>(items: &[T], initial: T, reducer: F) -> T
where
    F: Fn(T, &T) -> T,
{
    let mut acc = initial;
    for item in items {
        acc = reducer(acc, item);
    }
    acc
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
            default_timeout_ms: 5_000,
            enable_tracing: false,
        },
    }
}

pub fn vec_len<T>(v: &[T]) -> usize {
    v.len()
}

pub fn vec_is_empty<T>(v: &[T]) -> bool {
    v.is_empty()
}

pub fn vec_get<T: Clone>(v: &[T], idx: usize) -> Option<T> {
    v.get(idx).cloned()
}

pub fn vec_get_bool(v: &[bool], idx: usize) -> Option<bool> {
    vec_get(v, idx)
}

pub fn vec_is_empty_bool(v: &[bool]) -> bool {
    vec_is_empty(v)
}

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
    use super::{
        advanced_std_err,
        advanced_std_error_message,
        advanced_std_ok,
        AdvancedStdError,
        AdvancedStdResult,
    };

    fn sample_error(message: &str) -> StdError {
        StdError {
            kind: StdErrorKind::Message,
            message: message.to_string(),
            parameter: None,
            feature: None,
            source: None,
        }
    }

    #[test]
    fn std_ok_and_err_roundtrip() {
        let ok = std_ok(5);
        assert!(std_result_is_ok(&ok));
        assert_eq!(std_result_unwrap(&ok), 5);

        let error = sample_error("boom");
        let err_result: StdResult<i32> = std_err(error.clone());
        assert!(!std_result_is_ok(&err_result));
        assert_eq!(std_result_error(&err_result), Some(error));
    }

    #[test]
    fn std_result_unwrap_success() {
        let ok = std_ok(String::from("hello"));
        assert_eq!(std_result_unwrap(&ok), String::from("hello"));
    }

    #[test]
    #[should_panic(expected = "std_result_unwrap called on error result")]
    fn std_result_unwrap_panics_on_err() {
        let err: StdResult<String> = std_err(sample_error("panic"));
        let _ = std_result_unwrap(&err);
    }

    #[test]
    fn std_result_error_returns_clone() {
        let error = sample_error("boom");
        let result: StdResult<i64> = std_err(error.clone());
        let extracted = std_result_error(&result).expect("expected error");
        assert_eq!(extracted, error);
    }

    #[test]
    fn array_helpers_cover_common_cases() {
        let base = vec![1, 2, 3];

        assert_eq!(array_append(&base, 4), vec![1, 2, 3, 4]);
        assert_eq!(array_set_at(&base, 1, 99), vec![1, 99, 3]);
        assert_eq!(array_remove_at(&base, 0), vec![2, 3]);
        assert_eq!(array_remove_where(&base, |value| value % 2 == 0), vec![1, 3]);
        assert_eq!(array_filter(&base, |value| value % 2 == 1), vec![1, 3]);
        assert_eq!(array_find_index(&base, |value| *value == 2), Some(1));
        assert!(array_contains(&base, |value| *value == 3));
        assert_eq!(array_map(&base, |value| value * 2), vec![2, 4, 6]);
        assert!(array_all(&base, |value| *value > 0));
        assert!(array_any(&base, |value| *value == 3));
        assert_eq!(array_fold(&base, 0, |acc, value| acc + value), 6);
    }

    #[test]
    fn config_defaults_match_expectations() {
        let cfg = std_default_config();
        assert!(cfg.runtime.enable_preemption);
        assert_eq!(cfg.runtime.scheduling_quantum_ms, 10);
        assert_eq!(cfg.io.default_timeout_ms, 5_000);
        assert!(!cfg.io.enable_tracing);
    }

    #[test]
    fn vec_helpers_cover_common_cases() {
        let numbers = vec![10, 20, 30];
        assert_eq!(vec_len(&numbers), 3);
        assert!(!vec_is_empty(&numbers));
        assert_eq!(vec_get(&numbers, 1), Some(20));
        assert_eq!(vec_get(&numbers, 99), None);

        let bools = vec![true, false];
        assert_eq!(vec_get_bool(&bools, 0), Some(true));
        assert!(vec_is_empty_bool(&Vec::<bool>::new()));
    }

    #[test]
    fn vec_i64_round_trip() {
        let mut values = new_vec_i64();
        assert_eq!(vec_len_i64(&values), 0);

        vec_push_i64(&mut values, 10);
        vec_push_i64(&mut values, 20);
        assert_eq!(vec_len_i64(&values), 2);

        assert_eq!(vec_pop_i64(&mut values), Some(20));
        assert_eq!(vec_pop_i64(&mut values), Some(10));
        assert_eq!(vec_pop_i64(&mut values), None);
    }

    #[test]
    fn advanced_stdlib_bridge() {
        let ok: AdvancedStdResult<&str> = advanced_std_ok("bridge");
        match ok {
            AdvancedStdResult::Ok { value } => assert_eq!(value, "bridge"),
            AdvancedStdResult::Err { .. } => panic!("expected ok result"),
        }

        let err = AdvancedStdError::Unsupported {
            feature: "async".to_string(),
        };
        let err_result: AdvancedStdResult<()> = advanced_std_err(err.clone());
        match err_result {
            AdvancedStdResult::Err { error } => {
                assert_eq!(
                    advanced_std_error_message(&error),
                    advanced_std_error_message(&err)
                );
            }
            AdvancedStdResult::Ok { .. } => panic!("expected err result"),
        }
    }

    #[test]
    fn std_error_constructors() {
        let with_source = std_error_with_source(
            StdErrorKind::Panic,
            "crash".to_string(),
            Some("engine".to_string()),
        );
        assert_eq!(std_error_kind(&with_source), StdErrorKind::Panic);
        assert_eq!(std_error_message(&with_source), "crash");
        assert_eq!(with_source.source.as_deref(), Some("engine"));

        let invalid = std_error_invalid_argument("param".to_string(), "bad".to_string());
        assert_eq!(invalid.parameter.as_deref(), Some("param"));
        assert_eq!(invalid.message, "bad");

        let unsupported = std_error_unsupported("feature".to_string());
        assert_eq!(unsupported.feature.as_deref(), Some("feature"));
    }

    #[test]
    fn math_helpers_cover_common_cases() {
        assert_eq!(clamp_i64(-5, 0, 10), 0);
        assert_eq!(clamp_i64(5, 0, 10), 5);
        assert_eq!(clamp_i64(15, 0, 10), 10);

        assert_eq!(sign_i64(-4), -1);
        assert_eq!(sign_i64(0), 0);
        assert_eq!(sign_i64(7), 1);

        assert!(is_even_i64(12));
        assert!(!is_even_i64(13));
        assert!(is_odd_i64(13));
        assert!(!is_odd_i64(12));

        assert_eq!(rem_i64(7, 3), 1);
        assert_eq!(abs_i64(-8), 8);
        assert_eq!(min_i64(5, 3), 3);
        assert_eq!(max_i64(5, 3), 5);
        assert_eq!(id(-4), -4);
    }

    #[test]
    fn logging_does_not_panic() {
        log_message(&LogLevel::Info, "info");
        log_info("info helper");
        log_error("error helper");
    }
}
