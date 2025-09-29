//! Modern TriCTI Standard Library Types
//!
//! This module contains the modern enum-based error and result types.
//! These provide better type safety and ergonomics compared to the legacy struct-based types.

/// Modern enum-based error type with data directly in variants
#[derive(Debug, Clone, PartialEq)]
pub enum StdError {
    /// Simple message error
    Message { message: String },
    /// Panic with optional source information
    Panic { message: String, source: Option<String> },
    /// Invalid argument error
    InvalidArgument { parameter: String, message: String },
    /// Unsupported feature error
    Unsupported { feature: String },
}

/// Modern enum-based result type
#[derive(Debug, Clone, PartialEq)]
pub enum StdResult<T> {
    /// Success variant containing the value
    Ok { value: T },
    /// Error variant containing the error
    Err { error: StdError },
}

/// Get the error message from a modern StdError
pub fn std_error_message(error: &StdError) -> String {
    match error {
        StdError::Message { message } => message.clone(),
        StdError::Panic { message, .. } => message.clone(),
        StdError::InvalidArgument { message, .. } => message.clone(),
        StdError::Unsupported { feature } => feature.clone(),
    }
}

/// Create a modern StdResult with an Ok value
pub fn std_ok<T>(value: T) -> StdResult<T> {
    StdResult::Ok { value }
}

/// Create a modern StdResult with an Err value
pub fn std_err<T>(error: StdError) -> StdResult<T> {
    StdResult::Err { error }
}