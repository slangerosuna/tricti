use peano::ast::*;
use peano::parser;

#[test]
fn test_computed_column_basic() {
    let src = r#"
        Users :: table {
            first_name: String,
            last_name: String,
            full_name: computed(first_name + " " + last_name),
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            assert_eq!(name, "Users");
            match value {
                ConstValue::TableDef(table) => {
                    assert_eq!(table.columns.len(), 3);

                    let computed_col = &table.columns[2];
                    assert_eq!(computed_col.name, "full_name");
                    assert!(computed_col.is_computed);
                    assert!(computed_col.computed_expression.is_some());
                    assert!(computed_col.default_value.is_none());

                    // Verify the computed expression structure
                    if let Some(expr) = &computed_col.computed_expression {
                        match expr {
                            Expression::BinaryOp {
                                operator: BinaryOperator::Add,
                                ..
                            } => {
                                // Expected structure: first_name + " " + last_name
                            }
                            other => panic!("Expected binary add operation, got {:?}", other),
                        }
                    }
                }
                other => panic!("Expected TableDef, got {:?}", other),
            }
        }
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}

#[test]
fn test_computed_column_with_annotations() {
    let src = r#"
        Products :: table {
            price: f64,
            tax_rate: f64,
            @cached tax_amount: computed(price * tax_rate),
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => match value {
            ConstValue::TableDef(table) => {
                assert_eq!(table.columns.len(), 3);

                let computed_col = &table.columns[2];
                assert_eq!(computed_col.name, "tax_amount");
                assert!(computed_col.is_computed);
                assert_eq!(computed_col.annotations.len(), 1);
                assert_eq!(computed_col.annotations[0].name, "cached");
            }
            other => panic!("Expected TableDef, got {:?}", other),
        },
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}

#[test]
fn test_computed_column_complex_expression() {
    let src = r#"
        Orders :: table {
            quantity: u64,
            unit_price: f64,
            discount: f64,
            subtotal: computed(quantity * unit_price),
            discount_amount: computed(subtotal * discount),
            total: computed(subtotal - discount_amount),
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            match value {
                ConstValue::TableDef(table) => {
                    assert_eq!(table.columns.len(), 6);

                    // Verify all computed columns
                    let subtotal_col = &table.columns[3];
                    assert_eq!(subtotal_col.name, "subtotal");
                    assert!(subtotal_col.is_computed);

                    let discount_amount_col = &table.columns[4];
                    assert_eq!(discount_amount_col.name, "discount_amount");
                    assert!(discount_amount_col.is_computed);

                    let total_col = &table.columns[5];
                    assert_eq!(total_col.name, "total");
                    assert!(total_col.is_computed);
                }
                other => panic!("Expected TableDef, got {:?}", other),
            }
        }
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}

#[test]
fn test_mixed_regular_and_computed_columns() {
    let src = r#"
        Employees :: table {
            @primary id: u64,
            first_name: String,
            last_name: String,
            salary: f64 = 0.0,
            full_name: computed(first_name + " " + last_name),
            @indexed department: String,
            annual_salary: computed(salary * 12),
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            match value {
                ConstValue::TableDef(table) => {
                    assert_eq!(table.columns.len(), 7);

                    // Verify mixed column types
                    let id_col = &table.columns[0];
                    assert!(!id_col.is_computed);
                    assert_eq!(id_col.annotations.len(), 1);

                    let salary_col = &table.columns[3];
                    assert!(!salary_col.is_computed);
                    assert!(salary_col.default_value.is_some());

                    let full_name_col = &table.columns[4];
                    assert!(full_name_col.is_computed);

                    let dept_col = &table.columns[5];
                    assert!(!dept_col.is_computed);
                    assert_eq!(dept_col.annotations.len(), 1);

                    let annual_salary_col = &table.columns[6];
                    assert!(annual_salary_col.is_computed);
                }
                other => panic!("Expected TableDef, got {:?}", other),
            }
        }
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}

#[test]
fn test_computed_column_with_function_calls() {
    let src = r#"
        Analytics :: table {
            raw_data: String,
            processed_length: computed(len(raw_data)),
            hash_value: computed(hash(raw_data)),
            is_valid: computed(processed_length > 0 and hash_value ~= 0),
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => match value {
            ConstValue::TableDef(table) => {
                assert_eq!(table.columns.len(), 4);

                let computed_col = &table.columns[1];
                assert_eq!(computed_col.name, "processed_length");
                assert!(computed_col.is_computed);

                let hash_col = &table.columns[2];
                assert_eq!(hash_col.name, "hash_value");
                assert!(hash_col.is_computed);

                let is_valid_col = &table.columns[3];
                assert_eq!(is_valid_col.name, "is_valid");
                assert!(is_valid_col.is_computed);
            }
            other => panic!("Expected TableDef, got {:?}", other),
        },
        other => panic!("Expected ConstDecl, got {:?}", other),
    }
}
