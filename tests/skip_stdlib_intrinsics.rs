use inkwell::context::Context;
use tricti::{codegen, parser, semantic, tri_test_helpers};

#[test]
fn skip_stdlib_string_intrinsics_compile() {
    let source = tri_test_helpers::dedent(
        r#"
        Point :: struct
            x: i64,
            y: i64,

        main :: () -> bool => do
            label := String::from_cstr("origin" as *u8)
            check := String::equals(&label, &String::from_cstr("origin" as *u8))
            point := Point { x: 7, y: 11 }
            ptr := &point.x
            value := *ptr
            ret check and value == 7
        "#,
    );

    let program = parser::parse(source);
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen context");
    gen.enable_runtime_mode(true);
    gen.generate_program(&program).expect("codegen");
}
