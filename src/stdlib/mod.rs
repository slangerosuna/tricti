//! TriCTI Standard Library
//!
//! This module provides the standard library functions and types for TriCTI.
//! It includes both legacy struct-based types and modern enum-based types.

pub mod legacy;
pub mod modern;
pub mod parallel_vec;

// Re-export commonly used types and functions
pub use legacy::*;
pub use parallel_vec::*;
