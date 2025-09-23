use peano::{parser, semantic, codegen};
use inkwell::context::Context;
use std::process::Command;
use std::fs;
use std::path::Path;

fn clang_available() -> bool { Command::new("clang").arg("--version").output().is_ok() }

#[test]
fn comparisons_print_bools() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        println(1 < 2)
        println(2 < 1)
        println(3 == 3)
        println(3 == 4)
    "#;

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_cmp.o"; let exe = "tests/tmp_cmp.out";
    if Path::new(obj).exists() { let _ = fs::remove_file(obj); }
    if Path::new(exe).exists() { let _ = fs::remove_file(exe); }
    gen.write_object_file(obj).expect("write obj");

    let status = Command::new("clang").args(["-o", exe, obj]).status().expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(out.status.success(), "program failed to run");
    let stdout = String::from_utf8_lossy(&out.stdout);

    let expected = ["true\n","false\n","true\n","false\n"].concat();
    assert_eq!(stdout, expected, "stdout mismatch: {}", stdout);
}
