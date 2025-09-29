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
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn stdlib_prelude_id_and_print_i64() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = r#"
        main :: () => {
            print(123)
            println(id(42))
        }
    "#;
    let src = format!("{}\n{}", prelude, user);
    let stdout = compile_and_run(
        &src,
        "tests/tmp_stdlib_prelude1.o",
        "tests/tmp_stdlib_prelude1.out",
    );
    assert_eq!(stdout, "123\n42\n");
}

#[test]
fn stdlib_prelude_len_and_streq() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = r#"
        main :: () => {
            println(len("hé"))
            println(streq("a", "a"))
        }
    "#;
    let src = format!("{}\n{}", prelude, user);
    let stdout = compile_and_run(
        &src,
        "tests/tmp_stdlib_prelude2.o",
        "tests/tmp_stdlib_prelude2.out",
    );
    assert_eq!(stdout, "3\ntrue\n");
}

#[test]
fn stdlib_prelude_math_and_array_helpers() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = r#"
        main :: () => {
            println(clamp_i64(-5, 0, 10))
            println(sign_i64(-42))
            println(sign_i64(0))
            println(sign_i64(11))
            println(is_even_i64(12))
            println(is_odd_i64(13))

            data := [1i64, 2i64, 3i64]
            mapped := array_map(data, (x) => x * 2)
            println(mapped[0])
            println(mapped[1])
            println(mapped[2])

            println(array_all(mapped, (x) => x >= 2))
            println(array_any(mapped, (x) => x > 4))
            println(array_fold(mapped, 0i64, (acc, x) => acc + x))
        }
    "#;
    let src = format!("{}\n{}", prelude, user);
    let stdout = compile_and_run(
        &src,
        "tests/tmp_stdlib_prelude_math.o",
        "tests/tmp_stdlib_prelude_math.out",
    );
    assert_eq!(stdout, "0\n-1\n0\n1\ntrue\ntrue\n2\n4\n6\ntrue\ntrue\n12\n");
}
