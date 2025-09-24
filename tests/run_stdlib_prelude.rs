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
    let prelude = fs::read_to_string("stdlib/prelude.pn").expect("read prelude");
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
    let prelude = fs::read_to_string("stdlib/prelude.pn").expect("read prelude");
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
