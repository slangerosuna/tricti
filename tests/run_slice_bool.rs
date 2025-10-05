use inkwell::context::Context;
use std::fs;
use std::path::Path;
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
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = r#"
        vec_get_i64 :: (v: *Vec_i64, idx: i64) -> i64 => {
                if ~(idx >= 0) { panic("vec index negative") }
                if idx >= vec_len_i64(v) { panic("vec index out of bounds") }
                v.ptr[idx]
        }

        v := new_vec_i64()
        vec_push_i64(&v, 1)
        vec_push_i64(&v, 0)
        vec_push_i64(&v, 1)
        vec_push_i64(&v, 1)
        vec_push_i64(&v, 0)

        count := 0
        len := vec_len_i64(&v)
        for i in 0:len {
            if vec_get_i64(&v, i) == 1 {
                count = count + 1
            }
        }
        println(count)
    "#;
    let src = format!("{}\n{}", prelude, user);

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_slice_bool.o";
    let exe = "tests/tmp_slice_bool.out";
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
    assert_eq!(stdout, "3\n");
}

#[test]
fn vec_bool_helpers() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = r#"
        vec_get_i64 :: (v: *Vec_i64, idx: i64) -> i64 => {
                if ~(idx >= 0) { panic("vec index negative") }
                if idx >= vec_len_i64(v) { panic("vec index out of bounds") }
                v.ptr[idx]
        }

        vec_is_empty_i64 :: (v: *Vec_i64) -> bool => {
                vec_len_i64(v) == 0
        }

        v := new_vec_i64()
        vec_push_i64(&v, 1)
        vec_push_i64(&v, 1)
        vec_push_i64(&v, 0)
        vec_push_i64(&v, 0)
        println(vec_len_i64(&v))
        println(vec_is_empty_i64(&v))
        println(vec_get_i64(&v, 0) == 1)
        println(vec_get_i64(&v, 2) == 1)
    "#;
    let src = format!("{}\n{}", prelude, user);

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_slice_bool_helpers.o";
    let exe = "tests/tmp_slice_bool_helpers.out";
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
    assert_eq!(stdout, "4\nfalse\ntrue\nfalse\n");
}
