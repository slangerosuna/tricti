use inkwell::context::Context;
use tricti::{
    codegen::CodeGenerator,
    parser, semantic,
    tri_test_helpers::{clang_available, compile_and_run_tri, dedent},
};

fn compile_ir(source: &str) -> String {
    let tri_src = dedent(source);
    let program = parser::parse(tri_src.clone());
    let sema = semantic::analyze_program(&program).expect("semantic analysis");
    let context = Context::create();
    let mut gen = CodeGenerator::new(&context, sema).expect("codegen");
    gen.generate_program(&program).expect("generate_program");
    gen.ir_to_string()
}

fn run_tri(source: &str, tag: &str) -> String {
    let tri_src = dedent(source);
    let obj_path = format!("tests/tmp_{}.o", tag);
    let exe_path = format!("tests/tmp_{}.out", tag);
    compile_and_run_tri(&tri_src, &obj_path, &exe_path)
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

#[test]
fn drop_runs_on_scope_exit() {
    let ir = compile_ir(
        r#"
        Drop :: trait
            drop :: (*Self) -> none,

        ScopeResource :: struct
            value: i64,

        impl Drop for ScopeResource:
            drop :: (self: *ScopeResource) => do
                ret

        make_scope_resource :: () -> ScopeResource => ScopeResource { value: 42 }

        main :: () => do
            res: ScopeResource := make_scope_resource()
        "#,
    );

    let drop_call = "call i64 @Drop_ScopeResource_drop";
    assert!(
        ir.contains(drop_call),
        "expected implicit drop call on scope exit, IR was:\n{}",
        ir
    );
    assert_eq!(
        count_occurrences(&ir, drop_call),
        1,
        "expected exactly one drop invocation on scope exit"
    );
}

#[test]
fn drop_runs_before_explicit_return() {
    let ir = compile_ir(
        r#"
        Drop :: trait
            drop :: (*Self) -> none,

        ReturnResource :: struct
            value: i64,

        impl Drop for ReturnResource:
            drop :: (self: *ReturnResource) => do
                ret

        make_return_resource :: () -> ReturnResource => ReturnResource { value: 10 }

        test_fn :: () -> i64 => do
            res: ReturnResource := make_return_resource()
            ret 7
        "#,
    );

    let drop_call = "call i64 @Drop_ReturnResource_drop";
    assert!(
        ir.contains(drop_call),
        "expected drop before explicit return, IR was:\n{}",
        ir
    );
    assert_eq!(
        count_occurrences(&ir, drop_call),
        1,
        "expected single drop invocation prior to explicit return"
    );
}

#[test]
fn drop_occurs_on_assignment_before_overwrite() {
    let ir = compile_ir(
        r#"
        Drop :: trait
            drop :: (*Self) -> none,

        AssignResource :: struct
            value: i64,

        impl Drop for AssignResource:
            drop :: (self: *AssignResource) => do
                ret

        make_assign_resource :: (value: i64) -> AssignResource => AssignResource { value: value }

        main :: () => do
            res: AssignResource := make_assign_resource(1)
            res = make_assign_resource(2)
        "#,
    );

    let drop_call = "call i64 @Drop_AssignResource_drop";
    assert!(
        ir.contains(drop_call),
        "expected drop calls around assignment, IR was:\n{}",
        ir
    );
    assert_eq!(
        count_occurrences(&ir, drop_call),
        2,
        "expected drop on overwrite and drop at scope exit"
    );
}

#[test]
fn explicit_drop_prevents_double_drop() {
    let ir = compile_ir(
        r#"
        Drop :: trait
            drop :: (*Self) -> none,

        ExplicitResource :: struct
            value: i64,

        impl Drop for ExplicitResource:
            drop :: (self: *ExplicitResource) => do
                ret

        make_explicit_resource :: () -> ExplicitResource => ExplicitResource { value: 0 }

        main :: () => do
            res: ExplicitResource := make_explicit_resource()
            drop(res)
        "#,
    );

    let drop_call = "call i64 @Drop_ExplicitResource_drop";
    assert!(
        ir.contains(drop_call),
        "expected explicit drop to emit drop call, IR was:\n{}",
        ir
    );
    assert_eq!(
        count_occurrences(&ir, drop_call),
        1,
        "explicit drop should invoke destructor exactly once"
    );
}

#[test]
fn drop_runs_on_explicit_return() {
    if !clang_available() {
        eprintln!("clang not found; skipping drop_runs_on_explicit_return");
        return;
    }

    let stdout = run_tri(
        r#"
        Drop :: trait
            drop :: (*Self) -> none,

        MyResource :: struct
            id: i64,

        impl Drop for MyResource:
            drop :: (self: *MyResource) => do
                println(self.id)

        cleanup_on_return :: () => do
            res: MyResource := MyResource { id: 7 }
            ret

        main :: () => do
            cleanup_on_return()
        "#,
        "drop_explicit_return",
    );

    assert_eq!(stdout, "7\n");
}

#[test]
fn drop_on_assignment_and_explicit_drop() {
    if !clang_available() {
        eprintln!("clang not found; skipping drop_on_assignment_and_explicit_drop");
        return;
    }

    let stdout = run_tri(
        r#"
        Drop :: trait
            drop :: (*Self) -> none,

        MyResource :: struct
            id: i64,

        impl Drop for MyResource:
            drop :: (self: *MyResource) => do
                println(self.id)

        make :: (id: i64) -> MyResource => MyResource { id: id }

        main :: () => do
            value: MyResource := make(1)
            value = make(2)
            drop(value)
        "#,
        "drop_assignment_and_explicit",
    );

    assert_eq!(stdout, "1\n2\n");
}

#[test]
fn drop_runs_after_move_out_of_function() {
    if !clang_available() {
        eprintln!("clang not found; skipping drop_runs_after_move_out_of_function");
        return;
    }

    let stdout = run_tri(
        r#"
        Drop :: trait
            drop :: (*Self) -> none,

        MyResource :: struct
            id: i64,

        impl Drop for MyResource:
            drop :: (self: *MyResource) => do
                println(self.id)

        make :: () -> MyResource => do
            res := MyResource { id: 3 }
            ret res

        main :: () => do
            value: MyResource := make()
            println(100)
        "#,
        "drop_after_move",
    );

    assert_eq!(stdout, "100\n3\n");
}
