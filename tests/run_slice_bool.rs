use inkwell::context::Context;
use std::path::Path;
use std::fs;
use std::process::Command;
use tricti::{codegen, parser, semantic};

fn clang_available() -> bool {
    Command::new("clang").arg("--version").output().is_ok()
}

#[test]
fn iterate_vec_bool() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let source = r#"
use core::collections

main :: () => do
    v := new [bool]
    v.push(true)
    v.push(false)
    v.push(true)
    v.push(false)
    v.push(false)

    count := 0
    len := v.len()
    for i in 0..len:
        match v.get(i):
            some value => do
                if value:
                    count = count + 1
            none => panic("vec index out of bounds")
    println(count)

main()
    "#;

    let stdout = compile_and_run(source, "tests/tmp_slice_bool.o", "tests/tmp_slice_bool.out");
    assert_eq!(stdout, "2\n");
}

#[test]
fn vec_bool_helpers() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let source = r#"
use core::collections

main :: () => do
    v := new [i64]
    v.push(1)
    v.push(1)
    v.push(0)
    v.push(0)

    println(v.len())
    println(v.is_empty())

    println(v.get(0) == some 1)

    println(v.get(2) == some 1)

main()
    "#;

    let stdout = compile_and_run(
        source,
        "tests/tmp_slice_bool_helpers.o",
        "tests/tmp_slice_bool_helpers.out",
    );
    assert_eq!(stdout, "4\nfalse\ntrue\nfalse\n");
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
