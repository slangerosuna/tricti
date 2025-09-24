use peano::parser;

#[test]
fn print_enum_ast() {
    let src = r#"
        Color :: enum { Red, Green, Blue }
        main :: () => {
            c: Color := 2
            println(c)
        }
    "#;
    let program = parser::parse(src.to_string());
    println!("AST: {:#?}", program);
}
