use peano::{parser, semantic, codegen};
use inkwell::context::Context;
use std::process::Command;
use std::fs;
use std::path::Path;

fn clang_available() -> bool { Command::new("clang").arg("--version").output().is_ok() }

#[test]
fn for_loop_with_range_start_end() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        for i in 1:4 { println(i) }
    "#;

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_for_range1.o"; let exe = "tests/tmp_for_range1.out";
    if Path::new(obj).exists() { let _ = fs::remove_file(obj); }
    if Path::new(exe).exists() { let _ = fs::remove_file(exe); }
    gen.write_object_file(obj).expect("write obj");

    let status = Command::new("clang").args(["-o", exe, obj]).status().expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(out.status.success(), "program failed to run");
    let stdout = String::from_utf8_lossy(&out.stdout);

    let expected = ["1\n","2\n","3\n"].concat();
    assert_eq!(stdout, expected, "stdout mismatch: {}", stdout);
}

#[test]
fn for_loop_with_range_and_step() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        for i in 0:6:2 { println(i) }
    "#;

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_for_range2.o"; let exe = "tests/tmp_for_range2.out";
    if Path::new(obj).exists() { let _ = fs::remove_file(obj); }
    if Path::new(exe).exists() { let _ = fs::remove_file(exe); }
    gen.write_object_file(obj).expect("write obj");

    let status = Command::new("clang").args(["-o", exe, obj]).status().expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(out.status.success(), "program failed to run");
    let stdout = String::from_utf8_lossy(&out.stdout);

    let expected = ["0\n","2\n","4\n"].concat();
    assert_eq!(stdout, expected, "stdout mismatch: {}", stdout);
}
