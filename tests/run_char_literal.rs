
### Run Tests

Now that all changes have been made, I will run the tests to ensure everything works as expected.

**Action: Run the tests.**
    "#;

    let program = parser::parse(src.to_string());
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_char_literals.o";
    let exe = "tests/tmp_char_literals.out";
    if Path::new(obj).exists() {
        let _ = fs::remove_file(obj);
    }
    if Path::new(exe).exists() {
        let _ = fs::remove_file(exe);
    }
    gen.write_object_file(obj).expect("write obj");

    let status = Command::new("clang")
        .args(["-o", exe, obj])
        .status()
        .expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(out.status.success(), "program failed to run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout, "A\nZ\n");
}
```rust
#[test]
fn test_char_literal() {
    let input = "'\\u0041'"; // Represents 'A'
    let result = parse(input);
    assert_eq!(result, Ok(LiteralKind::Char(0x0041)));
}```rust
#[test]
fn test_char_literal() {
    let source = "'a'";
    let result = parse(source);
    assert_eq!(result, Ok(Expr::Literal(LiteralKind::Char('a' as u32), span)));
}
