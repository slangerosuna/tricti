use inkwell::context::Context;
use peano::codegen::CodeGenerator;
use peano::parser;
use peano::semantic::analyze_program;
use std::process::Command;

#[test]
fn trait_impl_static_dispatch_runs() {
    let src = r#"
        my_iterator :: trait {
            T: type,
            next: ( &mut self ) -> ?T,
        }
        my_struct :: { }
        impl my_iterator for my_struct {
            T :: i32,
            next :: (self: &mut my_struct) -> ?i32 => {
                5
            }
        }
        call_next :: () -> i64 => { 5 }
        main :: () -> i64 => {
            println(call_next())
            0
        }
    "#
    .to_string();

    let program = parser::parse(src);
    let sema = analyze_program(&program).expect("sema");

    let context = Context::create();
    let mut gen = CodeGenerator::new(&context, sema).expect("cg");
    gen.generate_program(&program).expect("gen");

    // try object + clang + run
    let obj = "/tmp/bplang_traits_test.o";
    gen.write_object_file(obj).expect("obj");

    let bin = "/tmp/bplang_traits_test";
    let output = Command::new("clang")
        .args([obj, "-o", bin])
        .output()
        .expect("clang link");
    assert!(output.status.success(), "clang failed: {:?}", output);

    let run = Command::new(bin).output().expect("run");
    assert!(run.status.success());
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains("5\n"));
}
