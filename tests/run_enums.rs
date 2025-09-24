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
fn enum_as_i64_tag_basics() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        Color :: enum { Red, Green, Blue }
        main :: () => {
            c: i64 := Color::Blue
            println(c)
        }
    "#;
    let stdout = compile_and_run(src, "tests/tmp_enum_basic.o", "tests/tmp_enum_basic.out");
    assert_eq!(stdout, "2\n");
}

#[test]
fn enum_variant_static_path_lowering() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        Color :: enum { Red, Green, Blue }
        main :: () => {
            c: i64 := Color::Green
            println(c)
        }
    "#;
    let stdout = compile_and_run(src, "tests/tmp_enum_variant.o", "tests/tmp_enum_variant.out");
    assert_eq!(stdout, "1\n");
}

#[test]
fn enum_match_basic() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        Color :: enum { Red, Green, Blue }
        main :: () => {
            c := Color::Green
            v := match c { Color::Red => 10, Color::Green => 20, Color::Blue => 30 }
            println(v)
        }
    "#;
    let stdout = compile_and_run(src, "tests/tmp_enum_match.o", "tests/tmp_enum_match.out");
    assert_eq!(stdout, "20\n");
}

#[test]
fn enum_match_destructure() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        Color :: enum { Red, Green: i64, Blue }
        main :: () => {
            c := Color::Green(42)
            v := match c { Color::Red => -1, Color::Green(x) => x, Color::Blue => -2 }
            println(v)
        }
    "#;
    let stdout = compile_and_run(
        src,
        "tests/tmp_enum_match_destructure.o",
        "tests/tmp_enum_match_destructure.out",
    );
    assert_eq!(stdout.trim(), "42");
}

#[test]
fn enum_match_wildcard_default() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        Color :: enum { Red, Green, Blue }
        main :: () => {
            c: Color := Color::Blue
            v := match c { Color::Red => 10, _ => 99 }
            println(v)
        }
    "#;
    let stdout = compile_and_run(src, "tests/tmp_enum_match_wc.o", "tests/tmp_enum_match_wc.out");
    assert_eq!(stdout, "99\n");
}

#[test]
fn enum_payload_basic() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        Color :: enum { Red, Green: i64, Blue }
        main :: () => {
            c := Color::Green(42)
            v := match c { Color::Red => -1, Color::Green => 100, Color::Blue => -2 }
            println(v)
        }
    "#;
    let stdout = compile_and_run(src, "tests/tmp_enum_payload.o", "tests/tmp_enum_payload.out");
    assert_eq!(stdout, "100\n");
}
