use inkwell::context::Context;
use peano::{codegen, parser, semantic};
use std::fs;
use std::path::Path;
use std::process::Command;

fn clang_available() -> bool {
    Command::new("clang").arg("--version").output().is_ok()
}

fn compile_and_run(src: &str, obj: &str, exe: &str) -> String {
    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    if Path::new(obj).exists() {
        let _ = fs::remove_file(obj);
    }
    if Path::new(exe).exists() {
        let _ = fs::remove_file(exe);
    }

    gen.write_object_file(obj).expect("write obj");

    let status = Command::new("clang")
        .args(["-o", exe, obj])
        .status()
        .expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(out.status.success(), "program failed to run");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn run_generic_identity_e2e() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        id <T> :: (x: T) -> T => { ret x }

        main :: () => {
            println(streq(id("hello"), "hello"))
            println(id(42))
        }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_generic_identity.o", "tests/tmp_generic_identity.out");
    assert_eq!(stdout, ["true\n", "42\n"].concat());
}

#[test]
fn run_identity_e2e() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        id :: (x: i64) -> i64 => { ret x }

        main :: () -> i64 => {
            println(id(42))
            ret 0
        }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_identity.o", "tests/tmp_identity.out");
    assert_eq!(stdout, "42\n");
}

#[test]
fn run_fact_e2e() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        fact :: (n: i64) -> i64 => {
            if n <= 1 { ret 1 } else { ret n * fact(n - 1) }
        }
        main :: () => { println(fact(6)) }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_fact.o", "tests/tmp_fact.out");
    assert_eq!(stdout, "720\n");
}

#[test]
fn run_fib_e2e() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        fib :: (n: i64) -> i64 => {
            if n <= 1 {
                ret n
            } else {
                ret fib(n - 1) + fib(n - 2)
            }
        }
        main :: () => {
            println(fib(10))
        }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_fib.o", "tests/tmp_fib.out");
    assert_eq!(stdout, "55\n");
}

#[test]
fn typed_function_return_and_call() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        add_i32 :: (a:i32, b:i32) -> i64 => { a + b }
        println(add_i32(5, 7))
    "#;

    let stdout = compile_and_run(src, "tests/tmp_fn_typed.o", "tests/tmp_fn_typed.out");
    assert_eq!(stdout, "12\n");
}
