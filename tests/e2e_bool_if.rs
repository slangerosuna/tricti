use inkwell::context::Context;
use peano::{codegen, parser, semantic};
use std::fs;
use std::path::Path;

#[test]
fn compiles_bool_and_ifexpr() {
    let src = r#"
        b := 1 < 2
        println(b)
        x := if 1 < 2 { 10 } else { 20 }
        println(x)
    "#;

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_bool_if.o";
    if Path::new(obj).exists() {
        let _ = fs::remove_file(obj);
    }
    gen.write_object_file(obj).expect("write obj");
    assert!(Path::new(obj).exists());
}
