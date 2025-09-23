use peano::{parser, semantic, codegen};
use inkwell::context::Context;
use std::process::Command;
use std::fs;
use std::path::Path;

fn clang_available() -> bool {
    Command::new("clang").arg("--version").output().is_ok()
}

#[test]
fn if_expression_values() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        x := if 1 < 2 { 10 } else { 20 }
        println(x)
        y := if 0 { 1 } else { 2 }
        println(y)
    "#;

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");
    // Debug: print IR for diagnosis
    gen.print_ir();

    let obj = "tests/tmp_if.o";
    let exe = "tests/tmp_if.out";
    if Path::new(obj).exists() { let _ = fs::remove_file(obj); }
    if Path::new(exe).exists() { let _ = fs::remove_file(exe); }

    gen.write_object_file(obj).expect("write obj");

    let status = Command::new("clang").args(["-o", exe, obj]).status().expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(out.status.success(), "program failed to run");
    let stdout = String::from_utf8_lossy(&out.stdout);

    let expected = [
        "10\n",
        "2\n",
    ].concat();

    assert_eq!(stdout, expected, "stdout mismatch: {}", stdout);
}
