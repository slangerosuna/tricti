use inkwell::context::Context;
use peano::{codegen, parser, semantic};
use std::fs;
use std::path::Path;
use std::process::Command;

fn clang_available() -> bool {
    Command::new("clang").arg("--version").output().is_ok()
}

#[test]
fn iterate_slice_i64() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    // Build a vector and then bind a slice over it
    let src = r#"
        slice_i64 := {
            ptr: &i64,
            len: i64,
        }
        v: [i64; 5] := [1,2,3,4,5]
        s: slice_i64 :: { ptr: v, len: 5 }
    acc := 0
    for x in s { acc = acc + x }
        println(acc)
    "#;

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_slice.o";
    let exe = "tests/tmp_slice.out";
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
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout, "15\n");
}
