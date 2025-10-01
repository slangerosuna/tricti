use tricti::ast::*;
use tricti::parser;

#[test]
fn test_simple_table_definition() {
    let src = r#"
        Users :: table {
            id: u64,
            name: String,
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            assert_eq!(name, "Users");
            match value {
                ConstValue::TableDef(table) => {
                    assert_eq!(table.name, "Users");
                    assert_eq!(table.columns.len(), 2);

                    let id_col = &table.columns[0];
                    assert_eq!(id_col.name, "id");
                    assert_eq!(
                        id_col.column_type,
                        Type::Identifier {
                            name: "u64".to_string(),
                            type_args: vec![]
                        }
                    );
                    assert!(id_col.annotations.is_empty());
                    assert!(id_col.default_value.is_none());

                    let name_col = &table.columns[1];
                    assert_eq!(name_col.name, "name");
                    assert_eq!(
                        name_col.column_type,
                        Type::Identifier {
                            name: "String".to_string(),
                            type_args: vec![]
                        }
                    );
                    assert!(name_col.annotations.is_empty());
                    assert!(name_col.default_value.is_none());
                }
                other => panic!("Expected TableDef, got {:?}", other),
            }
        }
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}

#[test]
fn test_table_with_primary_key_annotation() {
    let src = r#"
        Apps :: table {
            @primary id: u64,
            title: String,
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            assert_eq!(name, "Apps");
            match value {
                ConstValue::TableDef(table) => {
                    assert_eq!(table.name, "Apps");
                    assert_eq!(table.columns.len(), 2);

                    let id_col = &table.columns[0];
                    assert_eq!(id_col.name, "id");
                    assert_eq!(id_col.annotations.len(), 1);
                    assert_eq!(id_col.annotations[0].name, "primary");
                    assert!(id_col.annotations[0].args.is_empty());
                }
                other => panic!("Expected TableDef, got {:?}", other),
            }
        }
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}

#[test]
fn test_table_with_default_values() {
    let src = r#"
        Products :: table {
            id: u64,
            name: String,
            active: bool = true,
            price: f64 = 0.0,
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            assert_eq!(name, "Products");
            match value {
                ConstValue::TableDef(table) => {
                    assert_eq!(table.columns.len(), 4);

                    let active_col = &table.columns[2];
                    assert_eq!(active_col.name, "active");
                    assert!(active_col.default_value.is_some());
                    match &active_col.default_value {
                        Some(Expression::Literal(Literal::Boolean(true))) => {}
                        other => panic!("Expected boolean true default, got {:?}", other),
                    }

                    let price_col = &table.columns[3];
                    assert_eq!(price_col.name, "price");
                    assert!(price_col.default_value.is_some());
                    match &price_col.default_value {
                        Some(Expression::Literal(Literal::Float(val))) => {
                            assert!((*val - 0.0).abs() < f64::EPSILON);
                        }
                        other => panic!("Expected float 0.0 default, got {:?}", other),
                    }
                }
                other => panic!("Expected TableDef, got {:?}", other),
            }
        }
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}

#[test]
fn test_table_with_multiple_annotations() {
    let src = r#"
        Orders :: table {
            @primary @autoincrement id: u64,
            @indexed customer_id: u64,
            @nullable description: String,
            total: f64,
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            assert_eq!(name, "Orders");
            match value {
                ConstValue::TableDef(table) => {
                    let id_col = &table.columns[0];
                    assert_eq!(id_col.annotations.len(), 2);
                    assert_eq!(id_col.annotations[0].name, "primary");
                    assert_eq!(id_col.annotations[1].name, "autoincrement");

                    let customer_col = &table.columns[1];
                    assert_eq!(customer_col.annotations.len(), 1);
                    assert_eq!(customer_col.annotations[0].name, "indexed");

                    let desc_col = &table.columns[2];
                    assert_eq!(desc_col.annotations.len(), 1);
                    assert_eq!(desc_col.annotations[0].name, "nullable");
                }
                other => panic!("Expected TableDef, got {:?}", other),
            }
        }
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}

#[test]
fn test_table_with_complex_types() {
    let src = r#"
        Documents :: table {
            id: u64,
            tags: [String],
            metadata: {name: String, value: String},
            owner: ?User,
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            assert_eq!(name, "Documents");
            match value {
                ConstValue::TableDef(table) => {
                    assert_eq!(table.columns.len(), 4);

                    let tags_col = &table.columns[1];
                    assert_eq!(tags_col.name, "tags");
                    // Should be a matrix/array type

                    let metadata_col = &table.columns[2];
                    assert_eq!(metadata_col.name, "metadata");
                    // Should be a struct type

                    let owner_col = &table.columns[3];
                    assert_eq!(owner_col.name, "owner");
                    // Should be an optional type
                    match &owner_col.column_type {
                        Type::Optional { inner } => match inner.as_ref() {
                            Type::Identifier { name, .. } => {
                                assert_eq!(name, "User");
                            }
                            other => {
                                panic!("Expected User identifier inside optional, got {:?}", other)
                            }
                        },
                        other => panic!("Expected optional type, got {:?}", other),
                    }
                }
                other => panic!("Expected TableDef, got {:?}", other),
            }
        }
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}

#[test]
fn test_empty_table_definition() {
    let src = r#"
        EmptyTable :: table {
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            assert_eq!(name, "EmptyTable");
            match value {
                ConstValue::TableDef(table) => {
                    assert_eq!(table.name, "EmptyTable");
                    assert_eq!(table.columns.len(), 0);
                }
                other => panic!("Expected TableDef, got {:?}", other),
            }
        }
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}

#[test]
fn test_annotation_with_parameters() {
    let src = r#"
        Logs :: table {
            @primary @autoincrement id: u64,
            @size(255) message: String,
            @precision(10, 2) amount: f64,
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => match value {
            ConstValue::TableDef(table) => {
                let message_col = &table.columns[1];
                assert_eq!(message_col.annotations.len(), 1);
                assert_eq!(message_col.annotations[0].name, "size");
                assert_eq!(message_col.annotations[0].args.len(), 1);

                let amount_col = &table.columns[2];
                assert_eq!(amount_col.annotations.len(), 1);
                assert_eq!(amount_col.annotations[0].name, "precision");
                assert_eq!(amount_col.annotations[0].args.len(), 2);
            }
            other => panic!("Expected TableDef, got {:?}", other),
        },
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}
