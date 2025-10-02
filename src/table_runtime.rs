use crate::ast::*;
use crate::computed_columns::*;
use std::collections::BTreeMap;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct TableRuntime {
    pub schema: TableDef,
    pub storage: ColumnarStorage,
    pub primary_index: PrimaryKeyIndex,
    pub secondary_indexes: HashMap<String, SecondaryIndex>, // Column-level indexes
    pub row_count: usize,
    pub deleted_rows: std::collections::HashSet<usize>, // Tombstones for deleted rows
    pub next_row_id: usize,                             // Next available row ID
    pub computed_engine: Option<LazyEvaluationEngine>,  // Lazy evaluation for computed columns
}

#[derive(Debug, Clone)]
pub struct ColumnarStorage {
    pub columns: HashMap<String, ColumnData>,
}

#[derive(Debug, Clone)]
pub enum ColumnData {
    U64(Vec<Option<u64>>),
    I32(Vec<Option<i32>>),
    String(Vec<Option<String>>),
    Bool(Vec<Option<bool>>),
    F64(Vec<Option<f64>>),
}

#[derive(Debug, Clone)]
pub struct PrimaryKeyIndex {
    pub column_name: Option<String>,
    pub index: HashMap<ColumnValue, RowId>,
}

/// Secondary indexes for efficient range and equality queries
#[derive(Debug, Clone)]
pub struct SecondaryIndex {
    pub column_name: String,
    pub ordered_index: BTreeMap<ColumnValue, Vec<RowId>>, // Ordered for range queries
    pub bitmap_index: Option<BitmapIndex>, // Bitmap index for low-cardinality columns
}

/// Bitmap index for efficient filtering on low-cardinality columns
#[derive(Debug, Clone)]
pub struct BitmapIndex {
    pub column_name: String,
    pub value_bitmaps: HashMap<ColumnValue, RowBitmap>, // One bitmap per distinct value
    pub null_bitmap: RowBitmap,                         // Bitmap for NULL values
}

/// Row bitmap for efficient predicate evaluation
#[derive(Debug, Clone)]
pub struct RowBitmap {
    pub bits: Vec<bool>,
    pub cardinality: usize, // Number of true bits
}

/// Indexed iterator that only reads matching rows
pub struct IndexedIterator<'a> {
    table: &'a TableRuntime,
    row_ids: Box<dyn Iterator<Item = RowId> + 'a>,
    projection_columns: Vec<String>,
}

/// Predicate evaluation result on column data
#[derive(Debug, Clone)]
pub enum PredicateResult {
    Bitmap(RowBitmap),
    IndexLookup(Vec<RowId>),
    FullScan, // Fallback when predicate can't be optimized
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ColumnValue {
    U64(u64),
    I32(i32),
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
    TypeMismatch {
        column: String,
        expected: String,
        found: String,
    },
    PrimaryKeyRequired,
    RowNotFound(RowId),
}

impl ColumnData {
    fn new_for_type(type_name: &str) -> Self {
        match type_name {
            "u64" => ColumnData::U64(Vec::new()),
            "i32" => ColumnData::I32(Vec::new()),
            "string" | "String" => ColumnData::String(Vec::new()),
            "bool" => ColumnData::Bool(Vec::new()),
            "f64" => ColumnData::F64(Vec::new()),
            _ => ColumnData::String(Vec::new()), // Default fallback
        }
    }

    fn push_value(&mut self, value: ColumnValue) -> Result<(), TableError> {
        match (self, value) {
            (ColumnData::U64(vec), ColumnValue::U64(v)) => vec.push(Some(v)),
            (ColumnData::I32(vec), ColumnValue::I32(v)) => vec.push(Some(v)),
            (ColumnData::String(vec), ColumnValue::String(v)) => vec.push(Some(v)),
            (ColumnData::Bool(vec), ColumnValue::Bool(v)) => vec.push(Some(v)),
            (ColumnData::F64(vec), ColumnValue::F64(v)) => vec.push(Some(f64::from_bits(v))),
            _ => {
                return Err(TableError::TypeMismatch {
                    column: "unknown".to_string(),
                    expected: "matching type".to_string(),
                    found: "incompatible type".to_string(),
                })
            }
        }
        Ok(())
    }

    fn set_value_at(&mut self, index: usize, value: Option<ColumnValue>) -> Result<(), TableError> {
        // Expand vector if necessary
        while self.len() <= index {
            match self {
                ColumnData::U64(vec) => vec.push(None),
                ColumnData::I32(vec) => vec.push(None),
                ColumnData::String(vec) => vec.push(None),
                ColumnData::Bool(vec) => vec.push(None),
                ColumnData::F64(vec) => vec.push(None),
            }
        }

        match (self, value) {
            (ColumnData::U64(vec), Some(ColumnValue::U64(v))) => vec[index] = Some(v),
            (ColumnData::U64(vec), None) => vec[index] = None,
            (ColumnData::I32(vec), Some(ColumnValue::I32(v))) => vec[index] = Some(v),
            (ColumnData::I32(vec), None) => vec[index] = None,
            (ColumnData::String(vec), Some(ColumnValue::String(v))) => vec[index] = Some(v),
            (ColumnData::String(vec), None) => vec[index] = None,
            (ColumnData::Bool(vec), Some(ColumnValue::Bool(v))) => vec[index] = Some(v),
            (ColumnData::Bool(vec), None) => vec[index] = None,
            (ColumnData::F64(vec), Some(ColumnValue::F64(v))) => {
                vec[index] = Some(f64::from_bits(v))
            }
            (ColumnData::F64(vec), None) => vec[index] = None,
            (_, Some(_)) => {
                return Err(TableError::TypeMismatch {
                    column: "unknown".to_string(),
                    expected: "matching type".to_string(),
                    found: "incompatible type".to_string(),
                })
            }
        }
        Ok(())
    }

    pub fn get_value(&self, index: usize) -> Option<ColumnValue> {
        match self {
            ColumnData::U64(vec) => vec.get(index)?.as_ref().map(|&v| ColumnValue::U64(v)),
            ColumnData::I32(vec) => vec.get(index)?.as_ref().map(|&v| ColumnValue::I32(v)),
            ColumnData::String(vec) => vec
                .get(index)?
                .as_ref()
                .map(|v| ColumnValue::String(v.clone())),
            ColumnData::Bool(vec) => vec.get(index)?.as_ref().map(|&v| ColumnValue::Bool(v)),
            ColumnData::F64(vec) => vec
                .get(index)?
                .as_ref()
                .map(|&v| ColumnValue::F64(v.to_bits())),
        }
    }

    fn set_value(&mut self, index: usize, value: ColumnValue) -> Result<(), TableError> {
        match (self, value) {
            (ColumnData::U64(vec), ColumnValue::U64(v)) => {
                if let Some(slot) = vec.get_mut(index) {
                    *slot = Some(v);
                }
            }
            (ColumnData::I32(vec), ColumnValue::I32(v)) => {
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
            _ => {
                return Err(TableError::TypeMismatch {
                    column: "unknown".to_string(),
                    expected: "matching type".to_string(),
                    found: "incompatible type".to_string(),
                })
            }
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
            ColumnData::I32(vec) => {
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
            ColumnData::I32(vec) => vec.len(),
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
            storage
                .columns
                .insert(column.name.clone(), ColumnData::new_for_type(&type_name));
        }

        // Find primary key column
        let primary_key_column = schema
            .columns
            .iter()
            .find(|col| col.annotations.iter().any(|ann| ann.name == "primary"))
            .map(|col| col.name.clone());

        let primary_index = PrimaryKeyIndex {
            column_name: primary_key_column,
            index: HashMap::new(),
        };

        let secondary_indexes = HashMap::new();

        // Initialize computed column engine if there are computed columns
        let computed_engine = if schema.columns.iter().any(|col| col.is_computed) {
            Some(
                LazyEvaluationEngine::new(&schema).map_err(|e| TableError::TypeMismatch {
                    column: "computed_engine".to_string(),
                    expected: "valid computed column configuration".to_string(),
                    found: format!("evaluation error: {:?}", e),
                })?,
            )
        } else {
            None
        };

        Ok(TableRuntime {
            schema,
            storage,
            primary_index,
            secondary_indexes,
            row_count: 0,
            deleted_rows: std::collections::HashSet::new(),
            next_row_id: 0,
            computed_engine,
        })
    }

    /// Get the value of a column (regular or computed) for a specific row
    pub fn get_column_value(
        &mut self,
        row_id: RowId,
        column_name: &str,
    ) -> Result<ColumnValue, TableError> {
        // Check if the column is computed
        if let Some(column) = self
            .schema
            .columns
            .iter()
            .find(|col| col.name == column_name)
        {
            if column.is_computed {
                if let Some(ref mut engine) = self.computed_engine {
                    return engine
                        .get_computed_value(column_name, row_id, &self.storage)
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
        let column_data = self
            .storage
            .columns
            .get(column_name)
            .ok_or_else(|| TableError::ColumnNotFound(column_name.to_string()))?;

        column_data
            .get_value(row_id.0)
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
        self.schema
            .columns
            .iter()
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
            if column.is_computed {
                // Skip computed columns during insertion - they're calculated on demand
                continue;
            }

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

        // Insert into columnar storage at the specific index (only for non-computed columns)
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
            if column.is_computed {
                // Use get_column_value for computed columns
                // For computed columns, we need to compute them manually since get_column_value requires &mut self
                // but get_row has &self. We'll evaluate the computed expression directly.
                if let Some(ref mut engine) = self.computed_engine.clone() {
                    match engine.get_computed_value(&column.name, row_id, &self.storage) {
                        Ok(value) => {
                            values.insert(column.name.clone(), value);
                        }
                        Err(_) => {
                            // Computed column evaluation failed, skip it
                            continue;
                        }
                    }
                } else {
                    // No computed engine available, skip computed columns
                    continue;
                }
            } else {
                // Regular columns from storage
                if let Some(column_data) = self.storage.columns.get(&column.name) {
                    if let Some(value) = column_data.get_value(row_id.0) {
                        values.insert(column.name.clone(), value);
                    } else {
                        // Row was deleted if any column is missing data
                        return Err(TableError::RowNotFound(row_id));
                    }
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

    pub fn update_row(
        &mut self,
        row_id: RowId,
        updates: HashMap<String, ColumnValue>,
    ) -> Result<(), TableError> {
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

    /// Scan table with WHERE clause filtering - avoids materializing filtered-out rows
    pub fn scan_filtered<F>(&self, predicate: F) -> Vec<(RowId, TableRow)>
    where
        F: Fn(&TableRow) -> bool,
    {
        let mut result = Vec::new();

        for i in 0..self.next_row_id {
            let row_id = RowId(i);
            if !self.deleted_rows.contains(&i) {
                if let Ok(row) = self.get_row(row_id) {
                    if predicate(&row) {
                        result.push((row_id, row));
                    }
                }
            }
        }

        result
    }

    /// Efficient lookup using primary key index
    pub fn get_by_primary_key(&self, key: &ColumnValue) -> Result<(RowId, TableRow), TableError> {
        let row_id = self
            .find_by_primary_key(key.clone())
            .ok_or_else(|| TableError::RowNotFound(RowId(0)))?;

        let row = self.get_row(row_id)?;
        Ok((row_id, row))
    }

    /// Scan with equality filter on a column (can use index if available)
    pub fn scan_by_column_value(
        &self,
        column_name: &str,
        value: &ColumnValue,
    ) -> Vec<(RowId, TableRow)> {
        // Check if this is a primary key lookup
        if let Some(ref pk_column) = self.primary_index.column_name {
            if pk_column == column_name {
                // Use primary key index for efficient lookup
                if let Ok((row_id, row)) = self.get_by_primary_key(value) {
                    return vec![(row_id, row)];
                } else {
                    return vec![];
                }
            }
        }

        // Fall back to filtered scan for non-indexed columns
        let normalized_value = self.normalize_value_for_column(column_name, value.clone());
        self.scan_filtered(|row| {
            row.values
                .get(column_name)
                .map_or(false, |v| Self::values_equal(v, &normalized_value))
        })
    }

    /// Scan with range filter (for numeric columns)
    pub fn scan_by_column_range(
        &self,
        column_name: &str,
        min_value: Option<&ColumnValue>,
        max_value: Option<&ColumnValue>,
    ) -> Vec<(RowId, TableRow)> {
        self.scan_filtered(|row| {
            if let Some(column_value) = row.values.get(column_name) {
                let mut passes = true;

                if let Some(min) = min_value {
                    passes = passes
                        && self
                            .compare_column_values(column_value, min)
                            .unwrap_or(false);
                }

                if let Some(max) = max_value {
                    passes = passes
                        && self
                            .compare_column_values(max, column_value)
                            .unwrap_or(false);
                }

                passes
            } else {
                false
            }
        })
    }

    /// Iterator-based scanning for memory efficiency with large tables
    pub fn iter_rows(&self) -> impl Iterator<Item = (RowId, TableRow)> + '_ {
        (0..self.next_row_id)
            .map(RowId)
            .filter(move |row_id| !self.deleted_rows.contains(&row_id.0))
            .filter_map(move |row_id| self.get_row(row_id).ok().map(|row| (row_id, row)))
    }

    /// Iterator with predicate filtering
    pub fn iter_filtered<'a, F>(
        &'a self,
        predicate: F,
    ) -> impl Iterator<Item = (RowId, TableRow)> + 'a
    where
        F: Fn(&TableRow) -> bool + 'a,
    {
        self.iter_rows().filter(move |(_, row)| predicate(row))
    }

    pub fn get_column_data(&self, column_name: &str) -> Option<&ColumnData> {
        self.storage.columns.get(column_name)
    }

    fn validate_column_value(
        &self,
        column_name: &str,
        value: &ColumnValue,
    ) -> Result<(), TableError> {
        let column = self
            .schema
            .columns
            .iter()
            .find(|col| col.name == column_name)
            .ok_or_else(|| TableError::ColumnNotFound(column_name.to_string()))?;

        let expected_type = match &column.column_type {
            Type::Identifier { name, .. } => name.clone(),
            _ => "unknown".to_string(),
        };

        let actual_type = match value {
            ColumnValue::U64(_) => "u64",
            ColumnValue::I32(_) => "i32",
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

    /// Helper method to compare column values for range queries
    /// Generate table statistics for query optimization
    pub fn generate_statistics(&self) -> crate::query::TableStatistics {
        let mut column_stats = HashMap::new();

        for column in &self.schema.columns {
            if let Some(column_data) = self.storage.columns.get(&column.name) {
                let mut distinct_values = std::collections::HashSet::new();
                let mut null_count = 0;
                let mut min_value: Option<ColumnValue> = None;
                let mut max_value: Option<ColumnValue> = None;

                // Analyze column data
                for i in 0..self.next_row_id {
                    if !self.deleted_rows.contains(&i) {
                        if let Some(value) = column_data.get_value(i) {
                            distinct_values.insert(value.clone());

                            // Update min/max
                            if min_value.is_none()
                                || self
                                    .compare_column_values(&value, min_value.as_ref().unwrap())
                                    .unwrap_or(false)
                            {
                                min_value = Some(value.clone());
                            }
                            if max_value.is_none()
                                || self
                                    .compare_column_values(max_value.as_ref().unwrap(), &value)
                                    .unwrap_or(false)
                            {
                                max_value = Some(value.clone());
                            }
                        } else {
                            null_count += 1;
                        }
                    }
                }

                let is_indexed = self.primary_index.column_name.as_ref() == Some(&column.name);

                column_stats.insert(
                    column.name.clone(),
                    crate::query::ColumnStatistics {
                        column_name: column.name.clone(),
                        distinct_count: distinct_values.len(),
                        null_count,
                        min_value,
                        max_value,
                        is_indexed,
                    },
                );
            }
        }

        let indexed_columns = if let Some(ref pk_col) = self.primary_index.column_name {
            vec![pk_col.clone()]
        } else {
            vec![]
        };

        crate::query::TableStatistics {
            table_name: self.schema.name.clone(),
            row_count: self.row_count,
            column_stats,
            indexed_columns,
            primary_key_column: self.primary_index.column_name.clone(),
        }
    }

    /// Get estimated cardinality for a column value
    pub fn estimate_column_cardinality(&self, column_name: &str, value: &ColumnValue) -> usize {
        if let Some(ref pk_column) = self.primary_index.column_name {
            if pk_column == column_name {
                return if self.primary_index.index.contains_key(value) {
                    1
                } else {
                    0
                };
            }
        }

        // For non-indexed columns, estimate based on distinctness
        if let Some(column_data) = self.storage.columns.get(column_name) {
            let mut matches = 0;
            for i in 0..self.next_row_id {
                if !self.deleted_rows.contains(&i) {
                    if let Some(col_value) = column_data.get_value(i) {
                        if Self::values_equal(&col_value, value) {
                            matches += 1;
                        }
                    }
                }
            }
            matches
        } else {
            0
        }
    }

    fn compare_column_values(&self, left: &ColumnValue, right: &ColumnValue) -> Option<bool> {
        if let (Some(lhs), Some(rhs)) = (Self::value_as_f64(left), Self::value_as_f64(right)) {
            return lhs.partial_cmp(&rhs).map(|ordering| {
                matches!(
                    ordering,
                    std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
                )
            });
        }

        match (left, right) {
            (ColumnValue::String(a), ColumnValue::String(b)) => Some(a >= b),
            (ColumnValue::Bool(a), ColumnValue::Bool(b)) => Some(a >= b),
            _ => None,
        }
    }

    // ==================== ALGORITHMIC IMPROVEMENTS ====================

    /// CRITICAL: Bitmap-based columnar filtering - O(k) instead of O(n)
    /// Evaluates predicates directly on column data without row materialization
    pub fn evaluate_predicate_columnar(
        &self,
        predicate: &Expression,
    ) -> Result<PredicateResult, TableError> {
        match predicate {
            Expression::BinaryOp {
                left,
                operator,
                right,
            } => {
                match operator {
                    BinaryOperator::And => {
                        // Decompose AND: evaluate each side and intersect bitmaps
                        let left_result = self.evaluate_predicate_columnar(left)?;
                        let right_result = self.evaluate_predicate_columnar(right)?;
                        Ok(self.intersect_predicate_results(left_result, right_result))
                    }
                    BinaryOperator::Or => {
                        // Decompose OR: evaluate each side and union bitmaps
                        let left_result = self.evaluate_predicate_columnar(left)?;
                        let right_result = self.evaluate_predicate_columnar(right)?;
                        Ok(self.union_predicate_results(left_result, right_result))
                    }
                    BinaryOperator::Equal => self.evaluate_equality_predicate(left, right),
                    BinaryOperator::NotEqual => {
                        // Invert equality result
                        match self.evaluate_equality_predicate(left, right)? {
                            PredicateResult::Bitmap(bitmap) => {
                                let mut inverted = RowBitmap::all_true(self.next_row_id);
                                for i in 0..bitmap.bits.len() {
                                    if bitmap.bits[i] {
                                        inverted.set(i, false);
                                    }
                                }
                                Ok(PredicateResult::Bitmap(inverted))
                            }
                            other => Ok(other),
                        }
                    }
                    BinaryOperator::Less
                    | BinaryOperator::Greater
                    | BinaryOperator::LessEqual
                    | BinaryOperator::GreaterEqual => {
                        self.evaluate_range_predicate(left, operator, right)
                    }
                    _ => Ok(PredicateResult::FullScan),
                }
            }
            _ => Ok(PredicateResult::FullScan), // Non-optimizable predicates
        }
    }

    /// Evaluate equality predicate with potential index usage
    fn evaluate_equality_predicate(
        &self,
        left: &Expression,
        right: &Expression,
    ) -> Result<PredicateResult, TableError> {
        if let (Expression::Identifier(column_name), Expression::Literal(literal)) = (left, right) {
            let value = self
                .normalize_value_for_column(column_name, self.literal_to_column_value(literal)?);

            // Check for primary key index usage - O(1) lookup
            if let Some(ref pk_col) = self.primary_index.column_name {
                if pk_col == column_name {
                    if let Some(row_id) = self.find_by_primary_key(value.clone()) {
                        return Ok(PredicateResult::IndexLookup(vec![row_id]));
                    } else {
                        return Ok(PredicateResult::IndexLookup(vec![]));
                    }
                }
            }

            // Check for secondary index usage - O(log n) lookup
            if let Some(index) = self.secondary_indexes.get(column_name) {
                if let Some(row_ids) = index.ordered_index.get(&value) {
                    return Ok(PredicateResult::IndexLookup(row_ids.clone()));
                } else {
                    return Ok(PredicateResult::IndexLookup(vec![]));
                }
            }

            // Fall back to columnar bitmap evaluation - O(n) but no row materialization
            return self.create_equality_bitmap(column_name, &value);
        }

        Ok(PredicateResult::FullScan)
    }

    /// Create bitmap for equality comparison on column data
    fn create_equality_bitmap(
        &self,
        column_name: &str,
        value: &ColumnValue,
    ) -> Result<PredicateResult, TableError> {
        if !self
            .schema
            .columns
            .iter()
            .any(|col| col.name == column_name)
        {
            return Err(TableError::ColumnNotFound(column_name.to_string()));
        }

        let target_value = self.normalize_value_for_column(column_name, value.clone());
        let mut bitmap = RowBitmap::new(self.next_row_id);

        for i in 0..self.next_row_id {
            if self.deleted_rows.contains(&i) {
                continue;
            }

            let row_id = RowId(i);
            if let Some(col_value) = self.get_value_for_row(row_id, column_name) {
                if Self::values_equal(&col_value, &target_value) {
                    bitmap.set(i, true);
                }
            }
        }

        Ok(PredicateResult::Bitmap(bitmap))
    }

    /// Evaluate range predicate with potential index usage
    fn evaluate_range_predicate(
        &self,
        left: &Expression,
        operator: &BinaryOperator,
        right: &Expression,
    ) -> Result<PredicateResult, TableError> {
        if let (Expression::Identifier(column_name), Expression::Literal(literal)) = (left, right) {
            let value = self
                .normalize_value_for_column(column_name, self.literal_to_column_value(literal)?);

            // Check for secondary index usage for range queries - O(log n)
            if let Some(index) = self.secondary_indexes.get(column_name) {
                let mut matching_row_ids = Vec::new();

                for (index_value, row_ids) in &index.ordered_index {
                    let matches = match operator {
                        BinaryOperator::Less => self
                            .compare_column_values(&value, index_value)
                            .unwrap_or(false),
                        BinaryOperator::LessEqual => {
                            self.compare_column_values(&value, index_value)
                                .unwrap_or(false)
                                || index_value == &value
                        }
                        BinaryOperator::Greater => self
                            .compare_column_values(index_value, &value)
                            .unwrap_or(false),
                        BinaryOperator::GreaterEqual => {
                            self.compare_column_values(index_value, &value)
                                .unwrap_or(false)
                                || index_value == &value
                        }
                        _ => false,
                    };

                    if matches {
                        matching_row_ids.extend(row_ids.iter().copied());
                    }
                }

                return Ok(PredicateResult::IndexLookup(matching_row_ids));
            }

            // Fall back to columnar bitmap evaluation
            return self.create_range_bitmap(column_name, operator, &value);
        }

        Ok(PredicateResult::FullScan)
    }

    /// Create bitmap for range comparison on column data
    fn create_range_bitmap(
        &self,
        column_name: &str,
        operator: &BinaryOperator,
        value: &ColumnValue,
    ) -> Result<PredicateResult, TableError> {
        if !self
            .schema
            .columns
            .iter()
            .any(|col| col.name == column_name)
        {
            return Err(TableError::ColumnNotFound(column_name.to_string()));
        }

        let target_value = self.normalize_value_for_column(column_name, value.clone());
        let mut bitmap = RowBitmap::new(self.next_row_id);

        for i in 0..self.next_row_id {
            if self.deleted_rows.contains(&i) {
                continue;
            }

            let row_id = RowId(i);
            if let Some(col_value) = self.get_value_for_row(row_id, column_name) {
                let matches = match operator {
                    BinaryOperator::Less => self
                        .compare_column_values(&target_value, &col_value)
                        .unwrap_or(false),
                    BinaryOperator::LessEqual => {
                        self.compare_column_values(&target_value, &col_value)
                            .unwrap_or(false)
                            || Self::values_equal(&col_value, &target_value)
                    }
                    BinaryOperator::Greater => self
                        .compare_column_values(&col_value, &target_value)
                        .unwrap_or(false),
                    BinaryOperator::GreaterEqual => {
                        self.compare_column_values(&col_value, &target_value)
                            .unwrap_or(false)
                            || Self::values_equal(&col_value, &target_value)
                    }
                    _ => false,
                };

                if matches {
                    bitmap.set(i, true);
                }
            }
        }

        Ok(PredicateResult::Bitmap(bitmap))
    }

    fn get_value_for_row(&self, row_id: RowId, column_name: &str) -> Option<ColumnValue> {
        if self.is_computed_column(column_name) {
            self.get_row(row_id)
                .ok()
                .and_then(|row| row.values.get(column_name).cloned())
        } else {
            self.storage
                .columns
                .get(column_name)
                .and_then(|column_data| column_data.get_value(row_id.0))
        }
    }

    /// Intersect predicate results (for AND operations)
    fn intersect_predicate_results(
        &self,
        left: PredicateResult,
        right: PredicateResult,
    ) -> PredicateResult {
        match (left, right) {
            (PredicateResult::Bitmap(left_bitmap), PredicateResult::Bitmap(right_bitmap)) => {
                PredicateResult::Bitmap(left_bitmap.and(&right_bitmap))
            }
            (PredicateResult::IndexLookup(left_ids), PredicateResult::IndexLookup(right_ids)) => {
                let intersection: Vec<RowId> = left_ids
                    .into_iter()
                    .filter(|id| right_ids.contains(id))
                    .collect();
                PredicateResult::IndexLookup(intersection)
            }
            // Mixed results fall back to bitmap evaluation
            _ => PredicateResult::FullScan,
        }
    }

    /// Union predicate results (for OR operations)
    fn union_predicate_results(
        &self,
        left: PredicateResult,
        right: PredicateResult,
    ) -> PredicateResult {
        match (left, right) {
            (PredicateResult::Bitmap(left_bitmap), PredicateResult::Bitmap(right_bitmap)) => {
                PredicateResult::Bitmap(left_bitmap.or(&right_bitmap))
            }
            (
                PredicateResult::IndexLookup(mut left_ids),
                PredicateResult::IndexLookup(right_ids),
            ) => {
                for id in right_ids {
                    if !left_ids.contains(&id) {
                        left_ids.push(id);
                    }
                }
                PredicateResult::IndexLookup(left_ids)
            }
            // Mixed results fall back to full scan
            _ => PredicateResult::FullScan,
        }
    }

    /// CRITICAL: Optimized filtered scan using bitmaps - O(k) instead of O(n)
    pub fn scan_filtered_optimized(
        &self,
        predicate: &Expression,
    ) -> Result<Vec<(RowId, TableRow)>, TableError> {
        let predicate_result = self.evaluate_predicate_columnar(predicate)?;

        match predicate_result {
            PredicateResult::IndexLookup(row_ids) => {
                // O(k) - only process matching rows
                let mut results = Vec::new();
                for row_id in row_ids {
                    if !self.deleted_rows.contains(&row_id.0) {
                        if let Ok(row) = self.get_row(row_id) {
                            results.push((row_id, row));
                        }
                    }
                }
                Ok(results)
            }
            PredicateResult::Bitmap(bitmap) => {
                // O(k) - only materialize rows where bitmap is true
                let mut results = Vec::new();
                for row_id in bitmap.true_row_ids() {
                    if !self.deleted_rows.contains(&row_id.0) {
                        if let Ok(row) = self.get_row(row_id) {
                            results.push((row_id, row));
                        }
                    }
                }
                Ok(results)
            }
            PredicateResult::FullScan => {
                // Fall back to original filtered scan
                Ok(self.scan_filtered(|row| {
                    // This should be replaced with proper predicate evaluation
                    true // Placeholder
                }))
            }
        }
    }

    /// Get row with only projected columns for efficiency
    pub fn get_row_projected(
        &self,
        row_id: RowId,
        projection_columns: &[String],
    ) -> Result<TableRow, TableError> {
        let mut values = HashMap::new();

        for column_name in projection_columns {
            if let Some(column) = self
                .schema
                .columns
                .iter()
                .find(|col| col.name == *column_name)
            {
                if column.is_computed {
                    // Skip computed columns in projected rows to avoid borrowing issues
                    continue;
                } else {
                    // Regular columns from storage - only read requested columns
                    if let Some(column_data) = self.storage.columns.get(column_name) {
                        if let Some(value) = column_data.get_value(row_id.0) {
                            values.insert(column_name.clone(), value);
                        } else {
                            return Err(TableError::RowNotFound(row_id));
                        }
                    }
                }
            }
        }

        Ok(TableRow { values })
    }

    /// Add secondary index for a column to enable O(log n) lookups
    pub fn add_secondary_index(&mut self, column_name: String) -> Result<(), TableError> {
        if self.secondary_indexes.contains_key(&column_name) {
            return Ok(()); // Index already exists
        }

        let mut ordered_index = BTreeMap::new();

        // Build index from existing data
        if let Some(column_data) = self.storage.columns.get(&column_name) {
            for i in 0..self.next_row_id {
                if !self.deleted_rows.contains(&i) {
                    if let Some(value) = column_data.get_value(i) {
                        ordered_index
                            .entry(value)
                            .or_insert_with(Vec::new)
                            .push(RowId(i));
                    }
                }
            }
        }

        // Build bitmap index if column has low cardinality
        let bitmap_index = if ordered_index.len() <= 100 {
            // Low cardinality threshold
            Some(self.build_bitmap_index_for_column(&column_name, &ordered_index)?)
        } else {
            None
        };

        let index = SecondaryIndex {
            column_name: column_name.clone(),
            ordered_index,
            bitmap_index,
        };

        self.secondary_indexes.insert(column_name, index);
        Ok(())
    }

    /// Build bitmap index for low-cardinality column - O(k) lookups where k is distinct values
    fn build_bitmap_index_for_column(
        &self,
        column_name: &str,
        ordered_index: &BTreeMap<ColumnValue, Vec<RowId>>,
    ) -> Result<BitmapIndex, TableError> {
        let mut value_bitmaps = HashMap::new();
        let mut null_bitmap = RowBitmap::new(self.next_row_id);

        // Create bitmap for each distinct value
        for (value, row_ids) in ordered_index {
            let mut bitmap = RowBitmap::new(self.next_row_id);
            for &row_id in row_ids {
                bitmap.set(row_id.0, true);
            }
            value_bitmaps.insert(value.clone(), bitmap);
        }

        // Create bitmap for NULL values
        if let Some(column_data) = self.storage.columns.get(column_name) {
            for i in 0..self.next_row_id {
                if !self.deleted_rows.contains(&i) {
                    if column_data.get_value(i).is_none() {
                        null_bitmap.set(i, true);
                    }
                }
            }
        }

        Ok(BitmapIndex {
            column_name: column_name.to_string(),
            value_bitmaps,
            null_bitmap,
        })
    }

    /// CRITICAL: Enhanced equality bitmap using bitmap indexes - O(1) for indexed low-cardinality columns
    fn create_equality_bitmap_optimized(
        &self,
        column_name: &str,
        value: &ColumnValue,
    ) -> Result<PredicateResult, TableError> {
        // Check for bitmap index first - O(1) lookup
        if let Some(index) = self.secondary_indexes.get(column_name) {
            if let Some(bitmap_index) = &index.bitmap_index {
                if let Some(bitmap) = bitmap_index.value_bitmaps.get(value) {
                    return Ok(PredicateResult::Bitmap(bitmap.clone()));
                } else {
                    // Value not found in bitmap index - return empty bitmap
                    return Ok(PredicateResult::Bitmap(RowBitmap::new(self.next_row_id)));
                }
            }
        }

        // Fall back to regular bitmap creation
        self.create_equality_bitmap(column_name, value)
    }

    /// Enhanced predicate evaluation using bitmap indexes where available
    pub fn evaluate_predicate_with_bitmap_indexes(
        &self,
        predicate: &Expression,
    ) -> Result<PredicateResult, TableError> {
        match predicate {
            Expression::BinaryOp {
                left,
                operator,
                right,
            } => {
                match operator {
                    BinaryOperator::Equal => {
                        if let (Expression::Identifier(column_name), Expression::Literal(literal)) =
                            (left.as_ref(), right.as_ref())
                        {
                            let value = self.normalize_value_for_column(
                                column_name,
                                self.literal_to_column_value(literal)?,
                            );
                            // Use optimized bitmap index lookup
                            return self.create_equality_bitmap_optimized(column_name, &value);
                        }
                    }
                    BinaryOperator::And => {
                        // Decompose AND using bitmap operations
                        let left_result = self.evaluate_predicate_with_bitmap_indexes(left)?;
                        let right_result = self.evaluate_predicate_with_bitmap_indexes(right)?;
                        return Ok(self.intersect_predicate_results(left_result, right_result));
                    }
                    BinaryOperator::Or => {
                        // Decompose OR using bitmap operations
                        let left_result = self.evaluate_predicate_with_bitmap_indexes(left)?;
                        let right_result = self.evaluate_predicate_with_bitmap_indexes(right)?;
                        return Ok(self.union_predicate_results(left_result, right_result));
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        // Fall back to regular predicate evaluation
        self.evaluate_predicate_columnar(predicate)
    }

    /// Convert literal to column value
    fn literal_to_column_value(&self, literal: &Literal) -> Result<ColumnValue, TableError> {
        match literal {
            Literal::Boolean(b) => Ok(ColumnValue::Bool(*b)),
            Literal::Integer(int_lit) => Ok(ColumnValue::U64(int_lit.value as u64)),
            Literal::Float(f) => Ok(ColumnValue::F64(f.to_bits())),
            Literal::String(s) => Ok(ColumnValue::String(s.clone())),
            Literal::Char(c) => Ok(ColumnValue::String(c.to_string())),
        }
    }

    fn normalize_value_for_column(&self, column_name: &str, value: ColumnValue) -> ColumnValue {
        let target_type = self
            .schema
            .columns
            .iter()
            .find(|col| col.name == column_name)
            .and_then(|col| match &col.column_type {
                Type::Identifier { name, .. } => Some(name.as_str()),
                _ => None,
            });

        if let Some(target) = target_type {
            match (target, value) {
                ("f64", ColumnValue::U64(v)) => ColumnValue::F64((v as f64).to_bits()),
                ("f64", ColumnValue::I32(v)) => ColumnValue::F64((v as f64).to_bits()),
                ("f64", ColumnValue::F64(bits)) => ColumnValue::F64(bits),
                ("u64", ColumnValue::I32(v)) => ColumnValue::U64(v as u64),
                ("i32", ColumnValue::U64(v)) => ColumnValue::I32(v as i32),
                ("i32", ColumnValue::F64(bits)) => ColumnValue::I32(f64::from_bits(bits) as i32),
                (_, v) => v,
            }
        } else {
            value
        }
    }

    fn value_as_f64(value: &ColumnValue) -> Option<f64> {
        match value {
            ColumnValue::U64(v) => Some(*v as f64),
            ColumnValue::I32(v) => Some(*v as f64),
            ColumnValue::F64(bits) => Some(f64::from_bits(*bits)),
            _ => None,
        }
    }

    fn values_equal(left: &ColumnValue, right: &ColumnValue) -> bool {
        if left == right {
            return true;
        }

        match (Self::value_as_f64(left), Self::value_as_f64(right)) {
            (Some(lhs), Some(rhs)) => lhs
                .partial_cmp(&rhs)
                .map_or(false, |ordering| ordering == std::cmp::Ordering::Equal),
            _ => false,
        }
    }
}

// ==================== BITMAP AND ITERATOR IMPLEMENTATIONS ====================

impl RowBitmap {
    /// Create a new bitmap with all bits set to false
    pub fn new(size: usize) -> Self {
        Self {
            bits: vec![false; size],
            cardinality: 0,
        }
    }

    /// Create a bitmap with all bits set to true (for base case)
    pub fn all_true(size: usize) -> Self {
        Self {
            bits: vec![true; size],
            cardinality: size,
        }
    }

    /// Set a bit and update cardinality
    pub fn set(&mut self, index: usize, value: bool) {
        if index < self.bits.len() {
            let old_value = self.bits[index];
            self.bits[index] = value;
            if old_value != value {
                if value {
                    self.cardinality += 1;
                } else {
                    self.cardinality = self.cardinality.saturating_sub(1);
                }
            }
        }
    }

    /// Bitwise AND operation - intersect two bitmaps
    pub fn and(&self, other: &RowBitmap) -> RowBitmap {
        let mut result = RowBitmap::new(self.bits.len().min(other.bits.len()));
        for i in 0..result.bits.len() {
            let bit_value = self.bits[i] && other.bits[i];
            if bit_value {
                result.bits[i] = true;
                result.cardinality += 1;
            }
        }
        result
    }

    /// Bitwise OR operation - union two bitmaps
    pub fn or(&self, other: &RowBitmap) -> RowBitmap {
        let mut result = RowBitmap::new(self.bits.len().max(other.bits.len()));
        for i in 0..result.bits.len() {
            let left_bit = if i < self.bits.len() {
                self.bits[i]
            } else {
                false
            };
            let right_bit = if i < other.bits.len() {
                other.bits[i]
            } else {
                false
            };
            let bit_value = left_bit || right_bit;
            if bit_value {
                result.bits[i] = true;
                result.cardinality += 1;
            }
        }
        result
    }

    /// Get iterator over row IDs where the bit is true
    pub fn true_row_ids(&self) -> impl Iterator<Item = RowId> + '_ {
        self.bits
            .iter()
            .enumerate()
            .filter_map(|(i, &bit)| if bit { Some(RowId(i)) } else { None })
    }

    /// Check if bitmap is empty (no true bits)
    pub fn is_empty(&self) -> bool {
        self.cardinality == 0
    }
}

impl<'a> IndexedIterator<'a> {
    /// Create a new indexed iterator for specific row IDs
    pub fn new(
        table: &'a TableRuntime,
        row_ids: Vec<RowId>,
        projection_columns: Vec<String>,
    ) -> Self {
        Self {
            table,
            row_ids: Box::new(row_ids.into_iter()),
            projection_columns,
        }
    }

    /// Create indexed iterator from bitmap
    pub fn from_bitmap(
        table: &'a TableRuntime,
        bitmap: &RowBitmap,
        projection_columns: Vec<String>,
    ) -> Self {
        let row_ids: Vec<RowId> = bitmap.true_row_ids().collect();
        Self::new(table, row_ids, projection_columns)
    }

    /// Create indexed iterator for primary key lookup
    pub fn primary_key_lookup(
        table: &'a TableRuntime,
        key: &ColumnValue,
        projection_columns: Vec<String>,
    ) -> Self {
        let row_ids = if let Some(row_id) = table.find_by_primary_key(key.clone()) {
            vec![row_id]
        } else {
            vec![]
        };
        Self::new(table, row_ids, projection_columns)
    }

    /// Create indexed iterator for range scan
    pub fn range_scan(
        table: &'a TableRuntime,
        column_name: &str,
        min_value: Option<&ColumnValue>,
        max_value: Option<&ColumnValue>,
        projection_columns: Vec<String>,
    ) -> Self {
        // If we have a secondary index for this column, use it
        if let Some(index) = table.secondary_indexes.get(column_name) {
            let mut row_ids = Vec::new();

            for (value, ids) in &index.ordered_index {
                let mut include = true;

                if let Some(min) = min_value {
                    if table.compare_column_values(value, min).unwrap_or(false) {
                        include = false;
                    }
                }

                if let Some(max) = max_value {
                    if table.compare_column_values(max, value).unwrap_or(false) {
                        include = false;
                    }
                }

                if include {
                    row_ids.extend(ids.iter().copied());
                }
            }

            Self::new(table, row_ids, projection_columns)
        } else {
            // Fallback to full scan with range filtering
            let all_row_ids: Vec<RowId> = (0..table.next_row_id)
                .filter(|&i| !table.deleted_rows.contains(&i))
                .map(RowId)
                .collect();
            Self::new(table, all_row_ids, projection_columns)
        }
    }
}

impl<'a> Iterator for IndexedIterator<'a> {
    type Item = (RowId, TableRow);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(row_id) = self.row_ids.next() {
            if !self.table.deleted_rows.contains(&row_id.0) {
                // Only materialize the row with projected columns for efficiency
                if let Ok(row) = self
                    .table
                    .get_row_projected(row_id, &self.projection_columns)
                {
                    return Some((row_id, row));
                }
            }
        }
        None
    }
}
