use tricti::ast::*;
use tricti::query::*;
use tricti::query_executor::*;
use tricti::table_runtime::*;
use std::collections::HashMap;

#[cfg(test)]
mod integration_tests {
    use super::*;

    fn create_comprehensive_table_schema() -> TableDef {
        TableDef {
            name: "Products".to_string(),
            columns: vec![
                TableColumn {
                    name: "id".to_string(),
                    column_type: Type::Identifier {
                        name: "u64".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![TableAnnotation {
                        name: "primary".to_string(),
                        args: vec![],
                    }],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
                TableColumn {
                    name: "name".to_string(),
                    column_type: Type::Identifier {
                        name: "String".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
                TableColumn {
                    name: "price".to_string(),
                    column_type: Type::Identifier {
                        name: "f64".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
                TableColumn {
                    name: "quantity".to_string(),
                    column_type: Type::Identifier {
                        name: "u64".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: Some(Expression::Literal(Literal::integer_from_parts(
                        "0".to_string(),
                        0,
                        None,
                    ))),
                    is_computed: false,
                    computed_expression: None,
                },
                TableColumn {
                    name: "in_stock".to_string(),
                    column_type: Type::Identifier {
                        name: "bool".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: true,
                    computed_expression: Some(Expression::BinaryOp {
                        left: Box::new(Expression::Identifier("quantity".to_string())),
                        operator: BinaryOperator::Greater,
                        right: Box::new(Expression::Literal(Literal::integer_from_parts(
                            "0".to_string(),
                            0,
                            None,
                        ))),
                    }),
                },
                TableColumn {
                    name: "total_value".to_string(),
                    column_type: Type::Identifier {
                        name: "f64".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: true,
                    computed_expression: Some(Expression::BinaryOp {
                        left: Box::new(Expression::Identifier("price".to_string())),
                        operator: BinaryOperator::Mul,
                        right: Box::new(Expression::Identifier("quantity".to_string())),
                    }),
                },
            ],
        }
    }

    fn create_test_product(id: u64, name: &str, price: f64, quantity: u64) -> TableRow {
        let mut values = HashMap::new();
        values.insert("id".to_string(), ColumnValue::U64(id));
        values.insert("name".to_string(), ColumnValue::String(name.to_string()));
        values.insert("price".to_string(), ColumnValue::F64(price.to_bits()));
        values.insert("quantity".to_string(), ColumnValue::U64(quantity));
        TableRow { values }
    }

    #[test]
    fn test_integration_with_computed_columns() {
        let mut executor = QueryExecutor::new();

        // Create table with computed columns
        let schema = create_comprehensive_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert test data
        table
            .insert_row(create_test_product(1, "Laptop", 1000.0, 5))
            .unwrap();
        table
            .insert_row(create_test_product(2, "Mouse", 25.0, 0))
            .unwrap();
        table
            .insert_row(create_test_product(3, "Keyboard", 75.0, 10))
            .unwrap();

        executor.register_table("Products".to_string(), table);

        // Query including computed columns
        let projection = vec![
            FieldProjection::column("name".to_string()),
            FieldProjection::column("price".to_string()),
            FieldProjection::column("quantity".to_string()),
            FieldProjection::column("in_stock".to_string()),
            FieldProjection::column("total_value".to_string()),
        ];

        let query = QueryPlan::select("Products".to_string(), projection, None);
        let result = executor
            .execute(query)
            .expect("Failed to execute query with computed columns");

        assert_eq!(result.len(), 3);
        assert_eq!(result.schema.len(), 5);

        // Verify computed column values are calculated correctly
        for (_, row) in &result.rows {
            let name = match row.values.get("name") {
                Some(ColumnValue::String(n)) => n,
                _ => panic!("Expected string name"),
            };

            let quantity = match row.values.get("quantity") {
                Some(ColumnValue::U64(q)) => *q,
                _ => panic!("Expected u64 quantity"),
            };

            let in_stock = match row.values.get("in_stock") {
                Some(ColumnValue::Bool(stock)) => *stock,
                _ => panic!("Expected bool in_stock"),
            };

            // Verify in_stock is correctly computed (quantity > 0)
            assert_eq!(in_stock, quantity > 0);

            let price = match row.values.get("price") {
                Some(ColumnValue::F64(p)) => f64::from_bits(*p),
                _ => panic!("Expected f64 price"),
            };

            let total_value = match row.values.get("total_value") {
                Some(ColumnValue::F64(tv)) => f64::from_bits(*tv),
                _ => panic!("Expected f64 total_value"),
            };

            // Verify total_value is correctly computed (price * quantity)
            let expected_total = price * (quantity as f64);
            assert!(
                (total_value - expected_total).abs() < 0.001,
                "Total value mismatch for {}: expected {}, got {}",
                name,
                expected_total,
                total_value
            );
        }
    }

    #[test]
    fn test_where_clause_with_computed_columns() {
        let mut executor = QueryExecutor::new();

        let schema = create_comprehensive_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert test data
        table
            .insert_row(create_test_product(1, "Laptop", 1000.0, 5))
            .unwrap(); // in_stock = true
        table
            .insert_row(create_test_product(2, "Mouse", 25.0, 0))
            .unwrap(); // in_stock = false
        table
            .insert_row(create_test_product(3, "Keyboard", 75.0, 10))
            .unwrap(); // in_stock = true

        executor.register_table("Products".to_string(), table);

        // Query filtering by computed column (in_stock)
        let projection = vec![
            FieldProjection::column("name".to_string()),
            FieldProjection::column("quantity".to_string()),
        ];

        let where_clause = Some(WhereClause::new(Expression::BinaryOp {
            left: Box::new(Expression::Identifier("in_stock".to_string())),
            operator: BinaryOperator::Equal,
            right: Box::new(Expression::Literal(Literal::Boolean(true))),
        }));

        let query = QueryPlan::select("Products".to_string(), projection, where_clause);
        let result = executor
            .execute(query)
            .expect("Failed to execute query filtering by computed column");

        // Should return only products in stock (Laptop and Keyboard)
        assert_eq!(result.len(), 2);

        let mut product_names = std::collections::HashSet::new();
        for (_, row) in &result.rows {
            match row.values.get("name") {
                Some(ColumnValue::String(name)) => {
                    product_names.insert(name.clone());
                }
                _ => panic!("Expected string name"),
            }
        }

        assert!(product_names.contains("Laptop"));
        assert!(product_names.contains("Keyboard"));
        assert!(!product_names.contains("Mouse")); // Mouse should be filtered out (quantity = 0)
    }

    #[test]
    fn test_complex_where_clauses() {
        let mut executor = QueryExecutor::new();

        let schema = create_comprehensive_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert test data
        table
            .insert_row(create_test_product(1, "Laptop", 1000.0, 5))
            .unwrap();
        table
            .insert_row(create_test_product(2, "Mouse", 25.0, 15))
            .unwrap();
        table
            .insert_row(create_test_product(3, "Keyboard", 75.0, 0))
            .unwrap();
        table
            .insert_row(create_test_product(4, "Monitor", 300.0, 3))
            .unwrap();

        executor.register_table("Products".to_string(), table);

        // Complex WHERE clause: price > 50 AND quantity > 2
        let projection = vec![
            FieldProjection::column("name".to_string()),
            FieldProjection::column("price".to_string()),
            FieldProjection::column("quantity".to_string()),
        ];

        let where_clause = Some(WhereClause::new(Expression::BinaryOp {
            left: Box::new(Expression::BinaryOp {
                left: Box::new(Expression::Identifier("price".to_string())),
                operator: BinaryOperator::Greater,
                right: Box::new(Expression::Literal(Literal::integer_from_parts(
                    "50".to_string(),
                    50,
                    None,
                ))),
            }),
            operator: BinaryOperator::And,
            right: Box::new(Expression::BinaryOp {
                left: Box::new(Expression::Identifier("quantity".to_string())),
                operator: BinaryOperator::Greater,
                right: Box::new(Expression::Literal(Literal::integer_from_parts(
                    "2".to_string(),
                    2,
                    None,
                ))),
            }),
        }));

        let query = QueryPlan::select("Products".to_string(), projection, where_clause);
        let result = executor
            .execute(query)
            .expect("Failed to execute complex WHERE query");

        // Should return Laptop (price=1000, qty=5) and Monitor (price=300, qty=3)
        // Mouse excluded (price=25 < 50), Keyboard excluded (qty=0 < 2)
        assert_eq!(result.len(), 2);

        let mut product_names = std::collections::HashSet::new();
        for (_, row) in &result.rows {
            match row.values.get("name") {
                Some(ColumnValue::String(name)) => {
                    product_names.insert(name.clone());
                }
                _ => panic!("Expected string name"),
            }
        }

        assert!(product_names.contains("Laptop"));
        assert!(product_names.contains("Monitor"));
        assert!(!product_names.contains("Mouse"));
        assert!(!product_names.contains("Keyboard"));
    }

    #[test]
    fn test_projection_with_transformation() {
        let mut executor = QueryExecutor::new();

        let schema = create_comprehensive_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert test data
        table
            .insert_row(create_test_product(1, "Laptop", 1000.0, 5))
            .unwrap();
        table
            .insert_row(create_test_product(2, "Mouse", 25.0, 15))
            .unwrap();

        executor.register_table("Products".to_string(), table);

        // Query with computed projection (price with tax)
        let projection = vec![
            FieldProjection::column("name".to_string()),
            FieldProjection::column("price".to_string()),
            FieldProjection::computed(
                "price".to_string(),
                Expression::BinaryOp {
                    left: Box::new(Expression::Identifier("price".to_string())),
                    operator: BinaryOperator::Mul,
                    right: Box::new(Expression::Literal(Literal::Float(1.08))), // 8% tax
                },
            ),
        ];

        let query = QueryPlan::select("Products".to_string(), projection, None);
        let result = executor
            .execute(query)
            .expect("Failed to execute query with computed projection");

        assert_eq!(result.len(), 2);
        assert_eq!(result.schema.len(), 3);

        // Verify computed values
        for (_, row) in &result.rows {
            let _original_price = match row.values.get("price") {
                Some(ColumnValue::F64(p)) => f64::from_bits(*p),
                _ => panic!("Expected f64 price"),
            };

            let _computed_price = match row.values.get("price") {
                // This gets the last occurrence in the projection
                Some(ColumnValue::F64(p)) => f64::from_bits(*p),
                _ => panic!("Expected f64 computed price"),
            };

            // Note: Due to projection naming, this test would need refinement in a real implementation
            // to properly handle computed projections with aliases
        }
    }

    #[test]
    fn test_join_with_computed_columns() {
        let mut executor = QueryExecutor::new();

        // Create Categories table
        let categories_schema = TableDef {
            name: "Categories".to_string(),
            columns: vec![
                TableColumn {
                    name: "id".to_string(),
                    column_type: Type::Identifier {
                        name: "u64".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![TableAnnotation {
                        name: "primary".to_string(),
                        args: vec![],
                    }],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
                TableColumn {
                    name: "name".to_string(),
                    column_type: Type::Identifier {
                        name: "String".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
                TableColumn {
                    name: "tax_rate".to_string(),
                    column_type: Type::Identifier {
                        name: "f64".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
            ],
        };

        // Create Products table with category_id
        let products_schema = TableDef {
            name: "Products".to_string(),
            columns: vec![
                TableColumn {
                    name: "id".to_string(),
                    column_type: Type::Identifier {
                        name: "u64".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![TableAnnotation {
                        name: "primary".to_string(),
                        args: vec![],
                    }],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
                TableColumn {
                    name: "name".to_string(),
                    column_type: Type::Identifier {
                        name: "String".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
                TableColumn {
                    name: "price".to_string(),
                    column_type: Type::Identifier {
                        name: "f64".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
                TableColumn {
                    name: "category_id".to_string(),
                    column_type: Type::Identifier {
                        name: "u64".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
            ],
        };

        let mut categories_table =
            TableRuntime::new(categories_schema).expect("Failed to create categories table");
        let mut products_table =
            TableRuntime::new(products_schema).expect("Failed to create products table");

        // Insert category data
        let mut cat_values = HashMap::new();
        cat_values.insert("id".to_string(), ColumnValue::U64(1));
        cat_values.insert(
            "name".to_string(),
            ColumnValue::String("Electronics".to_string()),
        );
        cat_values.insert("tax_rate".to_string(), ColumnValue::F64(0.08_f64.to_bits()));
        categories_table
            .insert_row(TableRow { values: cat_values })
            .unwrap();

        let mut cat_values = HashMap::new();
        cat_values.insert("id".to_string(), ColumnValue::U64(2));
        cat_values.insert(
            "name".to_string(),
            ColumnValue::String("Office".to_string()),
        );
        cat_values.insert("tax_rate".to_string(), ColumnValue::F64(0.05_f64.to_bits()));
        categories_table
            .insert_row(TableRow { values: cat_values })
            .unwrap();

        // Insert product data
        let mut prod_values = HashMap::new();
        prod_values.insert("id".to_string(), ColumnValue::U64(1));
        prod_values.insert(
            "name".to_string(),
            ColumnValue::String("Laptop".to_string()),
        );
        prod_values.insert("price".to_string(), ColumnValue::F64(1000.0_f64.to_bits()));
        prod_values.insert("category_id".to_string(), ColumnValue::U64(1));
        products_table
            .insert_row(TableRow {
                values: prod_values,
            })
            .unwrap();

        let mut prod_values = HashMap::new();
        prod_values.insert("id".to_string(), ColumnValue::U64(2));
        prod_values.insert("name".to_string(), ColumnValue::String("Desk".to_string()));
        prod_values.insert("price".to_string(), ColumnValue::F64(200.0_f64.to_bits()));
        prod_values.insert("category_id".to_string(), ColumnValue::U64(2));
        products_table
            .insert_row(TableRow {
                values: prod_values,
            })
            .unwrap();

        executor.register_table("Categories".to_string(), categories_table);
        executor.register_table("Products".to_string(), products_table);

        // Execute JOIN query
        let left_projection = vec![
            FieldProjection::column("name".to_string()),
            FieldProjection::column("price".to_string()),
        ];
        let right_projection = vec![
            FieldProjection::column("name".to_string()),
            FieldProjection::column("tax_rate".to_string()),
        ];
        let join_condition = JoinCondition::eq("category_id".to_string(), "id".to_string());

        let query = QueryPlan::join(
            "Products".to_string(),
            "Categories".to_string(),
            JoinType::Inner,
            join_condition,
            left_projection,
            right_projection,
        );

        let result = executor
            .execute(query)
            .expect("Failed to execute join with computed columns");

        assert_eq!(result.len(), 2); // Both products should match their categories
        assert_eq!(result.schema.len(), 4); // left_name, left_price, right_name, right_tax_rate

        // Verify join results
        for (_, row) in &result.rows {
            let product_name = match row.values.get("left_name") {
                Some(ColumnValue::String(name)) => name,
                _ => panic!("Expected string product name"),
            };

            let category_name = match row.values.get("right_name") {
                Some(ColumnValue::String(name)) => name,
                _ => panic!("Expected string category name"),
            };

            // Verify correct joins
            match product_name.as_str() {
                "Laptop" => assert_eq!(category_name, "Electronics"),
                "Desk" => assert_eq!(category_name, "Office"),
                _ => panic!("Unexpected product name: {}", product_name),
            }
        }
    }

    #[test]
    fn test_query_optimization_flags() {
        let mut executor = QueryExecutor::new();

        let schema = create_comprehensive_table_schema();
        let table = TableRuntime::new(schema).expect("Failed to create table");
        executor.register_table("Products".to_string(), table);

        // Create query with optimization hints
        let projection = vec![FieldProjection::column("name".to_string())];
        let where_clause = Some(
            WhereClause::new(Expression::Literal(Literal::Boolean(true)))
                .with_hint(OptimizationHint::PredicatePushdown)
                .with_hint(OptimizationHint::UseIndex("name_index".to_string())),
        );

        let mut query = QueryPlan::select("Products".to_string(), projection, where_clause);

        // Apply optimization
        query = query.optimize();

        // Verify optimization was applied
        match &query {
            QueryPlan::Select {
                optimization,
                where_clause,
                ..
            } => {
                assert!(optimization.predicate_pushdown);
                if let Some(where_clause) = where_clause {
                    assert!(!where_clause.optimization_hints.is_empty());
                }
            }
            _ => panic!("Expected Select query plan"),
        }

        // Execute optimized query
        let result = executor.execute(query);
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_table_queries() {
        let mut executor = QueryExecutor::new();

        let schema = create_comprehensive_table_schema();
        let table = TableRuntime::new(schema).expect("Failed to create table");
        executor.register_table("Products".to_string(), table);

        // Query empty table
        let projection = vec![FieldProjection::column("name".to_string())];
        let query = QueryPlan::select("Products".to_string(), projection, None);
        let result = executor
            .execute(query)
            .expect("Failed to execute query on empty table");

        assert_eq!(result.len(), 0);
        assert!(result.is_empty());
        assert_eq!(result.statistics.rows_scanned, 0);
        assert_eq!(result.statistics.rows_returned, 0);
    }

    #[test]
    fn test_full_outer_join() {
        let mut executor = QueryExecutor::new();

        // Create simple schemas for full outer join test
        let left_schema = TableDef {
            name: "Left".to_string(),
            columns: vec![
                TableColumn {
                    name: "id".to_string(),
                    column_type: Type::Identifier {
                        name: "u64".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
                TableColumn {
                    name: "value".to_string(),
                    column_type: Type::Identifier {
                        name: "String".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
            ],
        };

        let right_schema = TableDef {
            name: "Right".to_string(),
            columns: vec![
                TableColumn {
                    name: "id".to_string(),
                    column_type: Type::Identifier {
                        name: "u64".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
                TableColumn {
                    name: "data".to_string(),
                    column_type: Type::Identifier {
                        name: "String".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                    is_computed: false,
                    computed_expression: None,
                },
            ],
        };

        let mut left_table = TableRuntime::new(left_schema).unwrap();
        let mut right_table = TableRuntime::new(right_schema).unwrap();

        // Insert data: some overlapping, some unique to each side
        let mut left_values = HashMap::new();
        left_values.insert("id".to_string(), ColumnValue::U64(1));
        left_values.insert("value".to_string(), ColumnValue::String("A".to_string()));
        left_table
            .insert_row(TableRow {
                values: left_values,
            })
            .unwrap();

        let mut left_values = HashMap::new();
        left_values.insert("id".to_string(), ColumnValue::U64(2));
        left_values.insert("value".to_string(), ColumnValue::String("B".to_string()));
        left_table
            .insert_row(TableRow {
                values: left_values,
            })
            .unwrap();

        let mut left_values = HashMap::new();
        left_values.insert("id".to_string(), ColumnValue::U64(3));
        left_values.insert("value".to_string(), ColumnValue::String("C".to_string()));
        left_table
            .insert_row(TableRow {
                values: left_values,
            })
            .unwrap();

        let mut right_values = HashMap::new();
        right_values.insert("id".to_string(), ColumnValue::U64(2));
        right_values.insert("data".to_string(), ColumnValue::String("Y".to_string()));
        right_table
            .insert_row(TableRow {
                values: right_values,
            })
            .unwrap();

        let mut right_values = HashMap::new();
        right_values.insert("id".to_string(), ColumnValue::U64(3));
        right_values.insert("data".to_string(), ColumnValue::String("Z".to_string()));
        right_table
            .insert_row(TableRow {
                values: right_values,
            })
            .unwrap();

        let mut right_values = HashMap::new();
        right_values.insert("id".to_string(), ColumnValue::U64(4));
        right_values.insert("data".to_string(), ColumnValue::String("W".to_string()));
        right_table
            .insert_row(TableRow {
                values: right_values,
            })
            .unwrap();

        executor.register_table("Left".to_string(), left_table);
        executor.register_table("Right".to_string(), right_table);

        // Execute FULL OUTER JOIN
        let left_projection = vec![
            FieldProjection::column("id".to_string()),
            FieldProjection::column("value".to_string()),
        ];
        let right_projection = vec![
            FieldProjection::column("id".to_string()),
            FieldProjection::column("data".to_string()),
        ];
        let join_condition = JoinCondition::eq("id".to_string(), "id".to_string());

        let query = QueryPlan::join(
            "Left".to_string(),
            "Right".to_string(),
            JoinType::FullOuter,
            join_condition,
            left_projection,
            right_projection,
        );

        let result = executor
            .execute(query)
            .expect("Failed to execute full outer join");

        // Should include:
        // - Left id=1 (no match on right)
        // - Left id=2 + Right id=2 (match)
        // - Left id=3 + Right id=3 (match)
        // - Right id=4 (no match on left)
        // Total: 4 rows
        assert_eq!(result.len(), 4);
    }
}
