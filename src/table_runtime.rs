use crate::ast::*;
use crate::computed_columns::*;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct TableRuntime {
    pub schema: TableDef,
    pub storage: ColumnarStorage,
    pub primary_index: PrimaryKeyIndex,
    pub row_count: usize,
    pub deleted_rows: std::collections::HashSet<usize>, // Tombstones for deleted rows
    pub next_row_id: usize, // Next available row ID
    pub computed_engine: Option<LazyEvaluationEngine>, // Lazy evaluation for computed columns
}

#[derive(Debug, Clone)]
pub struct ColumnarStorage {
    pub columns: HashMap<String, ColumnData>,
}

#[derive(Debug, Clone)]
pub enum ColumnData {
    U64(Vec<Option<u64>>),
    String(Vec<Option<String>>),
    Bool(Vec<Option<bool>>),
    F64(Vec<Option<f64>>),
}

#[derive(Debug, Clone)]
pub struct PrimaryKeyIndex {
    pub column_name: Option<String>,
    pub index: HashMap<ColumnValue, RowId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ColumnValue {
    U64(u64),
    String(String),
    Bool(bool),
    F64(u64), // Store as bits for Eq/Hash
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowId(pub usize);

#[derive(Debug, Clone)]
pub struct TableRow {
    pub values: HashMap<String, ColumnValue>,
}

#[derive(Debug, Clone)]
pub enum TableError {
    DuplicatePrimaryKey(ColumnValue),
    ColumnNotFound(String),
    TypeMismatch { column: String, expected: String, found: String },
    PrimaryKeyRequired,
    RowNotFound(RowId),
}

impl ColumnData {
    fn new_for_type(type_name: &str) -> Self {
        match type_name {
            "u64" => ColumnData::U64(Vec::new()),
            "String" => ColumnData::String(Vec::new()),
            "bool" => ColumnData::Bool(Vec::new()),
            "f64" => ColumnData::F64(Vec::new()),
            _ => ColumnData::String(Vec::new()), // Default fallback
        }
    }

    fn push_value(&mut self, value: ColumnValue) -> Result<(), TableError> {
        match (self, value) {
            (ColumnData::U64(vec), ColumnValue::U64(v)) => vec.push(Some(v)),
            (ColumnData::String(vec), ColumnValue::String(v)) => vec.push(Some(v)),
            (ColumnData::Bool(vec), ColumnValue::Bool(v)) => vec.push(Some(v)),
            (ColumnData::F64(vec), ColumnValue::F64(v)) => vec.push(Some(f64::from_bits(v))),
            _ => return Err(TableError::TypeMismatch {
                column: "unknown".to_string(),
                expected: "matching type".to_string(),
                found: "incompatible type".to_string(),
            }),
        }
        Ok(())
    }

    fn set_value_at(&mut self, index: usize, value: Option<ColumnValue>) -> Result<(), TableError> {
        // Expand vector if necessary
        while self.len() <= index {
            match self {
                ColumnData::U64(vec) => vec.push(None),
                ColumnData::String(vec) => vec.push(None),
                ColumnData::Bool(vec) => vec.push(None),
                ColumnData::F64(vec) => vec.push(None),
            }
        }

        match (self, value) {
            (ColumnData::U64(vec), Some(ColumnValue::U64(v))) => vec[index] = Some(v),
            (ColumnData::U64(vec), None) => vec[index] = None,
            (ColumnData::String(vec), Some(ColumnValue::String(v))) => vec[index] = Some(v),
            (ColumnData::String(vec), None) => vec[index] = None,
            (ColumnData::Bool(vec), Some(ColumnValue::Bool(v))) => vec[index] = Some(v),
            (ColumnData::Bool(vec), None) => vec[index] = None,
            (ColumnData::F64(vec), Some(ColumnValue::F64(v))) => vec[index] = Some(f64::from_bits(v)),
            (ColumnData::F64(vec), None) => vec[index] = None,
            (_, Some(_)) => return Err(TableError::TypeMismatch {
                column: "unknown".to_string(),
                expected: "matching type".to_string(),
                found: "incompatible type".to_string(),
            }),
        }
        Ok(())
    }

    pub fn get_value(&self, index: usize) -> Option<ColumnValue> {
        match self {
            ColumnData::U64(vec) => vec.get(index)?.as_ref().map(|&v| ColumnValue::U64(v)),
            ColumnData::String(vec) => vec.get(index)?.as_ref().map(|v| ColumnValue::String(v.clone())),
            ColumnData::Bool(vec) => vec.get(index)?.as_ref().map(|&v| ColumnValue::Bool(v)),
            ColumnData::F64(vec) => vec.get(index)?.as_ref().map(|&v| ColumnValue::F64(v.to_bits())),
        }
    }

    fn set_value(&mut self, index: usize, value: ColumnValue) -> Result<(), TableError> {
        match (self, value) {
            (ColumnData::U64(vec), ColumnValue::U64(v)) => {
                if let Some(slot) = vec.get_mut(index) {
                    *slot = Some(v);
                }
            }
            (ColumnData::String(vec), ColumnValue::String(v)) => {
                if let Some(slot) = vec.get_mut(index) {
                    *slot = Some(v);
                }
            }
            (ColumnData::Bool(vec), ColumnValue::Bool(v)) => {
                if let Some(slot) = vec.get_mut(index) {
                    *slot = Some(v);
                }
            }
            (ColumnData::F64(vec), ColumnValue::F64(v)) => {
                if let Some(slot) = vec.get_mut(index) {
                    *slot = Some(f64::from_bits(v));
                }
            }
            _ => return Err(TableError::TypeMismatch {
                column: "unknown".to_string(),
                expected: "matching type".to_string(),
                found: "incompatible type".to_string(),
            }),
        }
        Ok(())
    }

    fn mark_deleted(&mut self, index: usize) {
        match self {
            ColumnData::U64(vec) => {
                if let Some(slot) = vec.get_mut(index) {
                    *slot = None;
                }
            }
            ColumnData::String(vec) => {
                if let Some(slot) = vec.get_mut(index) {
                    *slot = None;
                }
            }
            ColumnData::Bool(vec) => {
                if let Some(slot) = vec.get_mut(index) {
                    *slot = None;
                }
            }
            ColumnData::F64(vec) => {
                if let Some(slot) = vec.get_mut(index) {
                    *slot = None;
                }
            }
        }
    }

    fn len(&self) -> usize {
        match self {
            ColumnData::U64(vec) => vec.len(),
            ColumnData::String(vec) => vec.len(),
            ColumnData::Bool(vec) => vec.len(),
            ColumnData::F64(vec) => vec.len(),
        }
    }
}

impl TableRuntime {
    pub fn new(schema: TableDef) -> Result<Self, TableError> {
        let mut storage = ColumnarStorage {
            columns: HashMap::new(),
        };

        // Initialize column storage based on schema
        for column in &schema.columns {
            let type_name = match &column.column_type {
                Type::Identifier { name, .. } => name.clone(),
                _ => "String".to_string(), // Default fallback
            };
            storage.columns.insert(column.name.clone(), ColumnData::new_for_type(&type_name));
        }

        // Find primary key column
        let primary_key_column = schema.columns.iter()
            .find(|col| col.annotations.iter().any(|ann| ann.name == "primary"))
            .map(|col| col.name.clone());

        let primary_index = PrimaryKeyIndex {
            column_name: primary_key_column,
            index: HashMap::new(),
        };

        // Initialize computed column engine if there are computed columns
        let computed_engine = if schema.columns.iter().any(|col| col.is_computed) {
            Some(LazyEvaluationEngine::new(&schema).map_err(|e| {
                TableError::TypeMismatch {
                    column: "computed_engine".to_string(),
                    expected: "valid computed column configuration".to_string(),
                    found: format!("evaluation error: {:?}", e),
                }
            })?)
        } else {
            None
        };

        Ok(TableRuntime {
            schema,
            storage,
            primary_index,
            row_count: 0,
            deleted_rows: std::collections::HashSet::new(),
            next_row_id: 0,
            computed_engine,
        })
    }

    /// Get the value of a column (regular or computed) for a specific row
    pub fn get_column_value(&mut self, row_id: RowId, column_name: &str) -> Result<ColumnValue, TableError> {
        // Check if the column is computed
        if let Some(column) = self.schema.columns.iter().find(|col| col.name == column_name) {
            if column.is_computed {
                if let Some(ref mut engine) = self.computed_engine {
                    return engine.get_computed_value(column_name, row_id, &self.storage)
                        .map_err(|e| TableError::TypeMismatch {
                            column: column_name.to_string(),
                            expected: "computed value".to_string(),
                            found: format!("evaluation error: {:?}", e),
                        });
                } else {
                    return Err(TableError::TypeMismatch {
                        column: column_name.to_string(),
                        expected: "computed column support".to_string(),
                        found: "no computed engine available".to_string(),
                    });
                }
            }
        }

        // Get value from regular column storage
        let column_data = self.storage.columns.get(column_name)
            .ok_or_else(|| TableError::ColumnNotFound(column_name.to_string()))?;

        column_data.get_value(row_id.0)
            .ok_or_else(|| TableError::RowNotFound(row_id))
    }

    /// Mark a column as dirty (needs recomputation for computed columns)
    pub fn mark_column_dirty(&mut self, column_name: &str) {
        if let Some(ref mut engine) = self.computed_engine {
            engine.mark_column_dirty(column_name);
        }
    }

    /// Mark a specific row in a column as dirty
    pub fn mark_row_dirty(&mut self, column_name: &str, row_id: RowId) {
        if let Some(ref mut engine) = self.computed_engine {
            engine.mark_row_dirty(column_name, row_id);
        }
    }

    /// Check if a column is computed
    pub fn is_computed_column(&self, column_name: &str) -> bool {
        self.schema.columns.iter()
            .find(|col| col.name == column_name)
            .map_or(false, |col| col.is_computed)
    }

    /// Get cache statistics for computed columns
    pub fn get_computed_cache_stats(&self) -> HashMap<String, usize> {
        self.computed_engine
            .as_ref()
            .map_or_else(HashMap::new, |engine| engine.get_cache_stats())
    }

    pub fn insert_row(&mut self, row: TableRow) -> Result<RowId, TableError> {
        // Validate all required columns are present and apply defaults
        let mut complete_row = HashMap::new();
        
        for column in &self.schema.columns {
            if let Some(value) = row.values.get(&column.name) {
                // Validate type compatibility
                self.validate_column_value(&column.name, value)?;
                complete_row.insert(column.name.clone(), value.clone());
            } else if let Some(default_expr) = &column.default_value {
                // Apply default value
                let default_value = self.evaluate_default_expression(default_expr)?;
                complete_row.insert(column.name.clone(), default_value);
            } else {
                return Err(TableError::ColumnNotFound(column.name.clone()));
            }
        }

        // Check primary key uniqueness
        if let Some(pk_column) = &self.primary_index.column_name {
            if let Some(pk_value) = complete_row.get(pk_column) {
                if self.primary_index.index.contains_key(pk_value) {
                    return Err(TableError::DuplicatePrimaryKey(pk_value.clone()));
                }
            }
        }

        // Get next available row ID (stable across deletions)
        let row_id = RowId(self.next_row_id);
        
        // Insert into columnar storage at the specific index
        for (column_name, value) in &complete_row {
            if let Some(column_data) = self.storage.columns.get_mut(column_name) {
                column_data.set_value_at(row_id.0, Some(value.clone()))?;
            }
        }

        // Update primary key index
        if let Some(pk_column) = &self.primary_index.column_name {
            if let Some(pk_value) = complete_row.get(pk_column) {
                self.primary_index.index.insert(pk_value.clone(), row_id);
            }
        }

        self.row_count += 1;
        self.next_row_id += 1;
        Ok(row_id)
    }

    pub fn get_row(&self, row_id: RowId) -> Result<TableRow, TableError> {
        // Check if row was deleted
        if self.deleted_rows.contains(&row_id.0) {
            return Err(TableError::RowNotFound(row_id));
        }

        // Check if row ID is valid
        if row_id.0 >= self.next_row_id {
            return Err(TableError::RowNotFound(row_id));
        }

        let mut values = HashMap::new();
        
        for column in &self.schema.columns {
            if let Some(column_data) = self.storage.columns.get(&column.name) {
                if let Some(value) = column_data.get_value(row_id.0) {
                    values.insert(column.name.clone(), value);
                } else {
                    // Row was deleted if any column is missing data
                    return Err(TableError::RowNotFound(row_id));
                }
            }
        }

        Ok(TableRow { values })
    }

    pub fn delete_row(&mut self, row_id: RowId) -> Result<(), TableError> {
        // Check if row exists and is not already deleted
        if self.deleted_rows.contains(&row_id.0) || row_id.0 >= self.next_row_id {
            return Err(TableError::RowNotFound(row_id));
        }

        // Remove from primary key index first
        if let Some(pk_column) = &self.primary_index.column_name {
            if let Some(column_data) = self.storage.columns.get(pk_column) {
                if let Some(pk_value) = column_data.get_value(row_id.0) {
                    self.primary_index.index.remove(&pk_value);
                }
            }
        }

        // Mark row as deleted using tombstones
        for column_data in self.storage.columns.values_mut() {
            column_data.mark_deleted(row_id.0);
        }

        // Add to deleted rows set
        self.deleted_rows.insert(row_id.0);
        self.row_count -= 1;
        Ok(())
    }

    pub fn update_row(&mut self, row_id: RowId, updates: HashMap<String, ColumnValue>) -> Result<(), TableError> {
        // Check if row exists and is not deleted
        if self.deleted_rows.contains(&row_id.0) || row_id.0 >= self.next_row_id {
            return Err(TableError::RowNotFound(row_id));
        }

        // Reject primary key updates for simplicity
        if let Some(pk_column) = &self.primary_index.column_name {
            if updates.contains_key(pk_column) {
                return Err(TableError::TypeMismatch {
                    column: pk_column.clone(),
                    expected: "immutable primary key".to_string(),
                    found: "update attempt".to_string(),
                });
            }
        }

        // Apply updates
        for (column_name, value) in updates {
            self.validate_column_value(&column_name, &value)?;
            
            if let Some(column_data) = self.storage.columns.get_mut(&column_name) {
                column_data.set_value(row_id.0, value)?;
            } else {
                return Err(TableError::ColumnNotFound(column_name));
            }
        }

        Ok(())
    }

    pub fn find_by_primary_key(&self, key: ColumnValue) -> Option<RowId> {
        self.primary_index.index.get(&key).copied()
    }

    pub fn scan_all(&self) -> Vec<(RowId, TableRow)> {
        let mut result = Vec::new();
        
        for i in 0..self.next_row_id {
            let row_id = RowId(i);
            if !self.deleted_rows.contains(&i) {
                if let Ok(row) = self.get_row(row_id) {
                    result.push((row_id, row));
                }
            }
        }
        
        result
    }

    pub fn get_column_data(&self, column_name: &str) -> Option<&ColumnData> {
        self.storage.columns.get(column_name)
    }

    fn validate_column_value(&self, column_name: &str, value: &ColumnValue) -> Result<(), TableError> {
        let column = self.schema.columns.iter()
            .find(|col| col.name == column_name)
            .ok_or_else(|| TableError::ColumnNotFound(column_name.to_string()))?;

        let expected_type = match &column.column_type {
            Type::Identifier { name, .. } => name.clone(),
            _ => "unknown".to_string(),
        };

        let actual_type = match value {
            ColumnValue::U64(_) => "u64",
            ColumnValue::String(_) => "String",
            ColumnValue::Bool(_) => "bool",
            ColumnValue::F64(_) => "f64",
        };

        if expected_type != actual_type {
            return Err(TableError::TypeMismatch {
                column: column_name.to_string(),
                expected: expected_type,
                found: actual_type.to_string(),
            });
        }

        Ok(())
    }

    fn evaluate_default_expression(&self, expr: &Expression) -> Result<ColumnValue, TableError> {
        match expr {
            Expression::Literal(Literal::Boolean(b)) => Ok(ColumnValue::Bool(*b)),
            Expression::Literal(Literal::Integer(int_lit)) => {
                Ok(ColumnValue::U64(int_lit.value as u64))
            }
            Expression::Literal(Literal::Float(f)) => Ok(ColumnValue::F64(f.to_bits())),
            Expression::Literal(Literal::String(s)) => Ok(ColumnValue::String(s.clone())),
            Expression::Literal(Literal::Char(c)) => Ok(ColumnValue::String(c.to_string())),
            _ => Err(TableError::TypeMismatch {
                column: "default".to_string(),
                expected: "literal".to_string(),
                found: "complex expression".to_string(),
            }),
        }
    }
}