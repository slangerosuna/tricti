use peano::{parser, semantic};

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
