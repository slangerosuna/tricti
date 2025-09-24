use inkwell::context::Context;
use peano::{codegen, parser, semantic};
use std::fs;
use std::path::Path;
use std::process::Command;

fn clang_available() -> bool { Command::new("clang").arg("--version").output().is_ok() }

#[test]
fn user_defined_slice_bool_iteration() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        SliceBool := {
            ptr: &bool,
            len: i64,
        }

        v: [bool; 4] := [true, false, true, true]
        s: SliceBool :: { ptr: v, len: 4 }
        acc := 0
        for b in s {
            if b { acc = acc + 1 }
        }
        println(acc)
        println(s.len)
    "#;

    let stdout = compile_and_run(src, "tests/tmp_slice_bool_user.o", "tests/tmp_slice_bool_user.out");
    assert_eq!(stdout, "3\n4\n");
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

    let status = Command::new("clang").args(["-o", exe, obj]).status().expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(out.status.success(), "program failed to run");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn iterate_slice_i64() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
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
    let stdout = compile_and_run(src, "tests/tmp_slice.o", "tests/tmp_slice.out");
    assert_eq!(stdout, "15\n");
}

#[test]
fn slice_len_and_empty() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        slice_i64 := {
            ptr: &i64,
            len: i64,
        }
        v: [i64; 5] := [1,2,3,4,5]
        s: slice_i64 :: { ptr: v, len: 5 }
        println(slice_len(s))
        println(slice_is_empty(s))
        e: slice_i64 :: { ptr: v, len: 0 }
        println(slice_len(e))
        println(slice_is_empty(e))
    "#;
    let stdout = compile_and_run(src, "tests/tmp_slice_helpers.o", "tests/tmp_slice_helpers.out");
    assert_eq!(stdout, "5\nfalse\n0\ntrue\n");
}

#[test]
fn slice_get_reads_elements() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        slice_i64 := {
            ptr: &i64,
            len: i64,
        }
        v: [i64; 5] := [1,2,3,4,5]
        s: slice_i64 :: { ptr: v, len: 5 }
        println(slice_get(s, 0))
        println(slice_get(s, 4))
        println(slice_get(s, 2))
    "#;
    let stdout = compile_and_run(src, "tests/tmp_slice_get.o", "tests/tmp_slice_get.out");
    assert_eq!(stdout, "1\n5\n3\n");
}

#[test]
fn slice_i64_constructor_in_prelude() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let prelude = fs::read_to_string("stdlib/prelude.pn").expect("read prelude");
    let user = r#"
        slice_i64 := {
            ptr: &i64,
            len: i64,
        }
        main :: () => {
            v: [i64; 5] := [1,2,3,4,5]
            s: slice_i64 := slice_i64_from(v, 5)
            println(slice_len(s))
            println(slice_get(s, 3))
            acc := 0
            for x in s { acc = acc + x }
            println(acc)
        }
    "#;
    let src = format!("{}\n{}", prelude, user);
    let stdout = compile_and_run(&src, "tests/tmp_slice_new.o", "tests/tmp_slice_new.out");
    assert_eq!(stdout, "5\n4\n15\n");
}

#[test]
fn vec_i64_push_pop_len() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let prelude = fs::read_to_string("stdlib/prelude.pn").expect("read prelude");
    let user = r#"
        peano_main :: () => {
            println("start")
            v := new_vec_i64()
            vec_push_i64(&v, 10)
            vec_push_i64(&v, 20)
            vec_push_i64(&v, 30)
            println(vec_len_i64(&v))
            println(vec_pop_i64(&v))
            println(vec_len_i64(&v))
            println(vec_pop_i64(&v))
            println(vec_pop_i64(&v))
            println(vec_len_i64(&v))
        }
        peano_main()
    "#;
    let src = format!("{}\n{}", prelude, user);
    let stdout = compile_and_run(&src, "tests/tmp_vec_i64.o", "tests/tmp_vec_i64.out");
    assert_eq!(stdout, "start\n3\n30\n2\n20\n10\n0\n");
}
