use tricti::tri_test_helpers::{clang_available, compile_and_run_tri, dedent};

fn run_tri(source: &str, suffix: &str) -> String {
    let tri_src = dedent(source);
    let obj_path = format!("tests/tmp_{}.o", suffix);
    let exe_path = format!("tests/tmp_{}.out", suffix);
    compile_and_run_tri(&tri_src, &obj_path, &exe_path)
}

#[test]
fn drop_runs_on_scope_exit() {
    if !clang_available() {
        eprintln!("clang not found; skipping drop_runs_on_scope_exit");
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

        make_resource :: (id: i64) -> MyResource => MyResource { id: id }

        main :: () => do
            res := make_resource(41)
        "#,
        "drop_scope_exit",
    );

        assert_eq!(stdout, "");
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
            res := MyResource { id: 7 }
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
            mut value := make(1)
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
            value := make()
            println(100)
        "#,
        "drop_after_move",
    );

    assert_eq!(stdout, "100\n3\n");
}