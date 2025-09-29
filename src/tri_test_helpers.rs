/// Testing utilities for TriCTI code.
/// This module provides macros and functions to simplify writing tests that compile and run TriCTI code,
/// asserting on stdout output. It encapsulates the common pattern from run_stdlib_prelude.rs.

use inkwell::context::Context;
use crate::{codegen, parser, semantic};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Compiles and runs TriCTI source code, returning the stdout as a String.
/// Panics on compilation or runtime errors.
pub fn compile_and_run_tri(src: &str, obj_path: &str, exe_path: &str) -> String {
    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis failed");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen context failed");
    gen.generate_program(&program).expect("codegen failed");

    // Clean up old files
    if Path::new(obj_path).exists() {
        fs::remove_file(obj_path).expect("failed to remove old obj file");
    }
    if Path::new(exe_path).exists() {
        fs::remove_file(exe_path).expect("failed to remove old exe file");
    }

    gen.write_object_file(obj_path).expect("write obj failed");

    let link_status = Command::new("clang")
        .args(["-o", exe_path, obj_path])
        .status()
        .expect("linking failed");
    assert!(link_status.success(), "linking failed");

    let run_output = Command::new(exe_path).output().expect("running failed");
    assert!(run_output.status.success(), "program failed to run");
    String::from_utf8_lossy(&run_output.stdout).to_string()
}

/// Macro to define a test that compiles and runs TriCTI code, asserting stdout equals expected.
/// Usage: tri_test!(test_name, tri_code, expected_stdout);
/// The tri_code should include the prelude if needed.
/// Generates unique temp file paths to avoid conflicts.
#[macro_export]
macro_rules! tri_test {
    ($name:ident, $tri_code:expr, $expected:expr) => {
        #[test]
        fn $name() {
            if !peano::tri_test_helpers::clang_available() {
                eprintln!("clang not found; skipping test {}", stringify!($name));
                return;
            }
            let obj_path = format!("tests/tmp_{}.o", stringify!($name));
            let exe_path = format!("tests/tmp_{}.out", stringify!($name));
            let stdout = peano::tri_test_helpers::compile_and_run_tri($tri_code, &obj_path, &exe_path);
            assert_eq!(stdout, $expected);
        }
    };
}

/// Helper function to check if clang is available.
pub fn clang_available() -> bool {
    Command::new("clang").arg("--version").output().is_ok()
}

/// Macro for asserting that TriCTI code panics (exits with non-zero status).
/// Usage: tri_test_panic!(test_name, tri_code);
#[macro_export]
macro_rules! tri_test_panic {
    ($name:ident, $tri_code:expr) => {
        #[test]
        fn $name() {
            if !peano::tri_test_helpers::clang_available() {
                eprintln!("clang not found; skipping test {}", stringify!($name));
                return;
            }
            let obj_path = format!("tests/tmp_{}.o", stringify!($name));
            let exe_path = format!("tests/tmp_{}.out", stringify!($name));
            let program = peano::parser::parse($tri_code.to_string());
            let sem = peano::semantic::analyze_program(&program).expect("semantic analysis");
            let context = inkwell::context::Context::create();
            let mut gen = peano::codegen::CodeGenerator::new(&context, sem).expect("codegen");
            gen.generate_program(&program).expect("codegen");

            if std::path::Path::new(&obj_path).exists() {
                std::fs::remove_file(&obj_path).unwrap();
            }
            if std::path::Path::new(&exe_path).exists() {
                std::fs::remove_file(&exe_path).unwrap();
            }

            gen.write_object_file(&obj_path).expect("write obj");
            let link_status = std::process::Command::new("clang")
                .args(["-o", &exe_path, &obj_path])
                .status()
                .expect("link");
            assert!(link_status.success(), "link failed");

            let run_output = std::process::Command::new(&exe_path).output().expect("run");
            assert!(!run_output.status.success(), "expected panic but program succeeded");
        }
    };
}

/// Macro for testing TriCTI code with prelude included automatically.
/// Usage: tri_test_with_prelude!(test_name, user_code, expected_stdout);
/// Loads stdlib/prelude.tri and appends user_code.
#[macro_export]
macro_rules! tri_test_with_prelude {
    ($name:ident, $user_code:expr, $expected:expr) => {
        #[test]
        fn $name() {
            if !peano::tri_test_helpers::clang_available() {
                eprintln!("clang not found; skipping test {}", stringify!($name));
                return;
            }
            let prelude = std::fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
            let full_code = format!("{}\n{}", prelude, $user_code);
            let obj_path = format!("tests/tmp_{}.o", stringify!($name));
            let exe_path = format!("tests/tmp_{}.out", stringify!($name));
            let stdout = peano::tri_test_helpers::compile_and_run_tri(&full_code, &obj_path, &exe_path);
            assert_eq!(stdout, $expected);
        }
    };
}