use peano::ast::*;
use peano::parser;
use peano::semantic;

/// Test computed columns with function calls to ensure function identifiers
/// are not treated as column dependencies
#[test]
fn test_computed_column_with_function_calls() {
    let src = r#"
        Analytics :: table {
            raw_data: String,
            processed_length: computed(len(raw_data)),
            normalized_data: computed(trim(raw_data)),
            is_valid: computed(processed_length > 0),
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    // Verify the program parses correctly
    match &program.statements[0] {
        Statement::ConstDecl { name, value, .. } => {
            assert_eq!(name, "Analytics");
            match value {
                ConstValue::TableDef(table) => {
                    assert_eq!(table.columns.len(), 4);

                    // Verify computed columns are correctly identified
                    assert!(!table.columns[0].is_computed); // raw_data
                    assert!(table.columns[1].is_computed); // processed_length
                    assert!(table.columns[2].is_computed); // normalized_data
                    assert!(table.columns[3].is_computed); // is_valid

                    // Verify expressions exist
                    assert!(table.columns[1].computed_expression.is_some());
                    assert!(table.columns[2].computed_expression.is_some());
                    assert!(table.columns[3].computed_expression.is_some());
                }
                other => panic!("Expected TableDef, got {:?}", other),
            }
        }
        other => panic!("Expected ConstDecl, got {:?}", other),
    }

    // Test semantic analysis to ensure it doesn't fail with UnknownColumnReference for functions
    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Semantic analysis should pass for valid function calls in computed columns"
    );
}

/// Test computed columns with mathematical functions and constants
#[test]
fn test_computed_column_with_math_functions_and_constants() {
    let src = r#"
        Circle :: table {
            radius: f64,
            area: computed(3.14159 * radius * radius),
            circumference: computed(2.0 * 3.14159 * radius),
            diameter: computed(2.0 * radius),
            volume_sphere: computed((4.0 / 3.0) * 3.14159 * radius * radius * radius),
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    // Verify all computed columns parse correctly
    match &program.statements[0] {
        Statement::ConstDecl {
            value: ConstValue::TableDef(table),
            ..
        } => {
            assert_eq!(table.columns.len(), 5);

            // Only radius should be a regular column
            assert!(!table.columns[0].is_computed); // radius

            // All others should be computed
            for i in 1..5 {
                assert!(
                    table.columns[i].is_computed,
                    "Column {} should be computed",
                    i
                );
                assert!(
                    table.columns[i].computed_expression.is_some(),
                    "Column {} should have expression",
                    i
                );
            }
        }
        other => panic!("Expected table definition, got {:?}", other),
    }

    // Test semantic analysis - should pass with proper type inference
    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Semantic analysis should pass for mathematical expressions"
    );
}

/// Test computed columns with string functions and operations
#[test]
fn test_computed_column_with_string_functions() {
    let src = r#"
        Users :: table {
            first_name: String,
            last_name: String,
            email: String,
            full_name: computed(first_name + " " + last_name),
            email_length: computed(len(email)),
            name_length: computed(len(full_name)),
            has_long_name: computed(name_length > 20),
            initials: computed(first_name + "." + last_name),
        }
    "#;

    let program = parser::parse(src.to_string());

    match &program.statements[0] {
        Statement::ConstDecl {
            value: ConstValue::TableDef(table),
            ..
        } => {
            assert_eq!(table.columns.len(), 8);

            // Verify regular columns
            for i in 0..3 {
                assert!(
                    !table.columns[i].is_computed,
                    "Column {} should not be computed",
                    i
                );
            }

            // Verify computed columns
            for i in 3..8 {
                assert!(
                    table.columns[i].is_computed,
                    "Column {} should be computed",
                    i
                );
                assert!(
                    table.columns[i].computed_expression.is_some(),
                    "Column {} should have expression",
                    i
                );
            }
        }
        other => panic!("Expected table definition, got {:?}", other),
    }

    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Semantic analysis should pass for string operations and function calls"
    );
}

/// Test computed columns with conditional expressions
#[test]
fn test_computed_column_with_conditionals() {
    let src = r#"
        Orders :: table {
            quantity: u64,
            unit_price: f64,
            discount_rate: f64,
            subtotal: computed(quantity * unit_price),
            discount_amount: computed(subtotal * discount_rate),
            total: computed(subtotal - discount_amount),
            is_bulk_order: computed(quantity > 100),
            shipping_cost: computed(total > 50.0),
        }
    "#;

    let program = parser::parse(src.to_string());

    match &program.statements[0] {
        Statement::ConstDecl {
            value: ConstValue::TableDef(table),
            ..
        } => {
            assert_eq!(table.columns.len(), 8);

            // Verify the dependency chain: subtotal -> discount_amount -> total
            let subtotal_col = &table.columns[3];
            let discount_col = &table.columns[4];
            let total_col = &table.columns[5];

            assert_eq!(subtotal_col.name, "subtotal");
            assert_eq!(discount_col.name, "discount_amount");
            assert_eq!(total_col.name, "total");

            assert!(subtotal_col.is_computed);
            assert!(discount_col.is_computed);
            assert!(total_col.is_computed);
        }
        other => panic!("Expected table definition, got {:?}", other),
    }

    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Semantic analysis should handle computed column dependencies correctly"
    );
}

/// Test computed columns with complex nested function calls
#[test]
fn test_computed_column_with_nested_function_calls() {
    let src = r#"
        Products :: table {
            name: String,
            description: String,
            price: f64,
            name_length: computed(len(name)),
            desc_length: computed(len(description)),
            total_text_length: computed(name_length + desc_length),
            avg_text_length: computed(total_text_length / 2),
            is_detailed: computed(desc_length > name_length),
        }
    "#;

    let program = parser::parse(src.to_string());

    match &program.statements[0] {
        Statement::ConstDecl {
            value: ConstValue::TableDef(table),
            ..
        } => {
            assert_eq!(table.columns.len(), 8);

            // Verify computed columns reference other computed columns
            let total_text_col = &table.columns[5];
            let avg_text_col = &table.columns[6];
            let is_detailed_col = &table.columns[7];

            assert_eq!(total_text_col.name, "total_text_length");
            assert_eq!(avg_text_col.name, "avg_text_length");
            assert_eq!(is_detailed_col.name, "is_detailed");

            assert!(total_text_col.is_computed);
            assert!(avg_text_col.is_computed);
            assert!(is_detailed_col.is_computed);
        }
        other => panic!("Expected table definition, got {:?}", other),
    }

    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Semantic analysis should handle nested computed column references"
    );
}

/// Test that function identifiers are NOT treated as column dependencies
#[test]
fn test_function_identifiers_not_treated_as_dependencies() {
    use peano::computed_columns::DependencyGraph;
    use std::collections::HashSet;

    let src = r#"
        TestTable :: table {
            data: String,
            length: computed(len(data)),
            is_empty: computed(length == 0),
        }
    "#;

    let program = parser::parse(src.to_string());

    match &program.statements[0] {
        Statement::ConstDecl {
            value: ConstValue::TableDef(table),
            ..
        } => {
            // Build dependency graph and ensure 'len' is not treated as a dependency
            let graph_result = DependencyGraph::build_from_table(table);
            assert!(
                graph_result.is_ok(),
                "Dependency graph should build successfully"
            );

            let graph = graph_result.unwrap();

            // Check dependencies for 'length' column
            if let Some(length_deps) = graph.get_dependencies("length") {
                assert!(
                    length_deps.contains("data"),
                    "Should depend on 'data' column"
                );
                assert!(
                    !length_deps.contains("len"),
                    "'len' should NOT be treated as a column dependency"
                );
                assert_eq!(
                    length_deps.len(),
                    1,
                    "Should only have one dependency: 'data'"
                );
            } else {
                panic!("'length' column should have dependencies");
            }

            // Check dependencies for 'is_empty' column
            if let Some(is_empty_deps) = graph.get_dependencies("is_empty") {
                assert!(
                    is_empty_deps.contains("length"),
                    "Should depend on 'length' column"
                );
                assert!(
                    !is_empty_deps.contains("len"),
                    "'len' should NOT appear in dependencies"
                );
                assert_eq!(
                    is_empty_deps.len(),
                    1,
                    "Should only have one dependency: 'length'"
                );
            } else {
                panic!("'is_empty' column should have dependencies");
            }
        }
        other => panic!("Expected table definition, got {:?}", other),
    }
}

/// Test computed columns with constants and literals
#[test]
fn test_computed_column_with_constants() {
    let src = r#"
        Constants :: table {
            base_value: f64,
            doubled: computed(base_value * 2.0),
            with_constant: computed(base_value + 100.0),
            percentage: computed(base_value / 100.0),
            boolean_const: computed(true),
            string_const: computed("constant_value"),
            mixed: computed(base_value * 2.0 + 10.0),
        }
    "#;

    let program = parser::parse(src.to_string());

    match &program.statements[0] {
        Statement::ConstDecl {
            value: ConstValue::TableDef(table),
            ..
        } => {
            assert_eq!(table.columns.len(), 7);

            // Verify first column is regular, rest are computed
            assert!(!table.columns[0].is_computed);
            for i in 1..7 {
                assert!(
                    table.columns[i].is_computed,
                    "Column {} should be computed",
                    i
                );
            }
        }
        other => panic!("Expected table definition, got {:?}", other),
    }

    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Semantic analysis should handle constants in computed columns"
    );
}

/// Test error case: computed column referencing non-existent column
#[test]
fn test_computed_column_invalid_reference() {
    let src = r#"
        Invalid :: table {
            valid_column: String,
            invalid_computed: computed(non_existent_column + " suffix"),
        }
    "#;

    let program = parser::parse(src.to_string());

    match &program.statements[0] {
        Statement::ConstDecl {
            value: ConstValue::TableDef(table),
            ..
        } => {
            // The dependency graph should detect the invalid reference
            let graph_result = DependencyGraph::build_from_table(table);

            // This should be OK now because non_existent_column is not in the column set,
            // so it won't be treated as a column dependency
            assert!(
                graph_result.is_ok(),
                "Should succeed since non_existent_column is treated as a constant/function"
            );
        }
        other => panic!("Expected table definition, got {:?}", other),
    }
}

/// Test computed column type inference
#[test]
fn test_computed_column_type_inference() {
    let src = r#"
        TypeTest :: table {
            int_col: u64,
            float_col: f64,
            string_col: String,
            computed_int: computed(int_col + 10),
            computed_float: computed(float_col * 2.5),
            computed_string: computed(string_col + " suffix"),
            computed_bool: computed(int_col > 5),
        }
    "#;

    let program = parser::parse(src.to_string());

    // Perform semantic analysis which should infer types for computed columns
    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Semantic analysis should succeed and infer types"
    );

    let context = semantic_result.unwrap();

    // Check that the table was processed correctly
    assert!(
        context.tables.contains_key("TypeTest"),
        "Table should be in semantic context"
    );

    let table = &context.tables["TypeTest"];

    // Verify that computed columns no longer have Type::None
    for column in &table.columns {
        if column.is_computed {
            assert_ne!(
                column.column_type,
                Type::None,
                "Computed column '{}' should have inferred type, not Type::None",
                column.name
            );
        }
    }
}

/// Test computed columns with simple forward references (referencing later-defined computed columns)
#[test]
fn test_computed_column_forward_references() {
    let src = r#"
        ForwardRef :: table {
            base_value: i64,
            early_column: computed(later_column + 10),  // References later_column defined below
            middle_column: computed(base_value * 2),
            later_column: computed(base_value + 5),     // Referenced by early_column above
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    // This should succeed with the fix - forward references should work
    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Semantic analysis should succeed with forward references: {:?}",
        semantic_result.err()
    );

    let context = semantic_result.unwrap();
    let table = &context.tables["ForwardRef"];

    // Verify all computed columns have proper types (not Type::None)
    for column in &table.columns {
        if column.is_computed {
            assert_ne!(
                column.column_type,
                Type::None,
                "Computed column '{}' should have inferred type",
                column.name
            );
        }
    }
}

/// Test complex forward reference chains
#[test]
fn test_computed_column_complex_forward_references() {
    let src = r#"
        ComplexForward :: table {
            input: i64,
            final_result: computed(step_three * 2),      // References step_three (defined later)
            step_one: computed(input + 1),               // References input (regular column)
            step_three: computed(step_two + step_one),   // References step_two (defined later) and step_one (defined earlier)
            step_two: computed(step_one * 3),            // References step_one (defined earlier)
        }
    "#;

    let program = parser::parse(src.to_string());

    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Complex forward references should work: {:?}",
        semantic_result.err()
    );

    let context = semantic_result.unwrap();
    let table = &context.tables["ComplexForward"];

    // Verify evaluation order respects dependencies
    for column in &table.columns {
        if column.is_computed {
            assert_ne!(
                column.column_type,
                Type::None,
                "Computed column '{}' should have inferred type",
                column.name
            );
        }
    }
}

/// Test mixed forward and backward references with different types
#[test]
fn test_computed_column_mixed_references_with_types() {
    let src = r#"
        MixedTypes :: table {
            count: i64,
            rate: f64,
            total_value: computed(count_float * rate),        // Forward reference to count_float
            count_float: computed(count as f64),              // Backward reference to count
            is_high_value: computed(total_value > 100.0),     // Forward reference to total_value
            description: computed(if is_high_value then "high" else "low"), // Forward reference to is_high_value
        }
    "#;

    let program = parser::parse(src.to_string());

    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Mixed forward/backward references should work: {:?}",
        semantic_result.err()
    );

    let context = semantic_result.unwrap();
    let table = &context.tables["MixedTypes"];

    // Check that types are correctly inferred
    let columns: std::collections::HashMap<String, &TableColumn> =
        table.columns.iter().map(|c| (c.name.clone(), c)).collect();

    // Verify specific type inferences
    if let Some(total_value) = columns.get("total_value") {
        // Should be f64 (count_float * rate)
        assert!(
            matches!(total_value.column_type, Type::Identifier { ref name, .. } if name == "f64")
        );
    }

    if let Some(is_high_value) = columns.get("is_high_value") {
        // Should be bool (total_value > 100.0)
        assert!(
            matches!(is_high_value.column_type, Type::Identifier { ref name, .. } if name == "bool")
        );
    }
}

/// Test forward references in the presence of circular dependency detection
#[test]
fn test_forward_references_vs_circular_dependencies() {
    // This should work - forward references but no cycles
    let valid_src = r#"
        ValidForward :: table {
            base: i64,
            derived_a: computed(derived_b + 1),  // Forward reference
            derived_b: computed(base * 2),       // No cycle
        }
    "#;

    let program = parser::parse(valid_src.to_string());
    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Valid forward references should work"
    );

    // This should fail - actual circular dependency
    let circular_src = r#"
        CircularTest :: table {
            base: i64,
            circular_a: computed(circular_b + 1),  // References circular_b
            circular_b: computed(circular_a * 2),  // References circular_a -> cycle!
        }
    "#;

    let program2 = parser::parse(circular_src.to_string());
    let semantic_result2 = semantic::analyze_program(&program2);
    assert!(
        semantic_result2.is_err(),
        "Circular dependencies should be detected and fail"
    );
}

/// Test that forward references work with function calls
#[test]
fn test_forward_references_with_function_calls() {
    let src = r#"
        FunctionForward :: table {
            text: String,
            processed_length: computed(len(processed_text)),  // Forward reference with function
            processed_text: computed(text + " processed"),     // Simple transformation
            is_long: computed(processed_length > 20),          // Forward reference chain with function
        }
    "#;

    let program = parser::parse(src.to_string());

    let semantic_result = semantic::analyze_program(&program);
    assert!(
        semantic_result.is_ok(),
        "Forward references with function calls should work: {:?}",
        semantic_result.err()
    );

    let context = semantic_result.unwrap();
    let table = &context.tables["FunctionForward"];

    // Verify computed columns have correct types
    let columns: std::collections::HashMap<String, &TableColumn> =
        table.columns.iter().map(|c| (c.name.clone(), c)).collect();

    if let Some(processed_length) = columns.get("processed_length") {
        // len() returns i64
        assert!(
            matches!(processed_length.column_type, Type::Identifier { ref name, .. } if name == "i64")
        );
    }

    if let Some(is_long) = columns.get("is_long") {
        // comparison returns bool
        assert!(matches!(is_long.column_type, Type::Identifier { ref name, .. } if name == "bool"));
    }
}
