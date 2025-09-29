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