use crate::ast::{
    BindingPattern, Expression, ResourceAccess, Statement, SystemDef, SystemParameter,
};
use crate::async_runtime::{AsyncExecutionError, SystemExecutionResult, TaskId, YieldPoint};
use crate::semantic::SemanticContext;
use crate::table_runtime::{ColumnValue, RowId, TableRuntime};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// State machine representation of a system execution
#[derive(Debug, Clone)]
pub struct SystemStateMachine {
    pub system_name: String,
    pub states: Vec<ExecutionState>,
    pub current_state: usize,
    pub execution_context: ExecutionContext,
    pub parameters: HashMap<String, ColumnValue>,
}

/// Individual state in the system execution state machine
#[derive(Debug, Clone)]
pub enum ExecutionState {
    /// Entry point - setup and resource acquisition
    Initialize { resources_to_acquire: Vec<String> },
    /// Execute a sequence of statements
    ExecuteStatements {
        statements: Vec<Statement>,
        current_statement: usize,
    },
    /// Await a resource to become available
    AwaitResource {
        resource_name: String,
        access_type: ResourceAccess,
        next_state: usize,
    },
    /// Execute a query against a table
    ExecuteQuery {
        table_name: String,
        query: QueryExecution,
        result_variable: String,
        next_state: usize,
    },
    /// Await completion of another system
    AwaitSystem {
        system_name: String,
        task_id: TaskId,
        next_state: usize,
    },
    /// Sleep for a specified duration
    Sleep {
        duration: Duration,
        next_state: usize,
    },
    /// Handle an error condition
    HandleError {
        error: AsyncExecutionError,
        recovery_state: Option<usize>,
    },
    /// Final state - cleanup and return
    Complete {
        return_value: Option<ColumnValue>,
        resources_to_release: Vec<String>,
    },
}

/// Query execution details
#[derive(Debug, Clone)]
pub enum QueryExecution {
    Select {
        columns: Vec<String>,
        where_clause: Option<Expression>,
    },
    Insert {
        values: HashMap<String, ColumnValue>,
    },
    Update {
        row_id: RowId,
        updates: HashMap<String, ColumnValue>,
    },
    Delete {
        row_id: RowId,
    },
}

/// Execution context maintaining state during system execution
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// Local variables and their values
    pub variables: HashMap<String, ColumnValue>,
    /// Stack for nested scopes
    pub scope_stack: Vec<HashMap<String, ColumnValue>>,
    /// Resources currently held by this execution
    pub held_resources: HashMap<String, ResourceAccess>,
    /// Tables accessed during execution
    pub accessed_tables: Vec<String>,
    /// Execution start time for timeout tracking
    pub started_at: Instant,
    /// Error recovery points
    pub error_handlers: Vec<ErrorHandler>,
}

/// Error handler for async execution
#[derive(Debug, Clone)]
pub struct ErrorHandler {
    pub error_type: String,
    pub recovery_state: usize,
    pub cleanup_actions: Vec<CleanupAction>,
}

/// Cleanup actions to perform during error recovery
#[derive(Debug, Clone)]
pub enum CleanupAction {
    ReleaseResource(String),
    RollbackTransaction(String),
    LogError(String),
}

/// Converts a SystemDef into a state machine for async execution
pub struct SystemStateMachineBuilder {
    semantic_context: SemanticContext,
}

impl SystemStateMachineBuilder {
    pub fn new(semantic_context: SemanticContext) -> Self {
        Self { semantic_context }
    }

    /// Convert a SystemDef into an executable state machine
    pub fn build_state_machine(
        &self,
        system_def: &SystemDef,
        parameters: HashMap<String, ColumnValue>,
    ) -> Result<SystemStateMachine, AsyncExecutionError> {
        let mut states = Vec::new();
        let current_state = 0;

        // Extract resources that need to be acquired
        let resources_to_acquire = self.extract_required_resources(system_def);

        // Initialize state
        states.push(ExecutionState::Initialize {
            resources_to_acquire: resources_to_acquire.clone(),
        });

        // Convert body statements to states
        let statement_states = self.lower_statements_to_states(&system_def.body)?;
        states.extend(statement_states);

        // Complete state
        states.push(ExecutionState::Complete {
            return_value: None, // Will be set during execution
            resources_to_release: resources_to_acquire,
        });

        let execution_context = ExecutionContext {
            variables: HashMap::new(),
            scope_stack: Vec::new(),
            held_resources: HashMap::new(),
            accessed_tables: Vec::new(),
            started_at: Instant::now(),
            error_handlers: Vec::new(),
        };

        Ok(SystemStateMachine {
            system_name: system_def.name.clone(),
            states,
            current_state,
            execution_context,
            parameters,
        })
    }

    /// Extract resources required by the system
    fn extract_required_resources(&self, system_def: &SystemDef) -> Vec<String> {
        let mut resources = Vec::new();

        for param in &system_def.parameters {
            if let SystemParameter::Resource { name, .. } = param {
                resources.push(name.clone());
            }
        }

        resources
    }

    /// Convert statements into state machine states
    fn lower_statements_to_states(
        &self,
        statements: &[Statement],
    ) -> Result<Vec<ExecutionState>, AsyncExecutionError> {
        let mut states = Vec::new();
        let mut state_index = 1; // Start after initialize state

        // Group statements into execution blocks
        for statement in statements {
            match self.lower_statement_to_states(statement, &mut state_index)? {
                StateLoweringResult::States(mut new_states) => {
                    states.append(&mut new_states);
                }
                StateLoweringResult::YieldPoint(yield_state) => {
                    states.push(yield_state);
                    state_index += 1;
                }
            }
        }

        Ok(states)
    }

    /// Convert a single statement into state machine states
    fn lower_statement_to_states(
        &self,
        statement: &Statement,
        state_index: &mut usize,
    ) -> Result<StateLoweringResult, AsyncExecutionError> {
        match statement {
            Statement::Expression(expr) => {
                match expr {
                    Expression::Query(query_spec) => {
                        // Async query execution
                        let state = ExecutionState::ExecuteQuery {
                            table_name: query_spec.from_table.clone(),
                            query: self.convert_query_spec_to_execution(query_spec)?,
                            result_variable: format!("query_result_{}", state_index),
                            next_state: *state_index + 1,
                        };
                        *state_index += 1;
                        Ok(StateLoweringResult::YieldPoint(state))
                    }
                    Expression::Call { function, .. } => {
                        // Check if this is an async function call
                        if let Expression::Identifier(func_name) = function.as_ref() {
                            if self.is_async_function(func_name) {
                                // Create await state for async function
                                let state = ExecutionState::AwaitSystem {
                                    system_name: func_name.clone(),
                                    task_id: TaskId::new(), // Will be set during execution
                                    next_state: *state_index + 1,
                                };
                                *state_index += 1;
                                return Ok(StateLoweringResult::YieldPoint(state));
                            }
                        }

                        // Regular synchronous expression
                        Ok(StateLoweringResult::States(vec![
                            ExecutionState::ExecuteStatements {
                                statements: vec![statement.clone()],
                                current_statement: 0,
                            },
                        ]))
                    }
                    _ => {
                        // Regular expression execution
                        Ok(StateLoweringResult::States(vec![
                            ExecutionState::ExecuteStatements {
                                statements: vec![statement.clone()],
                                current_statement: 0,
                            },
                        ]))
                    }
                }
            }
            Statement::VariableDecl { .. } | Statement::Assignment { .. } => {
                // Synchronous operations
                Ok(StateLoweringResult::States(vec![
                    ExecutionState::ExecuteStatements {
                        statements: vec![statement.clone()],
                        current_statement: 0,
                    },
                ]))
            }
            Statement::ForLoop { iterable, body, .. } => {
                // For loops may contain async operations
                let mut loop_states = Vec::new();

                // Check if the iterable expression requires async evaluation
                if self.requires_async_evaluation(iterable) {
                    // Create state for async iterable evaluation
                    let eval_state = self.create_async_evaluation_state(iterable, *state_index)?;
                    loop_states.push(eval_state);
                    *state_index += 1;
                }

                // Convert loop body
                let body_states = self.lower_statements_to_states(body)?;
                loop_states.extend(body_states);

                Ok(StateLoweringResult::States(loop_states))
            }
            _ => {
                // Other statements are handled synchronously
                Ok(StateLoweringResult::States(vec![
                    ExecutionState::ExecuteStatements {
                        statements: vec![statement.clone()],
                        current_statement: 0,
                    },
                ]))
            }
        }
    }

    /// Convert query spec to execution details
    fn convert_query_spec_to_execution(
        &self,
        query_spec: &crate::ast::QuerySpec,
    ) -> Result<QueryExecution, AsyncExecutionError> {
        // For now, treat all queries as select operations
        let columns = query_spec
            .projections
            .iter()
            .map(|proj| proj.name.clone())
            .collect();

        Ok(QueryExecution::Select {
            columns,
            where_clause: query_spec
                .where_clause
                .as_ref()
                .map(|expr| (**expr).clone()),
        })
    }

    /// Check if a function is async
    fn is_async_function(&self, func_name: &str) -> bool {
        // Check semantic context for function signature
        if let Some(func_sig) = self.semantic_context.functions.get(func_name) {
            func_sig.is_async
        } else {
            false
        }
    }

    /// Check if an expression requires async evaluation
    fn requires_async_evaluation(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Query(_) => true,
            Expression::Call { function, .. } => {
                if let Expression::Identifier(func_name) = function.as_ref() {
                    self.is_async_function(func_name)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Create state for async expression evaluation
    fn create_async_evaluation_state(
        &self,
        expr: &Expression,
        state_index: usize,
    ) -> Result<ExecutionState, AsyncExecutionError> {
        match expr {
            Expression::Query(query_spec) => Ok(ExecutionState::ExecuteQuery {
                table_name: query_spec.from_table.clone(),
                query: self.convert_query_spec_to_execution(query_spec)?,
                result_variable: format!("async_eval_{}", state_index),
                next_state: state_index + 1,
            }),
            _ => Err(AsyncExecutionError::SystemError {
                system: "state_machine_builder".to_string(),
                message: format!("Unsupported async expression: {:?}", expr),
            }),
        }
    }
}

/// Result of lowering a statement to states
enum StateLoweringResult {
    States(Vec<ExecutionState>),
    YieldPoint(ExecutionState),
}

/// Executor for system state machines
#[derive(Debug)]
pub struct SystemStateMachineExecutor {
    tables: HashMap<String, TableRuntime>,
}

impl SystemStateMachineExecutor {
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
        }
    }

    /// Register a table for query execution
    pub fn register_table(&mut self, name: String, table: TableRuntime) {
        self.tables.insert(name, table);
    }

    /// Execute a single step of the state machine
    pub fn execute_step(
        &mut self,
        state_machine: &mut SystemStateMachine,
    ) -> Result<ExecutionStepResult, AsyncExecutionError> {
        if state_machine.current_state >= state_machine.states.len() {
            let result = SystemExecutionResult::Success {
                return_value: None,
                resources_modified: Vec::new(),
                tables_modified: state_machine.execution_context.accessed_tables.clone(),
            };
            return Ok(ExecutionStepResult::Completed(result));
        }

        let current_state = &state_machine.states[state_machine.current_state].clone();

        match current_state {
            ExecutionState::Initialize {
                resources_to_acquire,
            } => {
                // Resources should be acquired by the runtime before this
                for resource in resources_to_acquire {
                    state_machine
                        .execution_context
                        .held_resources
                        .insert(resource.clone(), ResourceAccess::Mutable); // Default to mutable
                }

                state_machine.current_state += 1;
                Ok(ExecutionStepResult::Continue)
            }

            ExecutionState::ExecuteStatements {
                statements,
                current_statement,
            } => {
                if *current_statement >= statements.len() {
                    state_machine.current_state += 1;
                    return Ok(ExecutionStepResult::Continue);
                }

                // Execute the current statement
                let statement = &statements[*current_statement];
                self.execute_statement(statement, &mut state_machine.execution_context)?;

                // Update state machine
                if let ExecutionState::ExecuteStatements {
                    current_statement, ..
                } = &mut state_machine.states[state_machine.current_state]
                {
                    *current_statement += 1;
                }

                Ok(ExecutionStepResult::Continue)
            }

            ExecutionState::AwaitResource {
                resource_name,
                access_type,
                next_state,
            } => {
                // Check if resource is available (would be done by runtime)
                let yield_point = YieldPoint::AwaitingResource {
                    resource_name: resource_name.clone(),
                    access_type: access_type.clone(),
                };

                state_machine.current_state = *next_state;
                Ok(ExecutionStepResult::Yield(yield_point))
            }

            ExecutionState::ExecuteQuery {
                table_name,
                query,
                result_variable,
                next_state,
            } => {
                // Execute query against table
                let result = self.execute_query(table_name, query)?;

                // Store result in execution context
                state_machine
                    .execution_context
                    .variables
                    .insert(result_variable.clone(), result);

                state_machine.current_state = *next_state;
                Ok(ExecutionStepResult::Continue)
            }

            ExecutionState::AwaitSystem {
                system_name,
                task_id,
                next_state,
            } => {
                let yield_point = YieldPoint::AwaitingSystemCompletion {
                    system_name: system_name.clone(),
                    task_id: *task_id,
                };

                state_machine.current_state = *next_state;
                Ok(ExecutionStepResult::Yield(yield_point))
            }

            ExecutionState::Sleep {
                duration,
                next_state,
            } => {
                let yield_point = YieldPoint::Sleeping {
                    duration: *duration,
                    started_at: Instant::now(),
                };

                state_machine.current_state = *next_state;
                Ok(ExecutionStepResult::Yield(yield_point))
            }

            ExecutionState::HandleError {
                error,
                recovery_state,
            } => {
                // Handle error recovery
                if let Some(recovery) = recovery_state {
                    state_machine.current_state = *recovery;
                    Ok(ExecutionStepResult::Continue)
                } else {
                    Err(error.clone())
                }
            }

            ExecutionState::Complete {
                return_value,
                resources_to_release,
            } => {
                // Final cleanup
                for resource in resources_to_release {
                    state_machine
                        .execution_context
                        .held_resources
                        .remove(resource);
                }

                let result = SystemExecutionResult::Success {
                    return_value: return_value.clone(),
                    resources_modified: Vec::new(),
                    tables_modified: state_machine.execution_context.accessed_tables.clone(),
                };

                state_machine.current_state = state_machine.states.len();
                Ok(ExecutionStepResult::Completed(result))
            }
        }
    }

    /// Execute a synchronous statement
    fn execute_statement(
        &mut self,
        statement: &Statement,
        context: &mut ExecutionContext,
    ) -> Result<(), AsyncExecutionError> {
        match statement {
            Statement::VariableDecl {
                pattern,
                value,
                type_annotation: _,
            } => {
                let val = self.evaluate_expression(value, context)?;
                self.bind_pattern_value(pattern, val, context)
            }
            Statement::Assignment { target, value, .. } => {
                if let Expression::Identifier(var_name) = target {
                    let val = self.evaluate_expression(value, context)?;
                    context.variables.insert(var_name.clone(), val);
                }
                Ok(())
            }
            _ => {
                // Other statements not implemented yet
                Ok(())
            }
        }
    }

    /// Execute a query against a table
    fn execute_query(
        &mut self,
        table_name: &str,
        query: &QueryExecution,
    ) -> Result<ColumnValue, AsyncExecutionError> {
        let table =
            self.tables
                .get_mut(table_name)
                .ok_or_else(|| AsyncExecutionError::SystemError {
                    system: "executor".to_string(),
                    message: format!("Table not found: {}", table_name),
                })?;

        match query {
            QueryExecution::Select {
                columns,
                where_clause,
            } => {
                // Simplified query execution - return count for now
                let _ = (columns, where_clause);
                let all_rows = table.scan_all();
                Ok(ColumnValue::U64(all_rows.len() as u64))
            }
            QueryExecution::Insert { values } => {
                use crate::table_runtime::TableRow;
                let row = TableRow {
                    values: values.clone(),
                };
                let row_id = table.insert_row(row)?;
                Ok(ColumnValue::U64(row_id.0 as u64))
            }
            QueryExecution::Update { row_id, updates } => {
                table.update_row(*row_id, updates.clone())?;
                Ok(ColumnValue::Bool(true))
            }
            QueryExecution::Delete { row_id } => {
                table.delete_row(*row_id)?;
                Ok(ColumnValue::Bool(true))
            }
        }
    }

    /// Evaluate an expression to a value
    fn evaluate_expression(
        &self,
        expr: &Expression,
        context: &ExecutionContext,
    ) -> Result<ColumnValue, AsyncExecutionError> {
        match expr {
            Expression::Literal(lit) => match lit {
                crate::ast::Literal::Integer(int_lit) => Ok(ColumnValue::U64(int_lit.value as u64)),
                crate::ast::Literal::String(s) => Ok(ColumnValue::String(s.clone())),
                crate::ast::Literal::Boolean(b) => Ok(ColumnValue::Bool(*b)),
                _ => Ok(ColumnValue::String("unknown".to_string())),
            },
            Expression::Identifier(name) => context.variables.get(name).cloned().ok_or_else(|| {
                AsyncExecutionError::SystemError {
                    system: "executor".to_string(),
                    message: format!("Variable not found: {}", name),
                }
            }),
            _ => {
                // Other expressions not implemented yet
                Ok(ColumnValue::String("placeholder".to_string()))
            }
        }
    }
}

impl SystemStateMachineExecutor {
    fn bind_pattern_value(
        &mut self,
        pattern: &BindingPattern,
        value: ColumnValue,
        context: &mut ExecutionContext,
    ) -> Result<(), AsyncExecutionError> {
        match pattern {
            BindingPattern::Identifier(name) => {
                context.variables.insert(name.clone(), value);
                Ok(())
            }
            BindingPattern::Discard => Ok(()),
            BindingPattern::Tuple(_) => Err(AsyncExecutionError::SystemError {
                system: "executor".to_string(),
                message: "tuple destructuring is not supported in system executor".to_string(),
            }),
        }
    }
}

/// Result of executing a state machine step
#[derive(Debug)]
pub enum ExecutionStepResult {
    /// Continue to next step
    Continue,
    /// Yield control at this point
    Yield(YieldPoint),
    /// Execution completed
    Completed(SystemExecutionResult),
}

impl Default for SystemStateMachineExecutor {
    fn default() -> Self {
        Self::new()
    }
}
