use inkwell::context::Context;
use tricti::{codegen, parser, semantic, tri_test_helpers};

#[test]
fn skip_stdlib_regressions_compile() {
    let source = tri_test_helpers::dedent(
        r#"
        Point :: struct
            x: i64,
            y: i64,

        DependencySource :: enum
            Registry,
            Git :: struct
                reference: String,
            Path :: struct
                relative: String,

        is_git_origin :: (source: DependencySource) -> bool => match source:
            DependencySource::Git :: struct { reference } => String::equals(&reference, &String::from_cstr("origin" as *u8))
            _ => false

    extract_x :: (point: &Point) -> i64 => do
            ptr := &point.x
            ret *ptr

        main :: () -> bool => do
            src := DependencySource::Git { reference: String::from_cstr("origin" as *u8) }
            pt := Point { x: 7, y: 11 }
            value := extract_x(&pt)
            confirm := String::equals(&String::from_cstr("done" as *u8), &String::from_cstr("done" as *u8))
            ret is_git_origin(src) and confirm and value == 7
        "#,
    );

    let program = parser::parse(source);
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen context");
    gen.enable_runtime_mode(true);
    gen.generate_program(&program).expect("codegen");
}
