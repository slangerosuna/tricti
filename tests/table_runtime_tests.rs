use peano::ast::*;
use peano::table_runtime::*;
use std::collections::HashMap;

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_table_schema() -> TableDef {
        TableDef {
            name: "Apps".to_string(),
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
                },
                TableColumn {
                    name: "title".to_string(),
                    column_type: Type::Identifier {
                        name: "String".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: None,
                },
                TableColumn {
                    name: "display".to_string(),
                    column_type: Type::Identifier {
                        name: "bool".to_string(),
                        type_args: vec![],
                    },
                    annotations: vec![],
                    default_value: Some(Expression::Literal(Literal::Boolean(false))),
                },
            ],
        }
    }

    fn create_test_row(id: u64, title: &str, display: bool) -> TableRow {
        let mut values = HashMap::new();
        values.insert("id".to_string(), ColumnValue::U64(id));
        values.insert("title".to_string(), ColumnValue::String(title.to_string()));
        values.insert("display".to_string(), ColumnValue::Bool(display));
        TableRow { values }
    }

    #[test]
    fn test_table_creation_with_schema() {
        let schema = create_test_table_schema();
        let table = TableRuntime::new(schema.clone()).expect("Failed to create table");

        assert_eq!(table.schema.name, "Apps");
        assert_eq!(table.schema.columns.len(), 3);
        assert_eq!(table.row_count, 0);

        // Verify primary key detection
        assert_eq!(table.primary_index.column_name, Some("id".to_string()));
    }

    #[test]
    fn test_table_creation_without_primary_key() {
        let mut schema = create_test_table_schema();
        // Remove primary key annotation
        schema.columns[0].annotations.clear();

        let table = TableRuntime::new(schema);
        assert!(
            table.is_ok(),
            "Tables without primary keys should be allowed"
        );

        let table = table.unwrap();
        assert_eq!(table.primary_index.column_name, None);
    }

    #[test]
    fn test_row_insertion_basic() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        let row = create_test_row(1, "Test App", true);
        let row_id = table.insert_row(row).expect("Failed to insert row");

        assert_eq!(table.row_count, 1);
        assert_eq!(row_id, RowId(0));

        // Verify primary key index was updated
        let found_row_id = table.find_by_primary_key(ColumnValue::U64(1));
        assert_eq!(found_row_id, Some(RowId(0)));
    }

    #[test]
    fn test_row_insertion_with_default_values() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Create row without display field (should use default)
        let mut values = HashMap::new();
        values.insert("id".to_string(), ColumnValue::U64(1));
        values.insert("title".to_string(), ColumnValue::String("Test".to_string()));
        let row = TableRow { values };

        let row_id = table.insert_row(row).expect("Failed to insert row");

        // Verify default value was applied
        let retrieved_row = table.get_row(row_id).expect("Failed to get row");
        assert_eq!(
            retrieved_row.values.get("display"),
            Some(&ColumnValue::Bool(false))
        );
    }

    #[test]
    fn test_primary_key_uniqueness_enforcement() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert first row
        let row1 = create_test_row(1, "App 1", true);
        table.insert_row(row1).expect("Failed to insert first row");

        // Try to insert row with same primary key
        let row2 = create_test_row(1, "App 2", false);
        let result = table.insert_row(row2);

        match result {
            Err(TableError::DuplicatePrimaryKey(ColumnValue::U64(1))) => {
                // Expected error
            }
            _ => panic!("Expected DuplicatePrimaryKey error, got: {:?}", result),
        }

        assert_eq!(table.row_count, 1);
    }

    #[test]
    fn test_multiple_row_insertion() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert multiple rows
        let rows = vec![
            create_test_row(1, "App 1", true),
            create_test_row(2, "App 2", false),
            create_test_row(3, "App 3", true),
        ];

        let mut row_ids = Vec::new();
        for row in rows {
            let row_id = table.insert_row(row).expect("Failed to insert row");
            row_ids.push(row_id);
        }

        assert_eq!(table.row_count, 3);
        assert_eq!(row_ids, vec![RowId(0), RowId(1), RowId(2)]);

        // Verify all primary keys are indexed
        assert_eq!(
            table.find_by_primary_key(ColumnValue::U64(1)),
            Some(RowId(0))
        );
        assert_eq!(
            table.find_by_primary_key(ColumnValue::U64(2)),
            Some(RowId(1))
        );
        assert_eq!(
            table.find_by_primary_key(ColumnValue::U64(3)),
            Some(RowId(2))
        );
    }

    #[test]
    fn test_row_retrieval() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        let original_row = create_test_row(42, "Test App", true);
        let row_id = table
            .insert_row(original_row.clone())
            .expect("Failed to insert row");

        let retrieved_row = table.get_row(row_id).expect("Failed to get row");

        assert_eq!(retrieved_row.values.get("id"), Some(&ColumnValue::U64(42)));
        assert_eq!(
            retrieved_row.values.get("title"),
            Some(&ColumnValue::String("Test App".to_string()))
        );
        assert_eq!(
            retrieved_row.values.get("display"),
            Some(&ColumnValue::Bool(true))
        );
    }

    #[test]
    fn test_row_deletion() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        let row = create_test_row(1, "Test App", true);
        let row_id = table.insert_row(row).expect("Failed to insert row");

        // Delete the row
        table.delete_row(row_id).expect("Failed to delete row");

        assert_eq!(table.row_count, 0);

        // Verify row is no longer accessible
        let result = table.get_row(row_id);
        match result {
            Err(TableError::RowNotFound(_)) => {
                // Expected
            }
            _ => panic!("Expected RowNotFound error, got: {:?}", result),
        }

        // Verify primary key index was updated
        assert_eq!(table.find_by_primary_key(ColumnValue::U64(1)), None);
    }

    #[test]
    fn test_row_update() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        let row = create_test_row(1, "Original Title", false);
        let row_id = table.insert_row(row).expect("Failed to insert row");

        // Update title and display fields
        let mut updates = HashMap::new();
        updates.insert(
            "title".to_string(),
            ColumnValue::String("Updated Title".to_string()),
        );
        updates.insert("display".to_string(), ColumnValue::Bool(true));

        table
            .update_row(row_id, updates)
            .expect("Failed to update row");

        // Verify updates
        let updated_row = table.get_row(row_id).expect("Failed to get updated row");
        assert_eq!(updated_row.values.get("id"), Some(&ColumnValue::U64(1)));
        assert_eq!(
            updated_row.values.get("title"),
            Some(&ColumnValue::String("Updated Title".to_string()))
        );
        assert_eq!(
            updated_row.values.get("display"),
            Some(&ColumnValue::Bool(true))
        );
    }

    #[test]
    fn test_primary_key_update_error() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        let row = create_test_row(1, "Test App", true);
        let row_id = table.insert_row(row).expect("Failed to insert row");

        // Try to update primary key - should fail
        let mut updates = HashMap::new();
        updates.insert("id".to_string(), ColumnValue::U64(2));

        let result = table.update_row(row_id, updates);
        // Primary key updates should be rejected (implementation detail)
        assert!(result.is_err());
    }

    #[test]
    fn test_columnar_storage_access() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert multiple rows
        table.insert_row(create_test_row(1, "App 1", true)).unwrap();
        table
            .insert_row(create_test_row(2, "App 2", false))
            .unwrap();
        table.insert_row(create_test_row(3, "App 3", true)).unwrap();

        // Test columnar access
        let id_column = table.get_column_data("id").expect("id column should exist");
        match id_column {
            ColumnData::U64(values) => {
                assert_eq!(values, &vec![1, 2, 3]);
            }
            _ => panic!("Expected U64 column data"),
        }

        let title_column = table
            .get_column_data("title")
            .expect("title column should exist");
        match title_column {
            ColumnData::String(values) => {
                assert_eq!(
                    values,
                    &vec![
                        "App 1".to_string(),
                        "App 2".to_string(),
                        "App 3".to_string()
                    ]
                );
            }
            _ => panic!("Expected String column data"),
        }

        let display_column = table
            .get_column_data("display")
            .expect("display column should exist");
        match display_column {
            ColumnData::Bool(values) => {
                assert_eq!(values, &vec![true, false, true]);
            }
            _ => panic!("Expected Bool column data"),
        }
    }

    #[test]
    fn test_table_scan_all() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert test data
        table.insert_row(create_test_row(1, "App 1", true)).unwrap();
        table
            .insert_row(create_test_row(2, "App 2", false))
            .unwrap();

        let all_rows = table.scan_all();
        assert_eq!(all_rows.len(), 2);

        // Verify row IDs and data
        assert_eq!(all_rows[0].0, RowId(0));
        assert_eq!(all_rows[1].0, RowId(1));

        assert_eq!(all_rows[0].1.values.get("id"), Some(&ColumnValue::U64(1)));
        assert_eq!(all_rows[1].1.values.get("id"), Some(&ColumnValue::U64(2)));
    }

    #[test]
    fn test_type_mismatch_error() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Try to insert row with wrong type for a field
        let mut values = HashMap::new();
        values.insert(
            "id".to_string(),
            ColumnValue::String("not_a_number".to_string()),
        ); // Wrong type
        values.insert("title".to_string(), ColumnValue::String("Test".to_string()));
        values.insert("display".to_string(), ColumnValue::Bool(true));
        let row = TableRow { values };

        let result = table.insert_row(row);
        match result {
            Err(TableError::TypeMismatch {
                column,
                expected,
                found,
            }) => {
                assert_eq!(column, "id");
                assert_eq!(expected, "u64");
                assert_eq!(found, "String");
            }
            _ => panic!("Expected TypeMismatch error, got: {:?}", result),
        }
    }

    #[test]
    fn test_missing_column_error() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Try to insert row missing required column
        let mut values = HashMap::new();
        values.insert("id".to_string(), ColumnValue::U64(1));
        // Missing "title" column
        values.insert("display".to_string(), ColumnValue::Bool(true));
        let row = TableRow { values };

        let result = table.insert_row(row);
        match result {
            Err(TableError::ColumnNotFound(column)) => {
                assert_eq!(column, "title");
            }
            _ => panic!("Expected ColumnNotFound error, got: {:?}", result),
        }
    }

    #[test]
    fn test_empty_table_operations() {
        let schema = create_test_table_schema();
        let table = TableRuntime::new(schema).expect("Failed to create table");

        // Test operations on empty table
        assert_eq!(table.scan_all().len(), 0);
        assert_eq!(table.find_by_primary_key(ColumnValue::U64(1)), None);

        let result = table.get_row(RowId(0));
        assert!(matches!(result, Err(TableError::RowNotFound(_))));
    }

    #[test]
    fn test_row_id_stability_after_deletion() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert multiple rows
        let row_id_1 = table.insert_row(create_test_row(1, "Row 1", true)).unwrap();
        let row_id_2 = table
            .insert_row(create_test_row(2, "Row 2", false))
            .unwrap();
        let row_id_3 = table.insert_row(create_test_row(3, "Row 3", true)).unwrap();

        assert_eq!(row_id_1, RowId(0));
        assert_eq!(row_id_2, RowId(1));
        assert_eq!(row_id_3, RowId(2));

        // Delete the middle row
        table.delete_row(row_id_2).expect("Failed to delete row");

        // Verify that other RowIds are still valid and accessible
        let retrieved_row_1 = table
            .get_row(row_id_1)
            .expect("Row 1 should still be accessible");
        let retrieved_row_3 = table
            .get_row(row_id_3)
            .expect("Row 3 should still be accessible");

        assert_eq!(retrieved_row_1.values.get("id"), Some(&ColumnValue::U64(1)));
        assert_eq!(retrieved_row_3.values.get("id"), Some(&ColumnValue::U64(3)));

        // Verify deleted row is not accessible
        let result = table.get_row(row_id_2);
        assert!(matches!(result, Err(TableError::RowNotFound(_))));

        // Verify primary key index is correct
        assert_eq!(
            table.find_by_primary_key(ColumnValue::U64(1)),
            Some(row_id_1)
        );
        assert_eq!(table.find_by_primary_key(ColumnValue::U64(2)), None);
        assert_eq!(
            table.find_by_primary_key(ColumnValue::U64(3)),
            Some(row_id_3)
        );
    }

    #[test]
    fn test_scan_all_skips_deleted_rows() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert multiple rows
        let row_id_1 = table.insert_row(create_test_row(1, "Row 1", true)).unwrap();
        let row_id_2 = table
            .insert_row(create_test_row(2, "Row 2", false))
            .unwrap();
        let row_id_3 = table.insert_row(create_test_row(3, "Row 3", true)).unwrap();

        // Delete middle row
        table.delete_row(row_id_2).expect("Failed to delete row");

        // Scan should only return non-deleted rows
        let all_rows = table.scan_all();
        assert_eq!(all_rows.len(), 2);

        // Verify the returned rows have correct IDs and data
        let (found_row_id_1, found_row_1) = &all_rows[0];
        let (found_row_id_3, found_row_3) = &all_rows[1];

        assert_eq!(*found_row_id_1, row_id_1);
        assert_eq!(*found_row_id_3, row_id_3);

        assert_eq!(found_row_1.values.get("id"), Some(&ColumnValue::U64(1)));
        assert_eq!(found_row_3.values.get("id"), Some(&ColumnValue::U64(3)));
    }

    #[test]
    fn test_new_insertions_after_deletions() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert and delete some rows
        let row_id_1 = table.insert_row(create_test_row(1, "Row 1", true)).unwrap();
        let row_id_2 = table
            .insert_row(create_test_row(2, "Row 2", false))
            .unwrap();

        table.delete_row(row_id_2).expect("Failed to delete row");

        // Insert new rows - they should get new, stable RowIds
        let row_id_3 = table.insert_row(create_test_row(3, "Row 3", true)).unwrap();
        let row_id_4 = table
            .insert_row(create_test_row(4, "Row 4", false))
            .unwrap();

        // New rows should have RowIds 2 and 3 (continuing sequence)
        assert_eq!(row_id_3, RowId(2));
        assert_eq!(row_id_4, RowId(3));

        // All current rows should be accessible
        assert!(table.get_row(row_id_1).is_ok());
        assert!(table.get_row(row_id_3).is_ok());
        assert!(table.get_row(row_id_4).is_ok());

        // Deleted row should still be inaccessible
        assert!(table.get_row(row_id_2).is_err());

        // Row count should be correct
        assert_eq!(table.row_count, 3);
        assert_eq!(table.scan_all().len(), 3);
    }

    #[test]
    fn test_update_operations_with_stable_row_ids() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert rows
        let row_id_1 = table.insert_row(create_test_row(1, "Row 1", true)).unwrap();
        let row_id_2 = table
            .insert_row(create_test_row(2, "Row 2", false))
            .unwrap();
        let row_id_3 = table.insert_row(create_test_row(3, "Row 3", true)).unwrap();

        // Delete middle row
        table.delete_row(row_id_2).expect("Failed to delete row");

        // Update remaining rows using their stable RowIds
        let mut updates = HashMap::new();
        updates.insert(
            "title".to_string(),
            ColumnValue::String("Updated Row 1".to_string()),
        );
        table
            .update_row(row_id_1, updates)
            .expect("Failed to update row 1");

        let mut updates = HashMap::new();
        updates.insert("display".to_string(), ColumnValue::Bool(false));
        table
            .update_row(row_id_3, updates)
            .expect("Failed to update row 3");

        // Verify updates
        let updated_row_1 = table
            .get_row(row_id_1)
            .expect("Failed to get updated row 1");
        let updated_row_3 = table
            .get_row(row_id_3)
            .expect("Failed to get updated row 3");

        assert_eq!(
            updated_row_1.values.get("title"),
            Some(&ColumnValue::String("Updated Row 1".to_string()))
        );
        assert_eq!(
            updated_row_3.values.get("display"),
            Some(&ColumnValue::Bool(false))
        );

        // Try to update deleted row - should fail
        let mut updates = HashMap::new();
        updates.insert(
            "title".to_string(),
            ColumnValue::String("Should not work".to_string()),
        );
        let result = table.update_row(row_id_2, updates);
        assert!(matches!(result, Err(TableError::RowNotFound(_))));
    }

    #[test]
    fn test_multiple_deletions_and_primary_key_consistency() {
        let schema = create_test_table_schema();
        let mut table = TableRuntime::new(schema).expect("Failed to create table");

        // Insert multiple rows
        let mut row_ids = Vec::new();
        for i in 1..=5 {
            let row_id = table
                .insert_row(create_test_row(i, &format!("Row {}", i), i % 2 == 0))
                .unwrap();
            row_ids.push(row_id);
        }

        // Delete rows 2 and 4
        table
            .delete_row(row_ids[1])
            .expect("Failed to delete row 2");
        table
            .delete_row(row_ids[3])
            .expect("Failed to delete row 4");

        // Verify primary key index is consistent
        assert_eq!(
            table.find_by_primary_key(ColumnValue::U64(1)),
            Some(row_ids[0])
        );
        assert_eq!(table.find_by_primary_key(ColumnValue::U64(2)), None);
        assert_eq!(
            table.find_by_primary_key(ColumnValue::U64(3)),
            Some(row_ids[2])
        );
        assert_eq!(table.find_by_primary_key(ColumnValue::U64(4)), None);
        assert_eq!(
            table.find_by_primary_key(ColumnValue::U64(5)),
            Some(row_ids[4])
        );

        // Verify remaining rows are accessible
        assert!(table.get_row(row_ids[0]).is_ok());
        assert!(table.get_row(row_ids[2]).is_ok());
        assert!(table.get_row(row_ids[4]).is_ok());

        // Verify deleted rows are not accessible
        assert!(table.get_row(row_ids[1]).is_err());
        assert!(table.get_row(row_ids[3]).is_err());

        // Verify scan returns correct count
        assert_eq!(table.scan_all().len(), 3);
        assert_eq!(table.row_count, 3);
    }
}
