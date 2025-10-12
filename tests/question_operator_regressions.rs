use inkwell::context::Context;
use tricti::{codegen, parser, semantic, tri_test_helpers};

fn compile_source(source: &str) {
    let program = parser::parse(source.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis should succeed");
    let context = Context::create();
    let mut generator = codegen::CodeGenerator::new(&context, sem).expect("codegen context");
    generator.enable_runtime_mode(true);
    generator
        .generate_program(&program)
        .expect("code generation should succeed");
}

#[test]
fn question_operator_allows_temp_variable_unwrap() {
    let source = tri_test_helpers::dedent(
        r#"
        make_option :: () -> ?i64 => some 41

        use_question :: () -> ?i64 => do
            tmp := make_option()
            value := tmp?
            ret some (value + 1)

        main :: () -> ?i64 => use_question()
        "#,
    );

    compile_source(&source);
}

#[test]
fn question_operator_handles_return_value_chaining() {
    let source = tri_test_helpers::dedent(
        r#"
        fetch_id :: () -> ?i64 => some 7

        compute :: () -> ?i64 => do
            id := fetch_id()?
            ret some (id * 2)

        run :: () -> ?i64 => do
            value := compute()?
            ret some (value + 3)
        "#,
    );

    compile_source(&source);
}
