use peano::{parser, semantic, codegen};
use inkwell::context::Context;
use std::process::Command;
use std::fs;
use std::path::Path;

fn clang_available() -> bool { Command::new("clang").arg("--version").output().is_ok() }

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

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_enum_match_destructure.o"; let exe = "tests/tmp_enum_match_destructure.out";
    if Path::new(obj).exists() { let _ = fs::remove_file(obj); }
    if Path::new(exe).exists() { let _ = fs::remove_file(exe); }
    gen.write_object_file(obj).expect("write obj");

    Command::new("clang")
        .args(&[obj, "-o", exe])
        .status().expect("clang link");

    let output = Command::new(exe).output().expect("run exe");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.trim(), "42");
}
