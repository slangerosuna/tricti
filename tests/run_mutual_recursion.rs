use peano::{parser, semantic, codegen};
use inkwell::context::Context;
use std::process::Command;
use std::fs;
use std::path::Path;

fn clang_available() -> bool { Command::new("clang").arg("--version").output().is_ok() }

#[test]
fn mutual_recursion_even_odd() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
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

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_mutrec.o"; let exe = "tests/tmp_mutrec.out";
    if Path::new(obj).exists() { let _ = fs::remove_file(obj); }
    if Path::new(exe).exists() { let _ = fs::remove_file(exe); }
    gen.write_object_file(obj).expect("write obj");

    let status = Command::new("clang").args(["-o", exe, obj]).status().expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(out.status.success(), "program failed to run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout, ["1\n", "1\n"].concat());
}
