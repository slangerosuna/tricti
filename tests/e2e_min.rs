use inkwell::context::Context;
use peano::{codegen, parser, semantic};
use std::fs;
use std::path::Path;

#[test]
fn compiles_minimal_program_to_object() {
    // Minimal program supported by current simplified parser/semantic/codegen
    let src = r#"
        x := 1
        y := x + 2
        println("hi", y)
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(
        program.statements.len(),
        3,
        "expected three statements, got {:?}",
        program
    );
    match &program.statements[2] {
        peano::ast::Statement::Expression(peano::ast::Expression::Call {
            function,
            type_args,
            arguments,
        }) => {
            match function.as_ref() {
                peano::ast::Expression::Identifier(name) => assert_eq!(name, "println"),
                other => panic!("expected identifier function, got {:?}", other),
            }
            assert_eq!(arguments.len(), 2);
        }
        other => panic!("expected println call as 3rd stmt, got {:?}", other),
    }

    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_min.o";
    if Path::new(obj).exists() {
        let _ = fs::remove_file(obj);
    }
    gen.write_object_file(obj).expect("write obj");
    assert!(Path::new(obj).exists());
}
