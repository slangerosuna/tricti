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
fn generic_slice_i64() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        Slice<T> :: struct {
            ptr: *T,
            len: usize
        }

        slice_from_vec<T> :: (vec: &Vec<T>) -> Slice<T> => {
            Slice { ptr: vec.data.as_ptr(), len: vec.len }
        }

        slice_iter_next<T> :: (iter: &mut SliceIter<T>) -> ?T => {
            if iter.index >= iter.slice.len {
                ret none
            }
            value := unsafe_load(iter.slice.ptr, iter.index)
            iter.index += 1
            ret some(value)
        }

        v: Vec<i64> := [1,2,3,4,5]
        s: Slice<i64> := slice_from_vec(&v)
        acc := 0
        for x in s { acc = acc + x }
        println(acc)
    "#;
    let stdout = compile_and_run(src, "tests/tmp_generic_slice.o", "tests/tmp_generic_slice.out");
    assert_eq!(stdout, "15\n");
}
