use peano::ast::*;
use peano::parser;

#[test]
fn parse_struct_field_and_static_path() {
    let src = r#"
        Vec2 :: { x: i64, y: i64 }
        len :: (v: Vec2) -> i64 => { 0 }
        main :: () -> i64 => {
            println(Vec2::new)
            0
        }
    "#
    .to_string();

    let _ = parser::parse(src);
}

#[test]
fn parses_char_literal() {
    use peano::ast::{Expression, Literal, Program, Statement};
    let source = "'a'";
    let program = parser::parse(source.to_string());
    assert_eq!(
        program,
        Program {
            statements: vec![Statement::Expression(Expression::Literal(Literal::Char('a')))],
        }
    );
}

#[test]
fn parses_tuple_literal_and_pattern() {
    let src = r#"
        main :: () => {
            pair := (40, 2)
            value := match pair { (a, b) => a + b }
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);
}

#[test]
fn parse_struct_and_method_syntax() {
    let src = r#"
        point :: { x: i64, y: i64 }
        impl point {
            sum :: (self: &mut point) -> i64 => { self.x + self.y }
        }
        main :: () -> i64 => { 0 }
    "#
    .to_string();

    let _program = parser::parse(src);
}

#[test]
fn parse_trait_type_and_impl_for() {
    let src = r#"
        my_iterator :: trait {
            T: type,
            next: ( &mut self ) -> ?T,
        }
        my_struct :: { }
        impl my_iterator for my_struct {
            next :: (self: &mut my_struct) -> ?i32 => { }
        }
    "#
    .to_string();

    let program = parser::parse(src);
    // Expect 3 top-level statements: const trait, const struct, impl block
    assert_eq!(program.statements.len(), 3);

    // Check trait const
    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            assert_eq!(name, "my_iterator");
            match value {
                ConstValue::Type(Type::Trait {
                    associated_types,
                    methods,
                }) => {
                    assert!(associated_types.contains(&"T".to_string()));
                    assert!(methods.contains_key("next"));
                }
                other => panic!("expected trait type, got {:?}", other),
            }
        }
        other => panic!("expected ConstDecl for trait, got {:?}", other),
    }

    // Check impl block
    match &program.statements[2] {
        Statement::ImplBlock {
            trait_name,
            type_name,
            methods,
        } => {
            assert_eq!(trait_name.as_deref(), Some("my_iterator"));
            assert_eq!(type_name, "my_struct");
            assert_eq!(methods.len(), 1);
        }
        other => panic!("expected ImplBlock, got {:?}", other),
    }
}

#[test]
fn debug_enum_ast_prints() {
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
