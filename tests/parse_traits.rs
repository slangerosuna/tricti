use peano::ast::*;
use peano::parser;

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
