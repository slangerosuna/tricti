use peano::ast::*;
use peano::parser;
use peano::table_runtime::*;
use std::collections::HashMap;

#[test]
fn test_end_to_end_computed_columns() {
    // Parse a table with computed columns
    let src = r#"
        Orders :: table {
            @primary id: u64,
            quantity: u64,
            unit_price: f64,
            subtotal: computed(quantity * unit_price),
            tax_rate: f64 = 0.1,
            tax_amount: computed(subtotal * tax_rate),
            total: computed(subtotal + tax_amount),
        }
    "#;

    let program = parser::parse(src.to_string());
    assert_eq!(program.statements.len(), 1);

    // Extract the table definition
    let table_def = match &program.statements[0] {
        Statement::ConstDecl {
            value: ConstValue::TableDef(table),
            ..
        } => table.clone(),
        _ => panic!("Expected table definition"),
    };

    // Create table runtime
    let mut table = TableRuntime::new(table_def).expect("Failed to create table runtime");

    // Verify computed columns are recognized
    assert!(table.is_computed_column("subtotal"));
    assert!(table.is_computed_column("tax_amount"));
    assert!(table.is_computed_column("total"));
    assert!(!table.is_computed_column("quantity"));
    assert!(!table.is_computed_column("unit_price"));

    // Insert a row with only the regular columns
    let mut row_values = HashMap::new();
    row_values.insert("id".to_string(), ColumnValue::U64(1));
    row_values.insert("quantity".to_string(), ColumnValue::U64(5));
    row_values.insert(
        "unit_price".to_string(),
        ColumnValue::F64((10.0_f64).to_bits()),
    );

    let row = TableRow { values: row_values };
    let row_id = table.insert_row(row).expect("Failed to insert row");

    // Test getting computed values
    let subtotal = table
        .get_column_value(row_id, "subtotal")
        .expect("Failed to get subtotal");
    if let ColumnValue::F64(bits) = subtotal {
        let subtotal_f64 = f64::from_bits(bits);
        assert_eq!(subtotal_f64, 50.0); // 5 * 10.0
    } else {
        panic!("Expected f64 value for subtotal");
    }

    let tax_amount = table
        .get_column_value(row_id, "tax_amount")
        .expect("Failed to get tax_amount");
    if let ColumnValue::F64(bits) = tax_amount {
        let tax_amount_f64 = f64::from_bits(bits);
        assert_eq!(tax_amount_f64, 5.0); // 50.0 * 0.1
    } else {
        panic!("Expected f64 value for tax_amount");
    }

    let total = table
        .get_column_value(row_id, "total")
        .expect("Failed to get total");
    if let ColumnValue::F64(bits) = total {
        let total_f64 = f64::from_bits(bits);
        assert_eq!(total_f64, 55.0); // 50.0 + 5.0
    } else {
        panic!("Expected f64 value for total");
    }

    // Test caching - getting the same values again should be faster
    let total2 = table
        .get_column_value(row_id, "total")
        .expect("Failed to get total again");
    assert_eq!(total, total2);

    // Verify cache statistics
    let cache_stats = table.get_computed_cache_stats();
    assert!(cache_stats.get("subtotal").unwrap_or(&0) > &0);
    assert!(cache_stats.get("tax_amount").unwrap_or(&0) > &0);
    assert!(cache_stats.get("total").unwrap_or(&0) > &0);
}

#[test]
fn test_computed_column_dependency_chain() {
    // Test a more complex dependency chain
    let src = r#"
        Products :: table {
            @primary id: u64,
            cost: f64,
            markup: f64,
            price: computed(cost + markup),
            tax_rate: f64,
            tax: computed(price * tax_rate),
            final_price: computed(price + tax),
        }
    "#;

    let program = parser::parse(src.to_string());
    let table_def = match &program.statements[0] {
        Statement::ConstDecl {
            value: ConstValue::TableDef(table),
            ..
        } => table.clone(),
        _ => panic!("Expected table definition"),
    };

    let mut table = TableRuntime::new(table_def).expect("Failed to create table runtime");

    // Insert test data
    let mut row_values = HashMap::new();
    row_values.insert("id".to_string(), ColumnValue::U64(1));
    row_values.insert("cost".to_string(), ColumnValue::F64((100.0_f64).to_bits()));
    row_values.insert("markup".to_string(), ColumnValue::F64((20.0_f64).to_bits()));
    row_values.insert(
        "tax_rate".to_string(),
        ColumnValue::F64((0.08_f64).to_bits()),
    );

    let row = TableRow { values: row_values };
    let row_id = table.insert_row(row).expect("Failed to insert row");

    // Test dependency chain evaluation
    let price = table
        .get_column_value(row_id, "price")
        .expect("Failed to get price");
    if let ColumnValue::F64(bits) = price {
        let price_f64 = f64::from_bits(bits);
        assert_eq!(price_f64, 120.0); // 100.0 + 20.0
    } else {
        panic!("Expected f64 value for price");
    }

    let tax = table
        .get_column_value(row_id, "tax")
        .expect("Failed to get tax");
    if let ColumnValue::F64(bits) = tax {
        let tax_f64 = f64::from_bits(bits);
        assert_eq!(tax_f64, 9.6); // 120.0 * 0.08
    } else {
        panic!("Expected f64 value for tax");
    }

    let final_price = table
        .get_column_value(row_id, "final_price")
        .expect("Failed to get final_price");
    if let ColumnValue::F64(bits) = final_price {
        let final_price_f64 = f64::from_bits(bits);
        assert_eq!(final_price_f64, 129.6); // 120.0 + 9.6
    } else {
        panic!("Expected f64 value for final_price");
    }
}

#[test]
fn test_computed_column_with_string_concatenation() {
    let src = r#"
        Users :: table {
            @primary id: u64,
            first_name: String,
            last_name: String,
            full_name: computed(first_name + " " + last_name),
        }
    "#;

    let program = parser::parse(src.to_string());
    let table_def = match &program.statements[0] {
        Statement::ConstDecl {
            value: ConstValue::TableDef(table),
            ..
        } => table.clone(),
        _ => panic!("Expected table definition"),
    };

    let mut table = TableRuntime::new(table_def).expect("Failed to create table runtime");

    // Insert test data
    let mut row_values = HashMap::new();
    row_values.insert("id".to_string(), ColumnValue::U64(1));
    row_values.insert(
        "first_name".to_string(),
        ColumnValue::String("John".to_string()),
    );
    row_values.insert(
        "last_name".to_string(),
        ColumnValue::String("Doe".to_string()),
    );

    let row = TableRow { values: row_values };
    let row_id = table.insert_row(row).expect("Failed to insert row");

    // Test string concatenation
    let full_name = table
        .get_column_value(row_id, "full_name")
        .expect("Failed to get full_name");
    if let ColumnValue::String(name) = full_name {
        assert_eq!(name, "John Doe");
    } else {
        panic!("Expected string value for full_name");
    }
}

#[test]
fn test_computed_column_invalidation() {
    let src = r#"
        Inventory :: table {
            @primary id: u64,
            base_quantity: u64,
            multiplier: u64,
            total_quantity: computed(base_quantity * multiplier),
        }
    "#;

    let program = parser::parse(src.to_string());
    let table_def = match &program.statements[0] {
        Statement::ConstDecl {
            value: ConstValue::TableDef(table),
            ..
        } => table.clone(),
        _ => panic!("Expected table definition"),
    };

    let mut table = TableRuntime::new(table_def).expect("Failed to create table runtime");

    // Insert test data
    let mut row_values = HashMap::new();
    row_values.insert("id".to_string(), ColumnValue::U64(1));
    row_values.insert("base_quantity".to_string(), ColumnValue::U64(10));
    row_values.insert("multiplier".to_string(), ColumnValue::U64(3));

    let row = TableRow { values: row_values };
    let row_id = table.insert_row(row).expect("Failed to insert row");

    // Get computed value
    let total = table
        .get_column_value(row_id, "total_quantity")
        .expect("Failed to get total_quantity");
    if let ColumnValue::U64(val) = total {
        assert_eq!(val, 30); // 10 * 3
    } else {
        panic!("Expected u64 value for total_quantity");
    }

    // Test cache invalidation
    table.mark_column_dirty("base_quantity");
    table.mark_row_dirty("total_quantity", row_id);

    // Cache stats should show the value is cached
    let stats = table.get_computed_cache_stats();
    assert!(stats.get("total_quantity").unwrap_or(&0) > &0);
}
