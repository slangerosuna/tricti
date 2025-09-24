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
    let program = parser::parse(src.to_string());
    let _ = semantic::analyze_program(&program).expect("semantic analysis");
}

#[test]
fn range_with_zero_step_is_error() {
    let src = "for i in 0:10:0 { println(i) }";
    let program = parser::parse(src.to_string());
    let err = semantic::analyze_program(&program).expect_err("expected step=0 error");
    match err {
        semantic::SemanticError::InvalidRangeStepZero => {}
        other => panic!("unexpected error: {:?}", other),
    }
}

#[test]
fn records_function_generic_params() {
    let src = r#"
        id <T> :: (x: T) -> T => { ret x }
    "#
    .to_string();

    let program = parser::parse(src);
    let context = semantic::analyze_program(&program).expect("semantic");

    assert_eq!(
        context.function_generics.get("id"),
        Some(&vec!["T".to_string()])
    );
}

#[test]
fn records_type_generic_params() {
    let src = r#"
        Option <T> :: enum {
            Some: T,
            None: none,
        }
    "#
    .to_string();

    let program = parser::parse(src);
    let context = semantic::analyze_program(&program).expect("semantic");

    assert_eq!(
        context.type_generics.get("Option"),
        Some(&vec!["T".to_string()])
    );
}
