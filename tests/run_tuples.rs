use inkwell::context::Context;
use peano::{codegen, parser, semantic};
use std::fs;
use std::path::Path;
use std::process::Command;

fn clang_available() -> bool { Command::new("clang").arg("--version").output().is_ok() }

fn compile_and_run(src: &str, obj: &str, exe: &str) -> String {
    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    if Path::new(obj).exists() { let _ = fs::remove_file(obj); }
    if Path::new(exe).exists() { let _ = fs::remove_file(exe); }
    gen.write_object_file(obj).expect("write obj");

    let status = Command::new("clang").args(["-o", exe, obj]).status().expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(out.status.success(), "program failed to run");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn tuple_literal_and_match_runs() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        main :: () => {
            pair := (40, 2)
            println(match pair { (a, b) => a, _ => -1 })
            println(match pair { (a, b) => b, _ => -1 })
            println(match pair { (a, b) => a + b, _ => -1 })
        }
    "#;
    let stdout = compile_and_run(src, "tests/tmp_tuple_basic.o", "tests/tmp_tuple_basic.out");
    assert_eq!(stdout, "40\n2\n42\n");
}
