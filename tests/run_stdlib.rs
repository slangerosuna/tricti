use inkwell::context::Context;
use peano::{codegen, parser, semantic};
use std::fs;
use std::path::Path;
use std::process::Command;

fn clang_available() -> bool {
    Command::new("clang").arg("--version").output().is_ok()
}

fn compile_and_run(src: &str, obj: &str, exe: &str) -> String {
    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    if Path::new(obj).exists() { let _ = fs::remove_file(obj); }
    if Path::new(exe).exists() { let _ = fs::remove_file(exe); }

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

#[test]
fn prints_various_types() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        println("hello")
        println(42)
        println(3.14)
        println(true)
        println(false)
    "#;
    let stdout = compile_and_run(src, "tests/tmp_println.o", "tests/tmp_println.out");
    let expected = ["hello\n", "42\n", "3.140000\n", "true\n", "false\n"].concat();
    assert_eq!(stdout, expected);
}

#[test]
fn prints_string_variable_and_utf8() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        s := "hello, world"
        println(s)
        println("héllø – π")
    "#;
    let stdout = compile_and_run(
        src,
        "tests/tmp_println_strings.o",
        "tests/tmp_println_strings.out",
    );
    assert_eq!(stdout, ["hello, world\n", "héllø – π\n"].concat());
}

#[test]
fn mixed_printf_formats() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        println("x", 5, 2.5, true)
    "#;
    let stdout = compile_and_run(src, "tests/tmp_formats.o", "tests/tmp_formats.out");
    assert_eq!(stdout, "x 5 2.500000 true\n");
}

#[test]
fn len_of_string_literal_and_var() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        println(len("abc"))
        s := "hé"
        println(len(s))
    "#;
    let stdout = compile_and_run(src, "tests/tmp_len_str.o", "tests/tmp_len_str.out");
    assert_eq!(stdout, "3\n3\n");
}

#[test]
fn contains_substring_basic() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        main :: () => {
            println(contains("hello", "ell"))
            println(contains("hello", "world"))
            s := "héllo"
            println(contains(s, "hé"))
        }
    "#;
    let stdout = compile_and_run(src, "tests/tmp_contains.o", "tests/tmp_contains.out");
    assert_eq!(stdout, "true\nfalse\ntrue\n");
}

#[test]
fn streq_basic() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        println(streq("a", "a"))
        println(streq("a", "b"))
        s := "hé"
        t := "hé"
        println(streq(s, t))
    "#;
    let stdout = compile_and_run(src, "tests/tmp_streq.o", "tests/tmp_streq.out");
    assert_eq!(stdout, "true\nfalse\ntrue\n");
}

#[test]
fn starts_with_and_ends_with() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        main :: () => {
            println(starts_with("hello", "he"))
            println(starts_with("hello", "hello"))
            println(starts_with("hello", "hello!"))
            println(ends_with("hello", "lo"))
            println(ends_with("héllo", "llo"))
            println(ends_with("héllo", "éllo"))
        }
    "#;
    let stdout = compile_and_run(src, "tests/tmp_prefix_suffix.o", "tests/tmp_prefix_suffix.out");
    assert_eq!(stdout, "true\ntrue\nfalse\ntrue\ntrue\ntrue\n");
}

#[test]
fn find_returns_index_or_minus_one() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        main :: () => {
            println(find("hello", "ell"))
            println(find("hello", "world"))
            s := "héllo"
            println(find(s, "hé"))
        }
    "#;
    let stdout = compile_and_run(src, "tests/tmp_find.o", "tests/tmp_find.out");
    assert_eq!(stdout, "1\n-1\n0\n");
}

#[test]
fn string_helper_edge_cases() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        main :: () => {
            println(contains("", ""))
            println(contains("abc", ""))
            println(starts_with("abc", ""))
            println(ends_with("abc", ""))
            println(ends_with("abc", "abc"))
            println(ends_with("abc", "abcd"))
            println(find("", ""))
            println(find("abc", ""))
            println(find("abc", "c"))
            println(find("abc", "d"))
        }
    "#;
    let stdout = compile_and_run(src, "tests/tmp_string_edges.o", "tests/tmp_string_edges.out");
    assert_eq!(stdout, "true\ntrue\ntrue\ntrue\ntrue\nfalse\n0\n0\n2\n-1\n");
}
