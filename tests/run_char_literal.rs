use peano::{ast::{Expression, Literal, Program, Statement}, parser::parse};

#[test]
fn test_char_literal() {
    let source = "'a'";
    let program = parse(source.to_string());
    assert_eq!(program, Program { statements: vec![Statement::Expression(Expression::Literal(Literal::Char('a')))] });
}
