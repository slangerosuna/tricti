use inkwell::context::Context;
use peano::codegen::CodeGenerator;
use peano::parser;
use peano::semantic::analyze_program;
use std::process::Command;

#[test]
fn trait_static_dispatch_e2e() {
    let src = r#"
        point :: { x: i64 }
    ValLike :: trait { val: (&point) -> i64, }
        impl point { val :: (self: &point) -> i64 => { 111 } }
    impl ValLike for point { val :: (self: &point) -> i64 => { 222 } }
        main :: () -> i64 => {
            p: point := { x: 5 }
            println(p.val())
            0
        }
    "#
    .to_string();

    let program = parser::parse(src);
    println!("AST: {:#?}", program);
    let sema = analyze_program(&program).expect("sema");

    let context = Context::create();
    let mut gen = CodeGenerator::new(&context, sema).expect("cg");
    gen.generate_program(&program).expect("gen");
    gen.print_ir();

    let obj = "/tmp/bplang_trait_static_dispatch_e2e.o";
    gen.write_object_file(obj).expect("obj");

    let bin = "/tmp/bplang_trait_static_dispatch_e2e";
    let output = Command::new("clang")
        .args([obj, "-o", bin])
        .output()
        .expect("clang link");
    assert!(output.status.success(), "clang failed: {:?}", output);

    let run = Command::new(bin).output().expect("run");
    assert!(run.status.success());
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains("111\n"), "stdout = {}", stdout);
}
