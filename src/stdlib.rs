//! TriCTI Standard Library
//!
//! This module provides the standard library functions and types for TriCTI.
//! It includes both legacy struct-based types and modern enum-based types.

pub mod legacy;
pub mod modern;

// Re-export commonly used types and functions
pub use legacy::*;
pub use modern::*;