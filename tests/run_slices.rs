use inkwell::context::Context;
use std::fs;
use std::path::Path;
use std::process::Command;
use tricti::{codegen, parser, semantic, tri_test_helpers};

const VEC_HELPERS: &str = r#"
vec_get_i64 :: (v: *Vec_i64, idx: i64) -> i64 => do
    if idx < 0: panic("vec index negative")
    if idx >= vec_len_i64(v): panic("vec index out of bounds")
    v.ptr[idx]

vec_is_empty_i64 :: (v: *Vec_i64) -> bool => vec_len_i64(v) == 0
"#;

fn clang_available() -> bool {
    Command::new("clang").arg("--version").output().is_ok()
}

#[test]
fn user_defined_vec_bool_iteration() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = tri_test_helpers::dedent(
        r#"
        v := new_vec_i64()
        vec_push_i64(&v, 1)
        vec_push_i64(&v, 0)
        vec_push_i64(&v, 1)
        vec_push_i64(&v, 1)
        acc := 0
        len := vec_len_i64(&v)
        for i in 0..len:
            if vec_get_i64(&v, i) == 1:
                acc = acc + 1
        println(acc)
        println(vec_len_i64(&v))
    "#,
    );
    let src = format!("{}\n{}\n{}", prelude, VEC_HELPERS, user);

    let stdout = compile_and_run(
        &src,
        "tests/tmp_slice_bool_user.o",
        "tests/tmp_slice_bool_user.out",
    );
    assert_eq!(stdout, "3\n4\n");
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

#[test]
fn iterate_vec_i64() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = tri_test_helpers::dedent(
        r#"
        v := new_vec_i64()
        vec_push_i64(&v, 1)
        vec_push_i64(&v, 2)
        vec_push_i64(&v, 3)
        vec_push_i64(&v, 4)
        vec_push_i64(&v, 5)
        acc := 0
        len := vec_len_i64(&v)
        for i in 0..len:
            acc = acc + vec_get_i64(&v, i)
        println(acc)
    "#,
    );
    let src = format!("{}\n{}\n{}", prelude, VEC_HELPERS, user);
    let stdout = compile_and_run(&src, "tests/tmp_slice.o", "tests/tmp_slice.out");
    assert_eq!(stdout, "15\n");
}

#[test]
fn vec_len_and_empty() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = tri_test_helpers::dedent(
        r#"
        v := new_vec_i64()
        vec_push_i64(&v, 1)
        vec_push_i64(&v, 2)
        vec_push_i64(&v, 3)
        vec_push_i64(&v, 4)
        vec_push_i64(&v, 5)
        println(vec_len_i64(&v))
        println(vec_is_empty_i64(&v))
        e := new_vec_i64()
        println(vec_len_i64(&e))
        println(vec_is_empty_i64(&e))
    "#,
    );
    let src = format!("{}\n{}\n{}", prelude, VEC_HELPERS, user);
    let stdout = compile_and_run(
        &src,
        "tests/tmp_slice_helpers.o",
        "tests/tmp_slice_helpers.out",
    );
    assert_eq!(stdout, "5\nfalse\n0\ntrue\n");
}

#[test]
fn vec_get_reads_elements() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = tri_test_helpers::dedent(
        r#"
        v := new_vec_i64()
        vec_push_i64(&v, 1)
        vec_push_i64(&v, 2)
        vec_push_i64(&v, 3)
        vec_push_i64(&v, 4)
        vec_push_i64(&v, 5)
        println(vec_get_i64(&v, 0))
        println(vec_get_i64(&v, 4))
        println(vec_get_i64(&v, 2))
    "#,
    );
    let src = format!("{}\n{}\n{}", prelude, VEC_HELPERS, user);
    let stdout = compile_and_run(&src, "tests/tmp_slice_get.o", "tests/tmp_slice_get.out");
    assert_eq!(stdout, "1\n5\n3\n");
}

#[test]
fn vec_iteration_in_prelude() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = tri_test_helpers::dedent(
        r#"
        main :: () => do
            v := new_vec_i64()
            vec_push_i64(&v, 1)
            vec_push_i64(&v, 2)
            vec_push_i64(&v, 3)
            vec_push_i64(&v, 4)
            vec_push_i64(&v, 5)
            println(vec_len_i64(&v))
            println(vec_get_i64(&v, 3))
            acc := 0
            len := vec_len_i64(&v)
            for i in 0..len:
                acc = acc + vec_get_i64(&v, i)
            println(acc)
    "#,
    );
    let src = format!("{}\n{}\n{}", prelude, VEC_HELPERS, user);
    let stdout = compile_and_run(&src, "tests/tmp_slice_new.o", "tests/tmp_slice_new.out");
    assert_eq!(stdout, "5\n4\n15\n");
}

#[test]
fn vec_i64_push_pop_len() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = tri_test_helpers::dedent(
        r#"
        tricti_main :: () => do
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
        tricti_main()
    "#,
    );
    let src = format!("{}\n{}", prelude, user);
    let stdout = compile_and_run(&src, "tests/tmp_vec_i64.o", "tests/tmp_vec_i64.out");
    assert_eq!(stdout, "start\n3\n30\n2\n20\n10\n0\n");
}
