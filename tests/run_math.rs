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
fn comparisons_print_bools() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        println(1 < 2)
        println(2 < 1)
        println(3 == 3)
        println(3 == 4)
    "#;
    let stdout = compile_and_run(src, "tests/tmp_cmp.o", "tests/tmp_cmp.out");
    assert_eq!(stdout, ["true\n", "false\n", "true\n", "false\n"].concat());
}

#[test]
fn stdlib_abs_min_max_i64() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let prelude = fs::read_to_string("stdlib/prelude.pn").expect("read prelude");
    let user = r#"
        main :: () => {
            println(abs_i64(-5))
            println(min_i64(3, 9))
            println(max_i64(3, 9))
        }
    "#;
    let src = format!("{}\n{}", prelude, user);
    let stdout = compile_and_run(&src, "tests/tmp_absminmax.o", "tests/tmp_absminmax.out");
    assert_eq!(stdout, "5\n3\n9\n");
}

#[test]
fn stdlib_rem_i64() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let prelude = fs::read_to_string("stdlib/prelude.pn").expect("read prelude");
    let user = r#"
        main :: () => {
            println(rem_i64(10, 3))
            println(rem_i64(11, 3))
            println(rem_i64(3, 3))
        }
    "#;
    let src = format!("{}\n{}", prelude, user);
    let stdout = compile_and_run(&src, "tests/tmp_rem.o", "tests/tmp_rem.out");
    assert_eq!(stdout, "1\n2\n0\n");
}

#[test]
fn small_integer_widths_behave() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
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
    let stdout = compile_and_run(&src, "tests/tmp_integer_widths.o", "tests/tmp_integer_widths.out");
    assert_eq!(stdout, "255\n0\n61000\n-1\n");
}
