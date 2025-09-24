use inkwell::context::Context;
use peano::{codegen, parser, semantic};
use std::fs;
use std::path::Path;
use std::process::Command;

fn clang_available() -> bool {
    Command::new("clang").arg("--version").output().is_ok()
}

#[test]
fn small_integer_widths_behave() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.pn").expect("read prelude");
    let user = r#"
        main :: () => {
            x: u8 := 255u8
            println(x)

            y: u8 := x + 1u8
            println(y)

            a: u16 := 60000u16
            b: u16 := a + 1000u16
            println(b)

            c: i8 := -1i8
            println(c)
        }
    "#;
    let src = format!("{}\n{}", prelude, user);

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_integer_widths.o";
    let exe = "tests/tmp_integer_widths.out";
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
    assert_eq!(stdout, "255\n0\n61000\n-1\n");
}
