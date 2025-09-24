use peano::parser;
use peano::semantic::{analyze_program, SemanticError};

#[test]
fn ambiguous_trait_method_errors() {
    let src = r#"
        T :: { }
        TA :: trait { foo: (&T) -> i64, }
        TB :: trait { foo: (&T) -> i64, }
        impl TA for T { foo :: (self: &T) -> i64 => { 1 } }
        impl TB for T { foo :: (self: &T) -> i64 => { 2 } }
        main :: () -> i64 => {
            t: T := { }
            println(t.foo())
            0
        }
    "#
    .to_string();

    let program = parser::parse(src);
    println!("AST: {:#?}", program);
    let err = analyze_program(&program).expect_err("expected ambiguity error");
    match err {
        SemanticError::AmbiguousMethod {
            type_name,
            method,
            traits,
        } => {
            assert_eq!(type_name, "T");
            assert_eq!(method, "foo");
            assert_eq!(traits.len(), 2);
        }
        other => panic!("unexpected error: {:?}", other),
    }
}
