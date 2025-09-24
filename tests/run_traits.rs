use inkwell::context::Context;
use peano::codegen::CodeGenerator;
use peano::parser;
use peano::semantic::{analyze_program, SemanticError};
use std::process::Command;

fn clang_available() -> bool { Command::new("clang").arg("--version").output().is_ok() }

#[test]
fn trait_impl_static_dispatch_runs() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
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
    "#;

    let program = parser::parse(src.to_string());
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

#[test]
fn trait_static_dispatch_e2e() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
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
    "#;

    let program = parser::parse(src.to_string());
    let sema = analyze_program(&program).expect("sema");

    let context = Context::create();
    let mut gen = CodeGenerator::new(&context, sema).expect("cg");
    gen.generate_program(&program).expect("gen");

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

#[test]
fn trait_static_path_call_e2e() {
    if !clang_available() { eprintln!("clang not found; skipping"); return; }
    let src = r#"
        point :: { x: i64 }
        ValLike :: trait { val: (&point) -> i64, }
        impl point { val :: (self: &point) -> i64 => { 111 } }
        impl ValLike for point { val :: (self: &point) -> i64 => { 222 } }
        main :: () -> i64 => {
            p: point := { x: 5 }
            v: i64 := ValLike::val(p)
            println(v)
            0
        }
    "#;

    let program = parser::parse(src.to_string());
    let sema = analyze_program(&program).expect("sema");

    let context = Context::create();
    let mut gen = CodeGenerator::new(&context, sema).expect("cg");
    gen.generate_program(&program).expect("gen");

    let obj = "/tmp/bplang_trait_static_path_call_e2e.o";
    gen.write_object_file(obj).expect("obj");

    let bin = "/tmp/bplang_trait_static_path_call_e2e";
    let output = Command::new("clang")
        .args([obj, "-o", bin])
        .output()
        .expect("clang link");
    assert!(output.status.success(), "clang failed: {:?}", output);

    let run = Command::new(bin).output().expect("run");
    assert!(run.status.success());
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains("222\n"), "stdout = {}", stdout);
}

#[test]
fn ambiguous_trait_method_errors() {
    let src = r#"
        T :: { }
        TA :: trait { foo: (&T) -> i64, }
        TB :: trait { foo: (&T) -> i64, }
        impl TA for T { foo :: (self: &T) -> i64 => { 1 } }
        impl TB for T { foo :: (self: &T) -> i64 => { 2 } }
        main :: () -> i64 => {
            t: T := { }
            println(t.foo())
            0
        }
    "#
    .to_string();

    let program = parser::parse(src);
    let err = analyze_program(&program).expect_err("expected ambiguity error");
    match err {
        SemanticError::AmbiguousMethod { type_name, method, traits } => {
            assert_eq!(type_name, "T");
            assert_eq!(method, "foo");
            assert_eq!(traits.len(), 2);
        }
        other => panic!("unexpected error: {:?}", other),
    }
}
