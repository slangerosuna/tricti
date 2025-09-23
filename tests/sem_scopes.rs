use peano::{parser, semantic};

#[test]
fn loop_variable_is_scoped() {
    let src = r#"
        for i in 3 { println(i) }
        println(i) # i should be undefined here
    "#;
    let program = parser::parse(src.to_string());
    let err = semantic::analyze_program(&program).err();
    assert!(matches!(err, Some(semantic::SemanticError::UndefinedVariable(v)) if v == "i"));
}

#[test]
fn shadowing_within_inner_scope() {
    let src = r#"
        x := 10
        for i in 2 {
            x := 5
            println(x)
        }
        println(x)
    "#;
    // Should succeed semantically (inner x shadows outer)
    let program = parser::parse(src.to_string());
    let _ = semantic::analyze_program(&program).expect("semantic analysis");
}
