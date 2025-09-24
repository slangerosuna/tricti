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
fn if_expression_values() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        x := if 1 < 2 { 10 } else { 20 }
        println(x)
        y := if 0 { 1 } else { 2 }
        println(y)
    "#;

    let stdout = compile_and_run(src, "tests/tmp_if.o", "tests/tmp_if.out");
    assert_eq!(stdout, ["10\n", "2\n"].concat());
}

#[test]
fn if_expr_as_return_value() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        foo :: (n: i64) -> i64 => {
            ret if n - (n / 2) * 2 == 0 { 100 } else { 101 }
        }
        main :: () => {
            println(foo(2))
            println(foo(3))
        }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_if_expr_ret.o", "tests/tmp_if_expr_ret.out");
    assert_eq!(stdout, ["100\n", "101\n"].concat());
}

#[test]
fn if_with_early_return_in_then() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        foo :: (n: i64) -> i64 => {
            if n > 0 { ret 7 } else { 3 }
        }
        main :: () => {
            println(foo(1))
            println(foo(0))
        }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_if_early.o", "tests/tmp_if_early.out");
    assert_eq!(stdout, ["7\n", "3\n"].concat());
}

#[test]
fn numeric_for_loop_prints_indices() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        for i in 3 {
            println(i)
        }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_for.o", "tests/tmp_for.out");
    assert_eq!(stdout, ["0\n", "1\n", "2\n"].concat());
}

#[test]
fn for_loop_with_range_start_end() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        for i in 1:4 { println(i) }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_for_range1.o", "tests/tmp_for_range1.out");
    assert_eq!(stdout, ["1\n", "2\n", "3\n"].concat());
}

#[test]
fn for_loop_with_range_and_step() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        for i in 0:6:2 { println(i) }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_for_range2.o", "tests/tmp_for_range2.out");
    assert_eq!(stdout, ["0\n", "2\n", "4\n"].concat());
}

#[test]
fn for_loop_negative_range() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        for i in 3:0:-1 { println(i) }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_for_range_neg.o", "tests/tmp_for_range_neg.out");
    assert_eq!(stdout, ["3\n", "2\n", "1\n"].concat());
}

#[test]
fn for_loop_dynamic_step() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        step := 2
        for i in 0:6:step { println(i) }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_for_range_dyn.o", "tests/tmp_for_range_dyn.out");
    assert_eq!(stdout, ["0\n", "2\n", "4\n"].concat());
}

#[test]
fn for_loop_with_dynamic_negative_step() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        x := 2
        step := -x
        for i in 5:0:step { println(i) }
    "#;

    let stdout =
        compile_and_run(src, "tests/tmp_for_range_dyn_neg.o", "tests/tmp_for_range_dyn_neg.out");
    assert_eq!(stdout, ["5\n", "3\n", "1\n"].concat());
}

#[test]
fn for_loop_with_var_bound() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        n := 4
        for i in n { println(i) }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_for_var.o", "tests/tmp_for_var.out");
    assert_eq!(stdout, ["0\n", "1\n", "2\n", "3\n"].concat());
}

#[test]
fn mutual_recursion_example() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }

    let src = r#"
        is_even :: (n: i64) -> i64 => {
            if n == 0 { ret 1 } else { ret is_odd(n - 1) }
        }
        is_odd :: (n: i64) -> i64 => {
            if n == 0 { ret 0 } else { ret is_even(n - 1) }
        }
        main :: () => {
            println(is_even(10))
            println(is_odd(7))
        }
    "#;

    let stdout = compile_and_run(src, "tests/tmp_mutrec.o", "tests/tmp_mutrec.out");
    assert_eq!(stdout, ["1\n", "1\n"].concat());
}
