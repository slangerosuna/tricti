#[test]
fn parses_tuple_literal_and_pattern() {
    use peano::parser;

    let src = r#"
        main :: () => {
            pair := (40, 2)
            value := match pair { (a, b) => a + b }
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);
}
