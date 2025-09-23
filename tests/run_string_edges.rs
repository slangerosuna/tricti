use peano::{parser, semantic, codegen};
use inkwell::context::Context;
use std::process::Command;
use std::fs;
use std::path::Path;

fn clang_available() -> bool { Command::new("clang").arg("--version").output().is_ok() }

#[test]
fn string_helper_edge_cases() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        main :: () => {
            println(contains("", ""))      # true (C strstr returns hay on empty needle)
            println(contains("abc", ""))   # true
            println(starts_with("abc", "")) # true
            println(ends_with("abc", ""))  # true
            println(ends_with("abc", "abc")) # true
            println(ends_with("abc", "abcd")) # false
            println(find("", ""))          # 0
            println(find("abc", ""))       # 0
            println(find("abc", "c"))      # 2
            println(find("abc", "d"))      # -1
        }
    "#;

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_string_edges.o";
    let exe = "tests/tmp_string_edges.out";
    if Path::new(obj).exists() { let _ = fs::remove_file(obj); }
    if Path::new(exe).exists() { let _ = fs::remove_file(exe); }

    gen.write_object_file(obj).expect("write obj");

    let status = Command::new("clang").args(["-o", exe, obj]).status().expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(out.status.success(), "program failed to run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout, "true\ntrue\ntrue\ntrue\ntrue\nfalse\n0\n0\n2\n-1\n");
}
