use inkwell::context::Context;
use peano::codegen::CodeGenerator;
use peano::parser;
use peano::semantic::analyze_program;
use std::process::Command;

#[test]
fn struct_field_store_and_method_call_e2e() {
    let src = r#"
        point :: { x: i64, y: i64 }
        impl point {
            noop :: (self: &mut point) -> i64 => { 123 }
        }
        main :: () -> i64 => {
            p: point := { x: 2, y: 3 }
            p.x = 7
            println(p.x)
            println(p.noop())
            0
        }
    "#
    .to_string();

    let program = parser::parse(src);
    let sema = analyze_program(&program).expect("sema");

    let context = Context::create();
    let mut gen = CodeGenerator::new(&context, sema).expect("cg");
    gen.generate_program(&program).expect("gen");
    gen.print_ir();

    let obj = "/tmp/bplang_struct_fields_method_e2e.o";
    gen.write_object_file(obj).expect("obj");

    let bin = "/tmp/bplang_struct_fields_method_e2e";
    let output = Command::new("clang")
        .args([obj, "-o", bin])
        .output()
        .expect("clang link");
    assert!(output.status.success(), "clang failed: {:?}", output);

    let run = Command::new(bin).output().expect("run");
    assert!(run.status.success());
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains("7\n"));
    assert!(stdout.contains("123\n"));
}
