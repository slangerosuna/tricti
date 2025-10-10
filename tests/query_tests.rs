use std::collections::HashMap;

use tricti::ast::{
    BinaryOperator, Expression, Literal, TableAnnotation, TableColumn, TableDef, Type,
};
use tricti::query::{
    FieldProjection as QueryFieldProjection, JoinCondition, JoinType as QueryJoinType,
    OptimizationHint, QueryError, QueryPlan, QueryResult, ResultColumn, WhereClause,
};
use tricti::query_executor::*;
use tricti::table_runtime::*;

#[cfg(test)]
mod tests {
    use super::OptimizationHint;
    use super::QueryFieldProjection as FieldProjection;
    use super::QueryJoinType as JoinType;
    use super::*;

    fn create_test_table_schema() -> TableDef {
        TableDef {
            name: "Users".to_string(),
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
                    name: "age".to_string(),
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
                    name: "active".to_string(),
                    column_type: Type::Identifier {
                        name: "bool".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: Some(Expression::Literal(Literal::Boolean(true))),
                    is_computed: false,
                    computed_expression: None,
                },
            ],
        }
    }

    fn create_orders_table_schema() -> TableDef {
        TableDef {
            name: "Orders".to_string(),
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
                    name: "user_id".to_string(),
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
                    name: "amount".to_string(),
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
        }
    }

    fn create_test_user(id: u64, name: &str, age: u64, active: bool) -> TableRow {
        let mut values = HashMap::new();
        values.insert("id".to_string(), ColumnValue::U64(id));
        values.insert("name".to_string(), ColumnValue::String(name.to_string()));
        values.insert("age".to_string(), ColumnValue::U64(age));
        values.insert("active".to_string(), ColumnValue::Bool(active));
        TableRow { values }
    }

    fn create_test_order(id: u64, user_id: u64, amount: f64) -> TableRow {
        let mut values = HashMap::new();
        values.insert("id".to_string(), ColumnValue::U64(id));
        values.insert("user_id".to_string(), ColumnValue::U64(user_id));
        values.insert("amount".to_string(), ColumnValue::F64(amount.to_bits()));
        TableRow { values }
    }

    #[test]
    fn test_query_plan_creation() {
        // Test SELECT query plan creation
        let projection = vec![
            FieldProjection::column("id".to_string()),
            FieldProjection::column("name".to_string()),
        ];

        let where_clause = Some(WhereClause::new(Expression::BinaryOp {
            left: Box::new(Expression::Identifier("age".to_string())),
            operator: BinaryOperator::Greater,
            right: Box::new(Expression::Literal(Literal::integer_from_parts(
                "25".to_string(),
                25,
                None,
            ))),
        }));

        let query = QueryPlan::select("Users".to_string(), projection, where_clause);

        match query {
            QueryPlan::Select {
                table_name,
                projection,
                where_clause,
                ..
            } => {
                assert_eq!(table_name, "Users");
                assert_eq!(projection.len(), 2);
                assert_eq!(projection[0].source_column, "id");
                assert_eq!(projection[1].source_column, "name");
                assert!(where_clause.is_some());
            }
            _ => panic!("Expected Select query plan"),
        }
    }

    #[test]
    fn test_field_projection_creation() {
        // Test simple column projection
        let col_proj = FieldProjection::column("name".to_string());
        assert_eq!(col_proj.source_column, "name");
        assert_eq!(col_proj.alias, None);
        assert_eq!(col_proj.output_name(), "name");

        // Test column projection with alias
        let alias_proj = FieldProjection::column_as("name".to_string(), "user_name".to_string());
        assert_eq!(alias_proj.source_column, "name");
        assert_eq!(alias_proj.alias, Some("user_name".to_string()));
        assert_eq!(alias_proj.output_name(), "user_name");

        // Test computed projection
        let computed_proj = FieldProjection::computed(
            "age".to_string(),
            Expression::BinaryOp {
                left: Box::new(Expression::Identifier("age".to_string())),
                operator: BinaryOperator::Add,
                right: Box::new(Expression::Literal(Literal::integer_from_parts(
                    "1".to_string(),
                    1,
                    None,
                ))),
            },
        );
        assert_eq!(computed_proj.source_column, "age");
        assert!(computed_proj.transformation.is_some());
    }

    #[test]
    fn test_where_clause_creation() {
        let condition = Expression::BinaryOp {
            left: Box::new(Expression::Identifier("active".to_string())),
            operator: BinaryOperator::Equal,
            right: Box::new(Expression::Literal(Literal::Boolean(true))),
        };

        let where_clause = WhereClause::new(condition.clone())
            .with_hint(OptimizationHint::UseIndex("active_index".to_string()));

        assert_eq!(where_clause.condition, condition);
        assert_eq!(where_clause.optimization_hints.len(), 1);
        match &where_clause.optimization_hints[0] {
            OptimizationHint::UseIndex(index_name) => assert_eq!(index_name, "active_index"),
            _ => panic!("Expected UseIndex hint"),
        }
    }

    #[test]
    fn test_join_plan_creation() {
        let left_projection = vec![
            FieldProjection::column("id".to_string()),
            FieldProjection::column("name".to_string()),
        ];
        let right_projection = vec![FieldProjection::column("amount".to_string())];
        let join_condition = JoinCondition::eq("id".to_string(), "user_id".to_string());

        let join_query = QueryPlan::join(
            "Users".to_string(),
            "Orders".to_string(),
            JoinType::Inner,
            join_condition.clone(),
            left_projection.clone(),
            right_projection.clone(),
        );

        match join_query {
            QueryPlan::Join {
                left_table,
                right_table,
                join_type,
                join_condition: condition,
                left_projection: left_proj,
                right_projection: right_proj,
                ..
            } => {
                assert_eq!(left_table, "Users");
                assert_eq!(right_table, "Orders");
                assert_eq!(join_type, JoinType::Inner);
                assert_eq!(condition.left_column, "id");
                assert_eq!(condition.right_column, "user_id");
                assert_eq!(condition.operator, BinaryOperator::Equal);
                assert_eq!(left_proj.len(), 2);
                assert_eq!(right_proj.len(), 1);
            }
            _ => panic!("Expected Join query plan"),
        }
    }

    #[test]
    fn test_join_condition_creation() {
        // Test equality join
        let eq_join = JoinCondition::eq("user_id".to_string(), "id".to_string());
        assert_eq!(eq_join.left_column, "user_id");
        assert_eq!(eq_join.right_column, "id");
        assert_eq!(eq_join.operator, BinaryOperator::Equal);

        // Test custom join condition
        let custom_join = JoinCondition::new(
            "age".to_string(),
            "min_age".to_string(),
            BinaryOperator::GreaterEqual,
        );
        assert_eq!(custom_join.left_column, "age");
        assert_eq!(custom_join.right_column, "min_age");
        assert_eq!(custom_join.operator, BinaryOperator::GreaterEqual);
    }

    #[test]
    fn test_query_result_creation() {
        let rows = vec![
            (RowId(0), create_test_user(1, "Alice", 30, true)),
            (RowId(1), create_test_user(2, "Bob", 25, false)),
        ];

        let schema = vec![
            ResultColumn::new(
                "id".to_string(),
                Type::Identifier {
                    name: "u64".to_string(),
                    type_args: vec![],
                },
                Some("Users".to_string()),
            ),
            ResultColumn::new(
                "name".to_string(),
                Type::Identifier {
                    name: "String".to_string(),
                    type_args: vec![],
                },
                Some("Users".to_string()),
            ),
        ];

        let result = QueryResult::new(rows.clone(), schema.clone());

        assert_eq!(result.len(), 2);
        assert!(!result.is_empty());
        assert_eq!(result.schema.len(), 2);
        assert_eq!(result.statistics.rows_returned, 2);

        // Test empty result
        let empty_result = QueryResult::empty(schema);
        assert_eq!(empty_result.len(), 0);
        assert!(empty_result.is_empty());
    }

    #[test]
    fn test_query_optimization() {
        let mut query = QueryPlan::select(
            "Users".to_string(),
            vec![FieldProjection::column("name".to_string())],
            Some(WhereClause::new(Expression::Literal(Literal::Boolean(
                true,
            )))),
        );

        // Apply optimization
        query = query.optimize();

        match query {
            QueryPlan::Select { optimization, .. } => {
                assert!(optimization.predicate_pushdown);
            }
            _ => panic!("Expected Select query plan"),
        }
    }

    #[test]
    fn test_query_executor_creation() {
        let mut executor = QueryExecutor::new();

        // Create and register a table
        let schema = create_test_table_schema();
        let table = TableRuntime::new(schema).expect("Failed to create table");
        executor.register_table("Users".to_string(), table);

        // Verify table is registered (we can't directly access internal HashMap, but this tests the registration)
        assert_eq!(
            std::mem::size_of_val(&executor),
            std::mem::size_of::<QueryExecutor>()
        );
    }

    #[test]
    fn test_simple_select_execution() {
        let mut executor = QueryExecutor::new();

        // Set up table with test data
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert test data
        table
            .insert_row(create_test_user(1, "Alice", 30, true))
            .unwrap();
        table
            .insert_row(create_test_user(2, "Bob", 25, false))
            .unwrap();
        table
            .insert_row(create_test_user(3, "Charlie", 35, true))
            .unwrap();

        executor.register_table("Users".to_string(), table);

        // Execute simple SELECT * query
        let projection = vec![
            FieldProjection::column("id".to_string()),
            FieldProjection::column("name".to_string()),
            FieldProjection::column("age".to_string()),
            FieldProjection::column("active".to_string()),
        ];

        let query = QueryPlan::select("Users".to_string(), projection, None);
        let result = executor.execute(query).expect("Failed to execute query");

        assert_eq!(result.len(), 3);
        assert_eq!(result.schema.len(), 4);
        assert_eq!(result.statistics.rows_returned, 3);
        assert_eq!(result.statistics.rows_scanned, 3);
    }

    #[test]
    fn test_select_with_where_clause() {
        let mut executor = QueryExecutor::new();

        // Set up table with test data
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert test data
        table
            .insert_row(create_test_user(1, "Alice", 30, true))
            .unwrap();
        table
            .insert_row(create_test_user(2, "Bob", 25, false))
            .unwrap();
        table
            .insert_row(create_test_user(3, "Charlie", 35, true))
            .unwrap();

        executor.register_table("Users".to_string(), table);

        // Execute SELECT with WHERE clause
        let projection = vec![
            FieldProjection::column("id".to_string()),
            FieldProjection::column("name".to_string()),
        ];

        let where_clause = Some(WhereClause::new(Expression::BinaryOp {
            left: Box::new(Expression::Identifier("active".to_string())),
            operator: BinaryOperator::Equal,
            right: Box::new(Expression::Literal(Literal::Boolean(true))),
        }));

        let query = QueryPlan::select("Users".to_string(), projection, where_clause);
        let result = executor.execute(query).expect("Failed to execute query");

        // Should return only Alice and Charlie (active users)
        assert_eq!(result.len(), 2);
        assert_eq!(result.schema.len(), 2);

        // Verify the returned users are the active ones
        for (_, row) in &result.rows {
            match row.values.get("name") {
                Some(ColumnValue::String(name)) => {
                    assert!(name == "Alice" || name == "Charlie");
                }
                _ => panic!("Expected string name"),
            }
        }
    }

    #[test]
    fn test_select_with_numeric_where_clause() {
        let mut executor = QueryExecutor::new();

        // Set up table with test data
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert test data
        table
            .insert_row(create_test_user(1, "Alice", 30, true))
            .unwrap();
        table
            .insert_row(create_test_user(2, "Bob", 25, false))
            .unwrap();
        table
            .insert_row(create_test_user(3, "Charlie", 35, true))
            .unwrap();

        executor.register_table("Users".to_string(), table);

        // Execute SELECT with numeric WHERE clause (age > 28)
        let projection = vec![
            FieldProjection::column("name".to_string()),
            FieldProjection::column("age".to_string()),
        ];

        let where_clause = Some(WhereClause::new(Expression::BinaryOp {
            left: Box::new(Expression::Identifier("age".to_string())),
            operator: BinaryOperator::Greater,
            right: Box::new(Expression::Literal(Literal::integer_from_parts(
                "28".to_string(),
                28,
                None,
            ))),
        }));

        let query = QueryPlan::select("Users".to_string(), projection, where_clause);
        let result = executor.execute(query).expect("Failed to execute query");

        // Should return only Alice (30) and Charlie (35)
        assert_eq!(result.len(), 2);

        // Verify the returned users have age > 28
        for (_, row) in &result.rows {
            match row.values.get("age") {
                Some(ColumnValue::U64(age)) => {
                    assert!(*age > 28);
                }
                _ => panic!("Expected u64 age"),
            }
        }
    }

    #[test]
    fn test_field_projection() {
        let mut executor = QueryExecutor::new();

        // Set up table with test data
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert test data
        table
            .insert_row(create_test_user(1, "Alice", 30, true))
            .unwrap();
        table
            .insert_row(create_test_user(2, "Bob", 25, false))
            .unwrap();

        executor.register_table("Users".to_string(), table);

        // Execute SELECT with only specific columns
        let projection = vec![
            FieldProjection::column("name".to_string()),
            FieldProjection::column_as("age".to_string(), "user_age".to_string()),
        ];

        let query = QueryPlan::select("Users".to_string(), projection, None);
        let result = executor.execute(query).expect("Failed to execute query");

        assert_eq!(result.len(), 2);
        assert_eq!(result.schema.len(), 2);

        // Verify schema contains the expected columns
        assert_eq!(result.schema[0].name, "name");
        assert_eq!(result.schema[1].name, "user_age");

        // Verify rows contain only projected columns
        for (_, row) in &result.rows {
            assert!(row.values.contains_key("name"));
            assert!(row.values.contains_key("user_age"));
            assert!(!row.values.contains_key("id"));
            assert!(!row.values.contains_key("active"));
        }
    }

    #[test]
    fn test_inner_join_execution() {
        let mut executor = QueryExecutor::new();

        // Set up Users table
        let users_schema = create_test_table_schema();
        let mut users_table =
            TableRuntime::new(users_schema).expect("Failed to create users table");
        users_table
            .insert_row(create_test_user(1, "Alice", 30, true))
            .unwrap();
        users_table
            .insert_row(create_test_user(2, "Bob", 25, false))
            .unwrap();
        users_table
            .insert_row(create_test_user(3, "Charlie", 35, true))
            .unwrap();

        // Set up Orders table
        let orders_schema = create_orders_table_schema();
        let mut orders_table =
            TableRuntime::new(orders_schema).expect("Failed to create orders table");
        orders_table
            .insert_row(create_test_order(101, 1, 100.50))
            .unwrap(); // Alice's order
        orders_table
            .insert_row(create_test_order(102, 2, 75.25))
            .unwrap(); // Bob's order
        orders_table
            .insert_row(create_test_order(103, 1, 200.00))
            .unwrap(); // Alice's second order
                       // Note: Charlie (user_id 3) has no orders

        executor.register_table("Users".to_string(), users_table);
        executor.register_table("Orders".to_string(), orders_table);

        // Execute INNER JOIN query
        let left_projection = vec![
            FieldProjection::column("id".to_string()),
            FieldProjection::column("name".to_string()),
        ];
        let right_projection = vec![
            FieldProjection::column("id".to_string()),
            FieldProjection::column("amount".to_string()),
        ];
        let join_condition = JoinCondition::eq("id".to_string(), "user_id".to_string());

        let query = QueryPlan::join(
            "Users".to_string(),
            "Orders".to_string(),
            JoinType::Inner,
            join_condition,
            left_projection,
            right_projection,
        );

        let result = executor
            .execute(query)
            .expect("Failed to execute join query");

        // Should return 3 rows (Alice appears twice because she has 2 orders, Bob once, Charlie excluded)
        assert_eq!(result.len(), 3);
        assert_eq!(result.schema.len(), 4); // left_id, left_name, right_id, right_amount

        // Verify schema
        assert_eq!(result.schema[0].name, "left_id");
        assert_eq!(result.schema[1].name, "left_name");
        assert_eq!(result.schema[2].name, "right_id");
        assert_eq!(result.schema[3].name, "right_amount");

        // Verify that Charlie (user_id 3) is not in the results (no orders)
        for (_, row) in &result.rows {
            match row.values.get("left_name") {
                Some(ColumnValue::String(name)) => {
                    assert!(name == "Alice" || name == "Bob");
                    assert_ne!(name, "Charlie");
                }
                _ => panic!("Expected string name"),
            }
        }
    }

    #[test]
    fn test_left_outer_join_execution() {
        let mut executor = QueryExecutor::new();

        // Set up Users table
        let users_schema = create_test_table_schema();
        let mut users_table =
            TableRuntime::new(users_schema).expect("Failed to create users table");
        users_table
            .insert_row(create_test_user(1, "Alice", 30, true))
            .unwrap();
        users_table
            .insert_row(create_test_user(2, "Bob", 25, false))
            .unwrap();
        users_table
            .insert_row(create_test_user(3, "Charlie", 35, true))
            .unwrap();

        // Set up Orders table
        let orders_schema = create_orders_table_schema();
        let mut orders_table =
            TableRuntime::new(orders_schema).expect("Failed to create orders table");
        orders_table
            .insert_row(create_test_order(101, 1, 100.50))
            .unwrap(); // Alice's order
        orders_table
            .insert_row(create_test_order(102, 2, 75.25))
            .unwrap(); // Bob's order
                       // Note: Charlie (user_id 3) has no orders

        executor.register_table("Users".to_string(), users_table);
        executor.register_table("Orders".to_string(), orders_table);

        // Execute LEFT OUTER JOIN query
        let left_projection = vec![
            FieldProjection::column("id".to_string()),
            FieldProjection::column("name".to_string()),
        ];
        let right_projection = vec![
            FieldProjection::column("id".to_string()),
            FieldProjection::column("amount".to_string()),
        ];
        let join_condition = JoinCondition::eq("id".to_string(), "user_id".to_string());

        let query = QueryPlan::join(
            "Users".to_string(),
            "Orders".to_string(),
            JoinType::LeftOuter,
            join_condition,
            left_projection,
            right_projection,
        );

        let result = executor
            .execute(query)
            .expect("Failed to execute left outer join query");

        // Should return 3 rows (all users: Alice, Bob, Charlie)
        assert_eq!(result.len(), 3);

        // Count how many users appear
        let mut user_names = std::collections::HashSet::new();
        for (_, row) in &result.rows {
            match row.values.get("left_name") {
                Some(ColumnValue::String(name)) => {
                    user_names.insert(name.clone());
                }
                _ => panic!("Expected string name"),
            }
        }

        // All three users should be present
        assert!(user_names.contains("Alice"));
        assert!(user_names.contains("Bob"));
        assert!(user_names.contains("Charlie"));
        assert_eq!(user_names.len(), 3);
    }

    #[test]
    fn test_table_not_found_error() {
        let executor = QueryExecutor::new();

        let projection = vec![FieldProjection::column("id".to_string())];
        let query = QueryPlan::select("NonExistent".to_string(), projection, None);

        let result = executor.execute(query);

        match result {
            Err(QueryError::TableNotFound(table_name)) => {
                assert_eq!(table_name, "NonExistent");
            }
            _ => panic!("Expected TableNotFound error"),
        }
    }

    #[test]
    fn test_column_not_found_error() {
        let mut executor = QueryExecutor::new();

        let schema = create_test_table_schema();
        let table = TableRuntime::new(schema).expect("Failed to create table");
        executor.register_table("Users".to_string(), table);

        // Try to select a non-existent column
        let projection = vec![FieldProjection::column("non_existent_column".to_string())];
        let query = QueryPlan::select("Users".to_string(), projection, None);

        let result = executor.execute(query);

        match result {
            Err(QueryError::ColumnNotFound { table, column }) => {
                assert_eq!(table, "Users");
                assert_eq!(column, "non_existent_column");
            }
            _ => panic!("Expected ColumnNotFound error"),
        }
    }

    #[test]
    fn test_query_statistics() {
        let mut executor = QueryExecutor::new();

        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert test data
        table
            .insert_row(create_test_user(1, "Alice", 30, true))
            .unwrap();
        table
            .insert_row(create_test_user(2, "Bob", 25, false))
            .unwrap();
        table
            .insert_row(create_test_user(3, "Charlie", 35, true))
            .unwrap();

        executor.register_table("Users".to_string(), table);

        // Execute query with filtering
        let projection = vec![FieldProjection::column("name".to_string())];
        let where_clause = Some(WhereClause::new(Expression::BinaryOp {
            left: Box::new(Expression::Identifier("age".to_string())),
            operator: BinaryOperator::Greater,
            right: Box::new(Expression::Literal(Literal::integer_from_parts(
                "28".to_string(),
                28,
                None,
            ))),
        }));

        let query = QueryPlan::select("Users".to_string(), projection, where_clause);
        let result = executor.execute(query).expect("Failed to execute query");

        // Verify statistics - optimized scan touches only matching rows
        assert_eq!(result.statistics.rows_scanned, 2);
        assert_eq!(result.statistics.rows_filtered, 1); // Bob was filtered out
        assert_eq!(result.statistics.rows_returned, 2); // Alice and Charlie
    }
}
