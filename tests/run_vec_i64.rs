use peano::{parser, semantic, codegen};
use inkwell::context::Context;
use std::process::Command;
use std::fs;

fn clang_available() -> bool { Command::new("clang").arg("--version").output().is_ok() }

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

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_vec_i64.o"; let exe = "tests/tmp_vec_i64.out";
    // if Path::new(obj).exists() { let _ = fs::remove_file(obj); }
    // if Path::new(exe).exists() { let _ = fs::remove_file(exe); }
    gen.write_object_file(obj).expect("write obj");

    let status = Command::new("clang").args(["-o", exe, obj, "-lc"]).status().expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(out.status.success(), "program failed to run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout, "start\n3\n30\n2\n20\n10\n0\n");
}
