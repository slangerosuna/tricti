use tricti::ast::*;
use tricti::parser;

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
    use tricti::ast::{Expression, Literal, Program, Statement};
    let source = "'a'";
    let program = parser::parse(source.to_string());
    assert_eq!(
        program,
        Program {
            statements: vec![Statement::Expression(Expression::Literal(Literal::Char(
                'a'
            )))],
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
fn parse_struct_with_optional_field() {
    let src = r#"
        StdError :: struct {
            parameter: ?string,
        }
    "#
    .to_string();

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);
    if let Statement::ConstDecl { value, .. } = &program.statements[0] {
        match value {
            ConstValue::Type(Type::Struct { fields }) => {
                assert_eq!(fields.len(), 1);
                let ty = fields.get("parameter").expect("parameter field");
                match ty {
                    Type::Optional { inner } => match inner.as_ref() {
                        Type::Identifier { name, .. } => assert_eq!(name, "string"),
                        other => panic!("expected optional string, got {:?}", other),
                    },
                    other => panic!("expected optional type, got {:?}", other),
                }
            }
            other => panic!("expected struct type, got {:?}", other),
        }
    } else {
        panic!("expected const declaration");
    }
}

#[test]
fn parse_const_decl_with_attributes() {
    let src = r#"
        @par_const Gui :: @resource {}
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);
    match &program.statements[0] {
        Statement::ConstDecl { attributes, .. } => {
            assert_eq!(attributes.len(), 2);
            assert_eq!(attributes[0].name, "par_const");
            assert!(attributes[0].arguments.is_empty());
            assert_eq!(attributes[1].name, "resource");
            assert!(attributes[1].arguments.is_empty());
        }
        other => panic!("expected const declaration, got {:?}", other),
    }
}

#[test]
fn parse_function_with_attribute() {
    let src = r#"
        @memoize fib :: (n: i32) -> i32 => { n }
    "#;

    let program = parser::parse(src.to_string());
    match &program.statements[0] {
        Statement::ConstDecl {
            attributes, value, ..
        } => {
            assert_eq!(attributes.len(), 1);
            assert_eq!(attributes[0].name, "memoize");
            assert!(attributes[0].arguments.is_empty());

            match value {
                ConstValue::Expression(Expression::Function { attributes, .. }) => {
                    assert!(attributes.is_empty(), "function-level attributes should be empty when defined via const attribute");
                }
                other => panic!("expected function expression, got {:?}", other),
            }
        }
        other => panic!("expected const declaration, got {:?}", other),
    }
}

#[test]
fn parse_attribute_with_identifier_arguments() {
    let src = r#"
    @trigger(ExampleSignal, Database)
        receiver_sys :: sys () => {}
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);
    match &program.statements[0] {
        Statement::ConstDecl { attributes, .. } => {
            assert_eq!(attributes.len(), 1);
            let attr = &attributes[0];
            assert_eq!(attr.name, "trigger");
            assert_eq!(attr.arguments.len(), 2);
            assert_eq!(
                attr.arguments[0],
                Expression::Identifier("ExampleSignal".to_string())
            );
            assert_eq!(
                attr.arguments[1],
                Expression::Identifier("Database".to_string())
            );
        }
        other => panic!("expected const declaration, got {:?}", other),
    }
}

#[test]
fn parse_new_vec_expression() {
    let src = r#"
        output := new [i64; 0]
    "#
    .to_string();

    let program = parser::parse(src);
    assert_eq!(program.statements.len(), 1);
    match &program.statements[0] {
        Statement::VariableDecl { value, .. } => match value {
            Expression::VecNew {
                element_type,
                length,
                additional_dimensions,
                ..
            } => {
                match element_type {
                    Type::Identifier { name, .. } => assert_eq!(name, "i64"),
                    other => panic!("expected identifier type, got {:?}", other),
                }
                assert!(additional_dimensions.is_empty());
                let len_expr = length
                    .as_ref()
                    .map(|expr| expr.as_ref())
                    .expect("expected length expression");
                match len_expr {
                    Expression::Literal(Literal::Integer(int_lit)) => {
                        assert_eq!(int_lit.value, 0);
                    }
                    other => panic!("expected integer literal dimension, got {:?}", other),
                }
            }
            other => panic!("expected vec_new expression, got {:?}", other),
        },
        other => panic!("expected variable declaration, got {:?}", other),
    }
}

#[test]
fn parse_if_expression_in_block() {
    let src = r#"
        abs_i64 :: (x: i64) -> i64 => { if x < 0 { -x } else { x } }
    "#
    .to_string();

    let program = parser::parse(src);
    assert_eq!(program.statements.len(), 1);
}

#[test]
fn parse_identifier_comparison_expression() {
    let src = r#"
        result := value < 10
    "#
    .to_string();

    let program = parser::parse(src);
    assert_eq!(program.statements.len(), 1);
}

#[test]
fn parse_literal_comparison_expression() {
    let src = r#"
        result := 1 < 2
    "#
    .to_string();

    let program = parser::parse(src);
    assert_eq!(program.statements.len(), 1);
}

#[test]
fn parse_identifier_division_expression() {
    let src = r#"
        q := a / b
    "#
    .to_string();

    let program = parser::parse(src);
    assert_eq!(program.statements.len(), 1);
}

#[test]
fn parse_integer_with_underscores() {
    let src = r#"
        value := 5_000
    "#
    .to_string();

    let program = parser::parse(src);
    assert_eq!(program.statements.len(), 1);
}

#[test]
fn parse_match_with_return_and_value_arms() {
    let src = r#"
        result := match input {
            some value => value,
            none => ret default,
        }
    "#
    .to_string();

    let program = parser::parse(src);
    assert_eq!(program.statements.len(), 1);
}

#[test]
fn parse_async_system_with_query_and_resource() {
    let src = r#"
        display_apps :: async sys (
            query apps: select (image: &Image, title: &String)
                from Apps
                where display == true,
            renderer: res &mut Gui,
            input_size: f32 = 1.5,
        ) -> none => {
            println("hi")
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            assert_eq!(name, "display_apps");
            match value {
                ConstValue::SystemDef(system) => {
                    assert!(system.is_async, "system should be marked async");
                    assert_eq!(system.parameters.len(), 3);

                    match &system.parameters[0] {
                        SystemParameter::Query { name, query_spec } => {
                            assert_eq!(name, "apps");
                            assert_eq!(query_spec.from_table, "Apps");
                            assert_eq!(query_spec.projections.len(), 2);
                            assert_eq!(query_spec.projections[0].name, "image");
                            assert_eq!(
                                query_spec.projections[0].access,
                                Some(ResourceAccess::Immutable)
                            );
                            match &query_spec.projections[0].field_type {
                                Some(Type::Reference { is_mutable, inner }) => {
                                    assert!(!is_mutable);
                                    assert_eq!(
                                        inner.as_ref(),
                                        &Type::Identifier {
                                            name: "Image".to_string(),
                                            type_args: vec![],
                                        }
                                    );
                                }
                                other => panic!(
                                    "expected immutable reference projection, got {:?}",
                                    other
                                ),
                            }
                        }
                        other => panic!("expected query parameter, got {:?}", other),
                    }

                    match &system.parameters[1] {
                        SystemParameter::Resource {
                            name,
                            access,
                            resource_type,
                            ..
                        } => {
                            assert_eq!(name, "renderer");
                            assert_eq!(*access, ResourceAccess::Mutable);
                            assert_eq!(
                                resource_type,
                                &Type::Identifier {
                                    name: "Gui".to_string(),
                                    type_args: vec![],
                                }
                            );
                        }
                        other => panic!("expected resource parameter, got {:?}", other),
                    }

                    match &system.parameters[2] {
                        SystemParameter::Regular {
                            name,
                            value_type,
                            default_value,
                            ..
                        } => {
                            assert_eq!(name, "input_size");
                            assert_eq!(
                                value_type,
                                &Type::Identifier {
                                    name: "f32".to_string(),
                                    type_args: vec![],
                                }
                            );
                            match default_value {
                                Some(Expression::Literal(Literal::Float(value))) => {
                                    assert!((value - 1.5).abs() < f64::EPSILON);
                                }
                                other => {
                                    panic!("expected float literal default value, got {:?}", other)
                                }
                            }
                        }
                        other => panic!("expected regular parameter, got {:?}", other),
                    }
                }
                other => panic!("expected system definition, got {:?}", other),
            }
        }
        other => panic!("expected const declaration, got {:?}", other),
    }
}

#[test]
fn parse_system_query_without_name_defaults() {
    let src = r#"
        redraw :: sys (
            query: select (id: &u64)
                from Apps,
            renderer: res &Gui,
        ) => {
            println("ok")
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { value, .. } => match value {
            ConstValue::SystemDef(system) => {
                assert!(!system.is_async);
                assert_eq!(system.parameters.len(), 2);

                match &system.parameters[0] {
                    SystemParameter::Query { name, query_spec } => {
                        assert_eq!(name, "query");
                        assert_eq!(query_spec.from_table, "Apps");
                        assert_eq!(query_spec.projections.len(), 1);
                        assert_eq!(query_spec.projections[0].name, "id");
                    }
                    other => panic!("expected query parameter, got {:?}", other),
                }

                match &system.parameters[1] {
                    SystemParameter::Resource { name, access, .. } => {
                        assert_eq!(name, "renderer");
                        assert_eq!(*access, ResourceAccess::Immutable);
                    }
                    other => panic!("expected resource parameter, got {:?}", other),
                }
            }
            other => panic!("expected system definition, got {:?}", other),
        },
        other => panic!("expected const declaration, got {:?}", other),
    }
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
            type_params,
        } => {
            assert_eq!(trait_name.as_deref(), Some("my_iterator"));
            assert_eq!(type_name, "my_struct");
            assert_eq!(methods.len(), 1);
            assert!(type_params.is_empty());
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
