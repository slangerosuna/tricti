use crate::ast::*;
use crate::table_runtime::*;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedColumnDependency {
    pub column_name: String,
    pub depends_on: HashSet<String>,
    pub expression: Expression,
}

#[derive(Debug, Clone)]
pub struct DependencyGraph {
    pub dependencies: HashMap<String, ComputedColumnDependency>,
    pub evaluation_order: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum DependencyError {
    CircularDependency(Vec<String>),
    UnknownColumnReference(String),
    InvalidExpression(String),
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            evaluation_order: Vec::new(),
        }
    }

    /// Analyze a table schema and build dependency graph for computed columns
    pub fn build_from_table(table: &TableDef) -> Result<Self, DependencyError> {
        let mut graph = Self::new();
        let mut all_column_names: HashSet<String> = HashSet::new();

        // Collect all column names (regular + computed)
        for column in &table.columns {
            all_column_names.insert(column.name.clone());
        }

        // Analyze each computed column
        for column in &table.columns {
            if column.is_computed {
                if let Some(expr) = &column.computed_expression {
                    let dependencies = Self::extract_column_dependencies(expr, &all_column_names)?;

                    // Note: No need to validate dependencies anymore since we only extract valid column references

                    graph.dependencies.insert(
                        column.name.clone(),
                        ComputedColumnDependency {
                            column_name: column.name.clone(),
                            depends_on: dependencies,
                            expression: expr.clone(),
                        },
                    );
                }
            }
        }

        // Build evaluation order using topological sort
        graph.evaluation_order = graph.topological_sort()?;

        Ok(graph)
    }

    /// Extract column names referenced in an expression
    fn extract_column_dependencies(
        expr: &Expression,
        column_names: &HashSet<String>,
    ) -> Result<HashSet<String>, DependencyError> {
        let mut dependencies = HashSet::new();
        Self::extract_dependencies_recursive(expr, &mut dependencies, column_names)?;
        Ok(dependencies)
    }

    fn extract_dependencies_recursive(
        expr: &Expression,
        dependencies: &mut HashSet<String>,
        column_names: &HashSet<String>,
    ) -> Result<(), DependencyError> {
        match expr {
            Expression::Identifier(name) => {
                // Only treat identifiers as column references if they exist in the table's columns
                if column_names.contains(name) {
                    dependencies.insert(name.clone());
                }
                // Otherwise, it's a function name, constant, or other identifier - not a column dependency
            }
            Expression::BinaryOp { left, right, .. } => {
                Self::extract_dependencies_recursive(left, dependencies, column_names)?;
                Self::extract_dependencies_recursive(right, dependencies, column_names)?;
            }
            Expression::UnaryOp { operand, .. } => {
                Self::extract_dependencies_recursive(operand, dependencies, column_names)?;
            }
            Expression::Call {
                function,
                arguments,
                ..
            } => {
                // For function calls, don't recurse into the function identifier (it's not a column dependency)
                // Only recurse into arguments to find column dependencies
                for arg in arguments {
                    Self::extract_dependencies_recursive(&arg.value, dependencies, column_names)?;
                }
            }
            Expression::FieldAccess { object, .. } => {
                Self::extract_dependencies_recursive(object, dependencies, column_names)?;
            }
            Expression::Index { object, indices } => {
                Self::extract_dependencies_recursive(object, dependencies, column_names)?;
                for index in indices {
                    Self::extract_dependencies_recursive(index, dependencies, column_names)?;
                }
            }
            Expression::If {
                condition,
                then_branch,
                else_branch,
            } => {
                Self::extract_dependencies_recursive(condition, dependencies, column_names)?;
                for stmt in then_branch {
                    Self::extract_statement_dependencies(stmt, dependencies, column_names)?;
                }
                if let Some(else_stmts) = else_branch {
                    for stmt in else_stmts {
                        Self::extract_statement_dependencies(stmt, dependencies, column_names)?;
                    }
                }
            }
            Expression::Block { statements } => {
                for stmt in statements {
                    Self::extract_statement_dependencies(stmt, dependencies, column_names)?;
                }
            }
            Expression::UnsafeBlock { statements } => {
                for stmt in statements {
                    Self::extract_statement_dependencies(stmt, dependencies, column_names)?;
                }
            }
            Expression::Tuple(exprs) => {
                for expr in exprs {
                    Self::extract_dependencies_recursive(expr, dependencies, column_names)?;
                }
            }
            Expression::Match { value, arms } => {
                Self::extract_dependencies_recursive(value, dependencies, column_names)?;
                for arm in arms {
                    Self::extract_dependencies_recursive(&arm.pattern, dependencies, column_names)?;
                    Self::extract_dependencies_recursive(&arm.body, dependencies, column_names)?;
                }
            }
            Expression::StructLiteral { type_name, fields } => {
                for expr in fields.values() {
                    Self::extract_dependencies_recursive(expr, dependencies, column_names)?;
                }
            }
            Expression::ArrayNew { dimensions, .. } => {
                for expr in dimensions {
                    Self::extract_dependencies_recursive(expr, dependencies, column_names)?;
                }
            }
            Expression::Matrix { rows } => {
                for row in rows {
                    for expr in row {
                        Self::extract_dependencies_recursive(expr, dependencies, column_names)?;
                    }
                }
            }
            Expression::Range { start, end, step } => {
                Self::extract_dependencies_recursive(start, dependencies, column_names)?;
                Self::extract_dependencies_recursive(end, dependencies, column_names)?;
                if let Some(step_expr) = step {
                    Self::extract_dependencies_recursive(step_expr, dependencies, column_names)?;
                }
            }
            Expression::Question(expr) | Expression::Unwrap(expr) => {
                Self::extract_dependencies_recursive(expr, dependencies, column_names)?;
            }
            Expression::Cast { value, .. } => {
                Self::extract_dependencies_recursive(value, dependencies, column_names)?;
            }
            Expression::Query(_) => {
                // Query expressions might have dependencies, but for now we'll treat them as having none
                // TODO: Implement proper dependency extraction for query expressions
            }
            // Literals and other leaf nodes don't have dependencies
            Expression::Literal(_)
            | Expression::Loop { .. }
            | Expression::Function { .. }
            | Expression::Shader { .. }
            | Expression::StaticPath { .. } => {}
        }
        Ok(())
    }

    fn extract_statement_dependencies(
        stmt: &Statement,
        dependencies: &mut HashSet<String>,
        column_names: &HashSet<String>,
    ) -> Result<(), DependencyError> {
        match stmt {
            Statement::VariableDecl { value, .. } => {
                Self::extract_dependencies_recursive(value, dependencies, column_names)?;
            }
            Statement::ConstDecl {
                value: ConstValue::Expression(expr),
                ..
            } => {
                Self::extract_dependencies_recursive(expr, dependencies, column_names)?;
            }
            Statement::Assignment { target, value, .. } => {
                Self::extract_dependencies_recursive(target, dependencies, column_names)?;
                Self::extract_dependencies_recursive(value, dependencies, column_names)?;
            }
            Statement::Expression(expr) => {
                Self::extract_dependencies_recursive(expr, dependencies, column_names)?;
            }
            Statement::Return(Some(expr)) | Statement::Break(Some(expr)) => {
                Self::extract_dependencies_recursive(expr, dependencies, column_names)?;
            }
            Statement::ForLoop { iterable, body, .. } => {
                Self::extract_dependencies_recursive(iterable, dependencies, column_names)?;
                for stmt in body {
                    Self::extract_statement_dependencies(stmt, dependencies, column_names)?;
                }
            }
            _ => {} // Other statements don't contribute to dependencies
        }
        Ok(())
    }

    /// Perform topological sort to determine evaluation order
    fn topological_sort(&self) -> Result<Vec<String>, DependencyError> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

        // Initialize all computed columns
        for column_name in self.dependencies.keys() {
            in_degree.insert(column_name.clone(), 0);
            adjacency.insert(column_name.clone(), Vec::new());
        }

        // Build adjacency list and calculate in-degrees
        for (column, deps) in &self.dependencies {
            for dependency in &deps.depends_on {
                // Only consider dependencies on computed columns for ordering
                if self.dependencies.contains_key(dependency) {
                    adjacency.get_mut(dependency).unwrap().push(column.clone());
                    *in_degree.get_mut(column).unwrap() += 1;
                }
            }
        }

        // Kahn's algorithm for topological sorting
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut result: Vec<String> = Vec::new();

        // Start with nodes that have no dependencies
        for (column, &degree) in &in_degree {
            if degree == 0 {
                queue.push_back(column.clone());
            }
        }

        while let Some(current) = queue.pop_front() {
            result.push(current.clone());

            // Process all dependents
            for dependent in &adjacency[&current] {
                let degree = in_degree.get_mut(dependent).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(dependent.clone());
                }
            }
        }

        // Check for circular dependencies
        if result.len() != self.dependencies.len() {
            // Find the nodes involved in the cycle
            let mut cycle_nodes = Vec::new();
            for (column, &degree) in &in_degree {
                if degree > 0 {
                    cycle_nodes.push(column.clone());
                }
            }
            return Err(DependencyError::CircularDependency(cycle_nodes));
        }

        Ok(result)
    }

    /// Get the dependencies for a specific column
    pub fn get_dependencies(&self, column_name: &str) -> Option<&HashSet<String>> {
        self.dependencies
            .get(column_name)
            .map(|dep| &dep.depends_on)
    }

    /// Check if a column is computed
    pub fn is_computed_column(&self, column_name: &str) -> bool {
        self.dependencies.contains_key(column_name)
    }

    /// Get all columns that depend on a given column
    pub fn get_dependents(&self, column_name: &str) -> Vec<String> {
        let mut dependents = Vec::new();
        for (col, deps) in &self.dependencies {
            if deps.depends_on.contains(column_name) {
                dependents.push(col.clone());
            }
        }
        dependents
    }

    /// Get the evaluation order for computed columns
    pub fn get_evaluation_order(&self) -> &[String] {
        &self.evaluation_order
    }
}

/// Lazy evaluation engine for computed columns
#[derive(Debug, Clone)]
pub struct LazyEvaluationEngine {
    pub dependency_graph: DependencyGraph,
    pub cached_values: HashMap<String, HashMap<RowId, ColumnValue>>, // column_name -> row_id -> value
    pub dirty_columns: HashSet<String>, // columns that need recomputation
    pub dirty_rows: HashMap<String, HashSet<RowId>>, // column_name -> set of dirty row_ids
}

#[derive(Debug, Clone)]
pub enum EvaluationError {
    DependencyError(DependencyError),
    ExpressionEvaluationError(String),
    TypeMismatchError(String),
    MissingDependencyValue(String, RowId),
}

impl From<DependencyError> for EvaluationError {
    fn from(err: DependencyError) -> Self {
        EvaluationError::DependencyError(err)
    }
}

impl LazyEvaluationEngine {
    pub fn new(table: &TableDef) -> Result<Self, EvaluationError> {
        let dependency_graph = DependencyGraph::build_from_table(table)?;

        Ok(Self {
            dependency_graph,
            cached_values: HashMap::new(),
            dirty_columns: HashSet::new(),
            dirty_rows: HashMap::new(),
        })
    }

    /// Get the computed value for a column at a specific row
    pub fn get_computed_value(
        &mut self,
        column_name: &str,
        row_id: RowId,
        table_data: &ColumnarStorage,
    ) -> Result<ColumnValue, EvaluationError> {
        // Check if value is cached and not dirty
        if let Some(column_cache) = self.cached_values.get(column_name) {
            if let Some(cached_value) = column_cache.get(&row_id) {
                let is_dirty = self
                    .dirty_rows
                    .get(column_name)
                    .map_or(false, |dirty_set| dirty_set.contains(&row_id));

                if !is_dirty && !self.dirty_columns.contains(column_name) {
                    return Ok(cached_value.clone());
                }
            }
        }

        // Compute the value
        let computed_value = self.compute_column_value(column_name, row_id, table_data)?;

        // Cache the computed value
        self.cached_values
            .entry(column_name.to_string())
            .or_insert_with(HashMap::new)
            .insert(row_id, computed_value.clone());

        // Mark as clean
        self.dirty_columns.remove(column_name);
        if let Some(dirty_set) = self.dirty_rows.get_mut(column_name) {
            dirty_set.remove(&row_id);
        }

        Ok(computed_value)
    }

    /// Compute the value for a column at a specific row
    fn compute_column_value(
        &mut self,
        column_name: &str,
        row_id: RowId,
        table_data: &ColumnarStorage,
    ) -> Result<ColumnValue, EvaluationError> {
        let expression = self
            .dependency_graph
            .dependencies
            .get(column_name)
            .ok_or_else(|| {
                EvaluationError::ExpressionEvaluationError(format!(
                    "Column '{}' is not a computed column",
                    column_name
                ))
            })?
            .expression
            .clone();

        self.evaluate_expression(&expression, row_id, table_data)
    }

    /// Evaluate an expression for a specific row
    fn evaluate_expression(
        &mut self,
        expr: &Expression,
        row_id: RowId,
        table_data: &ColumnarStorage,
    ) -> Result<ColumnValue, EvaluationError> {
        match expr {
            Expression::Literal(literal) => Ok(self.literal_to_column_value(literal)?),
            Expression::Identifier(column_name) => {
                self.get_column_value(column_name, row_id, table_data)
            }
            Expression::BinaryOp {
                left,
                operator,
                right,
            } => {
                let left_val = self.evaluate_expression(left, row_id, table_data)?;
                let right_val = self.evaluate_expression(right, row_id, table_data)?;
                self.apply_binary_operator(&left_val, operator, &right_val)
            }
            Expression::UnaryOp { operator, operand } => {
                let operand_val = self.evaluate_expression(operand, row_id, table_data)?;
                self.apply_unary_operator(operator, &operand_val)
            }
            Expression::Call {
                function,
                arguments,
                ..
            } => self.evaluate_function_call(function, arguments, row_id, table_data),
            _ => Err(EvaluationError::ExpressionEvaluationError(
                "Unsupported expression type in computed column".to_string(),
            )),
        }
    }

    /// Get the value of a column (regular or computed) for a specific row
    fn get_column_value(
        &mut self,
        column_name: &str,
        row_id: RowId,
        table_data: &ColumnarStorage,
    ) -> Result<ColumnValue, EvaluationError> {
        // Check if it's a computed column
        if self.dependency_graph.is_computed_column(column_name) {
            return self.get_computed_value(column_name, row_id, table_data);
        }

        // Get value from regular column storage
        let column_data = table_data.columns.get(column_name).ok_or_else(|| {
            EvaluationError::MissingDependencyValue(column_name.to_string(), row_id)
        })?;

        column_data
            .get_value(row_id.0)
            .ok_or_else(|| EvaluationError::MissingDependencyValue(column_name.to_string(), row_id))
    }

    /// Convert a literal to a column value
    fn literal_to_column_value(&self, literal: &Literal) -> Result<ColumnValue, EvaluationError> {
        match literal {
            Literal::Integer(int_lit) => Ok(ColumnValue::U64(int_lit.value as u64)),
            Literal::Float(f) => Ok(ColumnValue::F64(f.to_bits())),
            Literal::String(s) => Ok(ColumnValue::String(s.clone())),
            Literal::Boolean(b) => Ok(ColumnValue::Bool(*b)),
            Literal::Char(c) => Ok(ColumnValue::String(c.to_string())),
        }
    }

    /// Apply a binary operator to two column values
    fn apply_binary_operator(
        &self,
        left: &ColumnValue,
        operator: &BinaryOperator,
        right: &ColumnValue,
    ) -> Result<ColumnValue, EvaluationError> {
        match (left, right) {
            (ColumnValue::U64(l), ColumnValue::U64(r)) => match operator {
                BinaryOperator::Add => Ok(ColumnValue::U64(l + r)),
                BinaryOperator::Sub => Ok(ColumnValue::U64(l - r)),
                BinaryOperator::Mul => Ok(ColumnValue::U64(l * r)),
                BinaryOperator::Div => Ok(ColumnValue::U64(l / r)),
                BinaryOperator::Equal => Ok(ColumnValue::Bool(l == r)),
                BinaryOperator::NotEqual => Ok(ColumnValue::Bool(l != r)),
                BinaryOperator::Less => Ok(ColumnValue::Bool(l < r)),
                BinaryOperator::LessEqual => Ok(ColumnValue::Bool(l <= r)),
                BinaryOperator::Greater => Ok(ColumnValue::Bool(l > r)),
                BinaryOperator::GreaterEqual => Ok(ColumnValue::Bool(l >= r)),
                _ => Err(EvaluationError::ExpressionEvaluationError(format!(
                    "Unsupported operator {:?} for u64 values",
                    operator
                ))),
            },
            (ColumnValue::F64(l_bits), ColumnValue::F64(r_bits)) => {
                let l = f64::from_bits(*l_bits);
                let r = f64::from_bits(*r_bits);
                match operator {
                    BinaryOperator::Add => Ok(ColumnValue::F64((l + r).to_bits())),
                    BinaryOperator::Sub => Ok(ColumnValue::F64((l - r).to_bits())),
                    BinaryOperator::Mul => Ok(ColumnValue::F64((l * r).to_bits())),
                    BinaryOperator::Div => Ok(ColumnValue::F64((l / r).to_bits())),
                    BinaryOperator::Equal => Ok(ColumnValue::Bool(l == r)),
                    BinaryOperator::NotEqual => Ok(ColumnValue::Bool(l != r)),
                    BinaryOperator::Less => Ok(ColumnValue::Bool(l < r)),
                    BinaryOperator::LessEqual => Ok(ColumnValue::Bool(l <= r)),
                    BinaryOperator::Greater => Ok(ColumnValue::Bool(l > r)),
                    BinaryOperator::GreaterEqual => Ok(ColumnValue::Bool(l >= r)),
                    _ => Err(EvaluationError::ExpressionEvaluationError(format!(
                        "Unsupported operator {:?} for f64 values",
                        operator
                    ))),
                }
            }
            (ColumnValue::String(l), ColumnValue::String(r)) => match operator {
                BinaryOperator::Add => Ok(ColumnValue::String(format!("{}{}", l, r))),
                BinaryOperator::Equal => Ok(ColumnValue::Bool(l == r)),
                BinaryOperator::NotEqual => Ok(ColumnValue::Bool(l != r)),
                _ => Err(EvaluationError::ExpressionEvaluationError(format!(
                    "Unsupported operator {:?} for string values",
                    operator
                ))),
            },
            (ColumnValue::Bool(l), ColumnValue::Bool(r)) => match operator {
                BinaryOperator::And => Ok(ColumnValue::Bool(*l && *r)),
                BinaryOperator::Or => Ok(ColumnValue::Bool(*l || *r)),
                BinaryOperator::Equal => Ok(ColumnValue::Bool(l == r)),
                BinaryOperator::NotEqual => Ok(ColumnValue::Bool(l != r)),
                _ => Err(EvaluationError::ExpressionEvaluationError(format!(
                    "Unsupported operator {:?} for boolean values",
                    operator
                ))),
            },
            _ => Err(EvaluationError::TypeMismatchError(format!(
                "Cannot apply operator {:?} to values {:?} and {:?}",
                operator, left, right
            ))),
        }
    }

    /// Apply a unary operator to a column value
    fn apply_unary_operator(
        &self,
        operator: &UnaryOperator,
        operand: &ColumnValue,
    ) -> Result<ColumnValue, EvaluationError> {
        match (operator, operand) {
            (UnaryOperator::Negate, ColumnValue::U64(val)) => {
                // Convert to signed and negate
                Ok(ColumnValue::U64((-(*val as i64)) as u64))
            }
            (UnaryOperator::Negate, ColumnValue::F64(bits)) => {
                let val = f64::from_bits(*bits);
                Ok(ColumnValue::F64((-val).to_bits()))
            }
            (UnaryOperator::Not, ColumnValue::Bool(val)) => Ok(ColumnValue::Bool(!val)),
            _ => Err(EvaluationError::ExpressionEvaluationError(format!(
                "Unsupported unary operator {:?} for value {:?}",
                operator, operand
            ))),
        }
    }

    /// Evaluate a function call
    fn evaluate_function_call(
        &mut self,
        function: &Expression,
        arguments: &[Argument],
        row_id: RowId,
        table_data: &ColumnarStorage,
    ) -> Result<ColumnValue, EvaluationError> {
        if let Expression::Identifier(func_name) = function {
            match func_name.as_str() {
                "len" => {
                    if arguments.len() != 1 {
                        return Err(EvaluationError::ExpressionEvaluationError(
                            "len() function expects exactly one argument".to_string(),
                        ));
                    }
                    let arg_val =
                        self.evaluate_expression(&arguments[0].value, row_id, table_data)?;
                    match arg_val {
                        ColumnValue::String(s) => Ok(ColumnValue::U64(s.len() as u64)),
                        _ => Err(EvaluationError::TypeMismatchError(
                            "len() function expects a string argument".to_string(),
                        )),
                    }
                }
                "abs" => {
                    if arguments.len() != 1 {
                        return Err(EvaluationError::ExpressionEvaluationError(
                            "abs() function expects exactly one argument".to_string(),
                        ));
                    }
                    let arg_val =
                        self.evaluate_expression(&arguments[0].value, row_id, table_data)?;
                    match arg_val {
                        ColumnValue::F64(bits) => {
                            let val = f64::from_bits(bits);
                            Ok(ColumnValue::F64(val.abs().to_bits()))
                        }
                        ColumnValue::U64(val) => Ok(ColumnValue::U64(val)), // Already positive
                        _ => Err(EvaluationError::TypeMismatchError(
                            "abs() function expects a numeric argument".to_string(),
                        )),
                    }
                }
                _ => Err(EvaluationError::ExpressionEvaluationError(format!(
                    "Unknown function: {}",
                    func_name
                ))),
            }
        } else {
            Err(EvaluationError::ExpressionEvaluationError(
                "Function calls must use identifier expressions".to_string(),
            ))
        }
    }

    /// Mark a column as dirty (needs recomputation)
    pub fn mark_column_dirty(&mut self, column_name: &str) {
        self.dirty_columns.insert(column_name.to_string());

        // Also mark all dependent columns as dirty
        let dependents = self.dependency_graph.get_dependents(column_name);
        for dependent in dependents {
            self.dirty_columns.insert(dependent);
        }
    }

    /// Mark a specific row in a column as dirty
    pub fn mark_row_dirty(&mut self, column_name: &str, row_id: RowId) {
        self.dirty_rows
            .entry(column_name.to_string())
            .or_insert_with(HashSet::new)
            .insert(row_id);

        // Also mark dependent columns and rows as dirty
        let dependents = self.dependency_graph.get_dependents(column_name);
        for dependent in dependents {
            self.dirty_rows
                .entry(dependent)
                .or_insert_with(HashSet::new)
                .insert(row_id);
        }
    }

    /// Clear all cached values and mark everything as dirty
    pub fn invalidate_all(&mut self) {
        self.cached_values.clear();
        self.dirty_columns.clear();
        self.dirty_rows.clear();

        // Mark all computed columns as dirty
        for column_name in self.dependency_graph.dependencies.keys() {
            self.dirty_columns.insert(column_name.clone());
        }
    }

    /// Get statistics about the cache
    pub fn get_cache_stats(&self) -> HashMap<String, usize> {
        let mut stats = HashMap::new();
        for (column, cache) in &self.cached_values {
            stats.insert(column.clone(), cache.len());
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_dependency_extraction() {
        let expr = Expression::BinaryOp {
            left: Box::new(Expression::Identifier("price".to_string())),
            operator: BinaryOperator::Mul,
            right: Box::new(Expression::Identifier("tax_rate".to_string())),
        };

        let mut column_names = HashSet::new();
        column_names.insert("price".to_string());
        column_names.insert("tax_rate".to_string());

        let deps = DependencyGraph::extract_column_dependencies(&expr, &column_names).unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.contains("price"));
        assert!(deps.contains("tax_rate"));
    }

    #[test]
    fn test_dependency_graph_building() {
        let table = TableDef {
            name: "Orders".to_string(),
            columns: vec![
                TableColumn {
                    name: "quantity".to_string(),
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
                    name: "unit_price".to_string(),
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
                    name: "subtotal".to_string(),
                    column_type: Type::None,
                    annotations: vec![],
                    default_value: None,
                    is_computed: true,
                    computed_expression: Some(Expression::BinaryOp {
                        left: Box::new(Expression::Identifier("quantity".to_string())),
                        operator: BinaryOperator::Mul,
                        right: Box::new(Expression::Identifier("unit_price".to_string())),
                    }),
                },
            ],
        };

        let graph = DependencyGraph::build_from_table(&table).unwrap();
        assert_eq!(graph.dependencies.len(), 1);
        assert!(graph.dependencies.contains_key("subtotal"));

        let subtotal_deps = graph.get_dependencies("subtotal").unwrap();
        assert_eq!(subtotal_deps.len(), 2);
        assert!(subtotal_deps.contains("quantity"));
        assert!(subtotal_deps.contains("unit_price"));
    }

    #[test]
    fn test_circular_dependency_detection() {
        let table = TableDef {
            name: "Circular".to_string(),
            columns: vec![
                TableColumn {
                    name: "a".to_string(),
                    column_type: Type::None,
                    annotations: vec![],
                    default_value: None,
                    is_computed: true,
                    computed_expression: Some(Expression::Identifier("b".to_string())),
                },
                TableColumn {
                    name: "b".to_string(),
                    column_type: Type::None,
                    annotations: vec![],
                    default_value: None,
                    is_computed: true,
                    computed_expression: Some(Expression::Identifier("a".to_string())),
                },
            ],
        };

        let result = DependencyGraph::build_from_table(&table);
        assert!(result.is_err());
        if let Err(DependencyError::CircularDependency(cycle)) = result {
            assert_eq!(cycle.len(), 2);
        } else {
            panic!("Expected circular dependency error");
        }
    }
}
