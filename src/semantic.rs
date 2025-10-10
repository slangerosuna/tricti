use crate::ast::*;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct SemanticContext {
    // Global (module-level) variables/constants
    pub variables: HashMap<String, Type>,
    // Lexical scopes (innermost at the end). Only used during analysis
    // to model block/loop scopes. Variables declared while a scope is
    // active live here and are removed on exit.
    pub var_scopes: Vec<HashMap<String, Type>>,
    pub functions: HashMap<String, FunctionSignature>,
    pub types: HashMap<String, Type>,
    pub function_generics: HashMap<String, Vec<String>>,
    pub type_generics: HashMap<String, Vec<String>>,
    pub current_function_return_type: Option<Type>,
    pub loop_depth: usize,
    // Traits and impls
    pub traits: HashMap<String, TraitInfo>,
    pub trait_impls: HashMap<String, HashMap<String, ImplInfo>>, // trait -> (type -> impl)
    pub inherent_impls: HashMap<String, ImplInfo>,               // type -> impl
    // Table schemas
    pub tables: HashMap<String, TableDef>,

    // Module system - Rust-like namespacing
    pub modules: HashMap<String, ModuleInfo>,
    pub current_module_path: Vec<String>,
    pub use_imports: HashMap<String, String>, // alias -> full_path
    pub glob_imports: Vec<String>,            // modules imported with *
}

#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub parameters: Vec<Type>,
    pub return_type: Type,
    pub is_async: bool,
}

#[derive(Debug, Clone)]
pub struct TraitInfo {
    pub associated_types: Vec<String>,
    pub methods: HashMap<String, Type>, // function types
}

#[derive(Debug, Clone, Default)]
pub struct ImplInfo {
    pub associated_types: HashMap<String, Type>,
    pub methods: HashMap<String, FunctionSignature>,
}

#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub name: String,
    pub path: Vec<String>, // Full module path (e.g., ["std", "collections", "vec"])
    pub is_public: bool,
    pub variables: HashMap<String, Type>,
    pub functions: HashMap<String, FunctionSignature>,
    pub types: HashMap<String, Type>,
    pub traits: HashMap<String, TraitInfo>,
    pub submodules: HashMap<String, String>, // name -> full_path
    pub exports: HashMap<String, String>,    // name -> full_path for pub use
}

#[derive(Debug)]
pub enum SemanticError {
    UndefinedVariable(String),
    UndefinedFunction(String),
    TypeMismatch {
        expected: Type,
        found: Type,
    },
    InvalidOperation {
        operator: String,
        operand_types: Vec<Type>,
    },
    ReturnOutsideFunction,
    BreakOutsideLoop,
    ContinueOutsideLoop,
    ArgumentCountMismatch {
        expected: usize,
        found: usize,
    },
    AmbiguousMethod {
        type_name: String,
        method: String,
        traits: Vec<String>,
    },
    InvalidRangeStepZero,
}

impl SemanticContext {
    pub fn new() -> Self {
        let mut context = SemanticContext {
            variables: HashMap::new(),
            var_scopes: Vec::new(),
            functions: HashMap::new(),
            types: HashMap::new(),
            function_generics: HashMap::new(),
            type_generics: HashMap::new(),
            current_function_return_type: None,
            loop_depth: 0,
            traits: HashMap::new(),
            trait_impls: HashMap::new(),
            inherent_impls: HashMap::new(),
            tables: HashMap::new(),
            modules: HashMap::new(),
            current_module_path: Vec::new(),
            use_imports: HashMap::new(),
            glob_imports: Vec::new(),
        };

        // Add built-in functions
        context.functions.insert(
            "println".to_string(),
            FunctionSignature {
                parameters: vec![], // Variadic, we'll handle this specially
                return_type: Type::None,
                is_async: false,
            },
        );
        // len: string length in bytes
        context.functions.insert(
            "len".to_string(),
            FunctionSignature {
                parameters: vec![Type::Identifier {
                    name: "string".to_string(),
                    type_args: vec![],
                }],
                return_type: Type::Identifier {
                    name: "i64".to_string(),
                    type_args: vec![],
                },
                is_async: false,
            },
        );
        // trim: remove leading and trailing ASCII whitespace
        context.functions.insert(
            "trim".to_string(),
            FunctionSignature {
                parameters: vec![Type::Identifier {
                    name: "string".to_string(),
                    type_args: vec![],
                }],
                return_type: Type::Identifier {
                    name: "string".to_string(),
                    type_args: vec![],
                },
                is_async: false,
            },
        );
        // streq: string equality (byte-wise)
        context.functions.insert(
            "streq".to_string(),
            FunctionSignature {
                parameters: vec![
                    Type::Identifier {
                        name: "string".to_string(),
                        type_args: vec![],
                    },
                    Type::Identifier {
                        name: "string".to_string(),
                        type_args: vec![],
                    },
                ],
                return_type: Type::Identifier {
                    name: "bool".to_string(),
                    type_args: vec![],
                },
                is_async: false,
            },
        );
        // contains: substring check (byte-wise)
        context.functions.insert(
            "contains".to_string(),
            FunctionSignature {
                parameters: vec![
                    Type::Identifier {
                        name: "string".to_string(),
                        type_args: vec![],
                    },
                    Type::Identifier {
                        name: "string".to_string(),
                        type_args: vec![],
                    },
                ],
                return_type: Type::Identifier {
                    name: "bool".to_string(),
                    type_args: vec![],
                },
                is_async: false,
            },
        );
        // starts_with / ends_with: prefix/suffix checks (byte-wise)
        for name in ["starts_with", "ends_with"] {
            context.functions.insert(
                name.to_string(),
                FunctionSignature {
                    parameters: vec![
                        Type::Identifier {
                            name: "string".to_string(),
                            type_args: vec![],
                        },
                        Type::Identifier {
                            name: "string".to_string(),
                            type_args: vec![],
                        },
                    ],
                    return_type: Type::Identifier {
                        name: "bool".to_string(),
                        type_args: vec![],
                    },
                    is_async: false,
                },
            );
        }
        // find: first index of needle in haystack, or -1 if missing (byte index)
        context.functions.insert(
            "find".to_string(),
            FunctionSignature {
                parameters: vec![
                    Type::Identifier {
                        name: "string".to_string(),
                        type_args: vec![],
                    },
                    Type::Identifier {
                        name: "string".to_string(),
                        type_args: vec![],
                    },
                ],
                return_type: Type::Identifier {
                    name: "i64".to_string(),
                    type_args: vec![],
                },
                is_async: false,
            },
        );
        // slice helpers (prototype): slice_len(s: slice_i64) -> i64; slice_is_empty(s: slice_i64) -> bool
        context.functions.insert(
            "slice_len".to_string(),
            FunctionSignature {
                parameters: vec![Type::Identifier {
                    name: "slice_i64".to_string(),
                    type_args: vec![],
                }],
                return_type: Type::Identifier {
                    name: "i64".to_string(),
                    type_args: vec![],
                },
                is_async: false,
            },
        );
        context.functions.insert(
            "slice_is_empty".to_string(),
            FunctionSignature {
                parameters: vec![Type::Identifier {
                    name: "slice_i64".to_string(),
                    type_args: vec![],
                }],
                return_type: Type::Identifier {
                    name: "bool".to_string(),
                    type_args: vec![],
                },
                is_async: false,
            },
        );
        // slice_get(s: slice_i64, idx: i64) -> i64
        context.functions.insert(
            "slice_get".to_string(),
            FunctionSignature {
                parameters: vec![
                    Type::Identifier {
                        name: "slice_i64".to_string(),
                        type_args: vec![],
                    },
                    Type::Identifier {
                        name: "i64".to_string(),
                        type_args: vec![],
                    },
                ],
                return_type: Type::Optional {
                    inner: Box::new(Type::Identifier {
                        name: "i64".to_string(),
                        type_args: vec![],
                    }),
                },
                is_async: false,
            },
        );

        // bool slice helpers
        context.functions.insert(
            "slice_len_bool".to_string(),
            FunctionSignature {
                parameters: vec![Type::Identifier {
                    name: "slice_bool".to_string(),
                    type_args: vec![],
                }],
                return_type: Type::Identifier {
                    name: "i64".to_string(),
                    type_args: vec![],
                },
                is_async: false,
            },
        );
        context.functions.insert(
            "slice_is_empty_bool".to_string(),
            FunctionSignature {
                parameters: vec![Type::Identifier {
                    name: "slice_bool".to_string(),
                    type_args: vec![],
                }],
                return_type: Type::Identifier {
                    name: "bool".to_string(),
                    type_args: vec![],
                },
                is_async: false,
            },
        );
        context.functions.insert(
            "slice_get_bool".to_string(),
            FunctionSignature {
                parameters: vec![
                    Type::Identifier {
                        name: "slice_bool".to_string(),
                        type_args: vec![],
                    },
                    Type::Identifier {
                        name: "i64".to_string(),
                        type_args: vec![],
                    },
                ],
                return_type: Type::Optional {
                    inner: Box::new(Type::Identifier {
                        name: "bool".to_string(),
                        type_args: vec![],
                    }),
                },
                is_async: false,
            },
        );

        // Process exit (libc)
        context.functions.insert(
            "exit".to_string(),
            FunctionSignature {
                parameters: vec![Type::Identifier {
                    name: "i32".to_string(),
                    type_args: vec![],
                }],
                return_type: Type::None,
                is_async: false,
            },
        );
        context.types.insert(
            "i32".to_string(),
            Type::Identifier {
                name: "i32".to_string(),
                type_args: vec![],
            },
        );
        context.types.insert(
            "i64".to_string(),
            Type::Identifier {
                name: "i64".to_string(),
                type_args: vec![],
            },
        );
        context.types.insert(
            "f32".to_string(),
            Type::Identifier {
                name: "f32".to_string(),
                type_args: vec![],
            },
        );
        context.types.insert(
            "f64".to_string(),
            Type::Identifier {
                name: "f64".to_string(),
                type_args: vec![],
            },
        );
        context.types.insert(
            "bool".to_string(),
            Type::Identifier {
                name: "bool".to_string(),
                type_args: vec![],
            },
        );
        context.types.insert(
            "string".to_string(),
            Type::Identifier {
                name: "string".to_string(),
                type_args: vec![],
            },
        );
        context.types.insert(
            "char".to_string(),
            Type::Identifier {
                name: "char".to_string(),
                type_args: vec![],
            },
        );
        // Minimal built-in slice for i64: { ptr: &i64, len: i64 }
        context.types.insert(
            "slice_i64".to_string(),
            Type::Struct {
                fields: {
                    let mut m = HashMap::new();
                    m.insert(
                        "ptr".to_string(),
                        Type::Pointer {
                            is_mutable: false,
                            pointee: Box::new(Type::Identifier {
                                name: "i64".to_string(),
                                type_args: vec![],
                            }),
                        },
                    );
                    m.insert(
                        "len".to_string(),
                        Type::Identifier {
                            name: "i64".to_string(),
                            type_args: vec![],
                        },
                    );
                    m
                },
            },
        );

        // Minimal built-in slice for bool: { ptr: &bool, len: i64 }
        context.types.insert(
            "slice_bool".to_string(),
            Type::Struct {
                fields: {
                    let mut m = HashMap::new();
                    m.insert(
                        "ptr".to_string(),
                        Type::Pointer {
                            is_mutable: false,
                            pointee: Box::new(Type::Identifier {
                                name: "bool".to_string(),
                                type_args: vec![],
                            }),
                        },
                    );
                    m.insert(
                        "len".to_string(),
                        Type::Identifier {
                            name: "i64".to_string(),
                            type_args: vec![],
                        },
                    );
                    m
                },
            },
        );

        context
    }

    pub fn enter_scope(&mut self) {
        // Push a new lexical scope for variables declared within blocks/loops
        self.var_scopes.push(HashMap::new());
    }

    pub fn exit_scope(&mut self) {
        // Pop the most recent lexical scope. If none exists, it's a no-op.
        let _ = self.var_scopes.pop();
    }

    pub fn define_variable(&mut self, name: String, var_type: Type) {
        if let Some(scope) = self.var_scopes.last_mut() {
            scope.insert(name, var_type);
        } else {
            self.variables.insert(name, var_type);
        }
    }

    /// Enter a module namespace
    pub fn enter_module(&mut self, module_name: String) {
        self.current_module_path.push(module_name.clone());
        let full_path = self.current_module_path.join("::");

        if !self.modules.contains_key(&full_path) {
            self.modules.insert(
                full_path.clone(),
                ModuleInfo {
                    name: module_name,
                    path: self.current_module_path.clone(),
                    is_public: true, // Default to public, can be changed later
                    variables: HashMap::new(),
                    functions: HashMap::new(),
                    types: HashMap::new(),
                    traits: HashMap::new(),
                    submodules: HashMap::new(),
                    exports: HashMap::new(),
                },
            );
        }
    }

    /// Exit current module namespace
    pub fn exit_module(&mut self) {
        self.current_module_path.pop();
    }

    /// Get the current module's full path
    pub fn current_module_full_path(&self) -> String {
        self.current_module_path.join("::")
    }

    /// Register a use import
    pub fn add_use_import(&mut self, path: Vec<String>, alias: Option<String>) {
        let full_path = path.join("::");
        let import_name = alias.unwrap_or_else(|| path.last().unwrap_or(&"".to_string()).clone());

        if full_path.ends_with("*") {
            // Glob import
            let module_path = full_path.trim_end_matches("::*");
            self.glob_imports.push(module_path.to_string());
        } else {
            // Named import
            self.use_imports.insert(import_name, full_path);
        }
    }

    /// Resolve a name through module system
    pub fn resolve_name(&self, name: &str) -> Option<String> {
        // First check direct imports
        if let Some(full_path) = self.use_imports.get(name) {
            return Some(full_path.clone());
        }

        // Then check glob imports
        for glob_module in &self.glob_imports {
            let potential_path = format!("{}::{}", glob_module, name);
            if self.modules.contains_key(&potential_path) {
                return Some(potential_path);
            }
        }

        // Finally check current module
        let current_module = self.current_module_full_path();
        if !current_module.is_empty() {
            let local_path = format!("{}::{}", current_module, name);
            if self.modules.contains_key(&local_path) {
                return Some(local_path);
            }
        }

        // Return as-is if not found (for built-ins or local names)
        Some(name.to_string())
    }

    pub fn resolve_type(&self, ty: &Type) -> Type {
        match ty {
            Type::Identifier { name, type_args } if !type_args.is_empty() => {
                if let Some(params) = self.type_generics.get(name) {
                    if let Some(base_ty) = self.types.get(name) {
                        self.substitute_type(base_ty, params, type_args)
                    } else {
                        ty.clone()
                    }
                } else {
                    ty.clone()
                }
            }
            _ => ty.clone(),
        }
    }

    fn substitute_type(&self, ty: &Type, params: &[String], args: &[Type]) -> Type {
        match ty {
            Type::Identifier { name, type_args } => {
                if type_args.is_empty() {
                    if let Some(pos) = params.iter().position(|p| p == name) {
                        args[pos].clone()
                    } else {
                        ty.clone()
                    }
                } else {
                    Type::Identifier {
                        name: name.clone(),
                        type_args: type_args
                            .iter()
                            .map(|t| self.substitute_type(t, params, args))
                            .collect(),
                    }
                }
            }
            Type::Pointer {
                is_mutable,
                pointee,
            } => Type::Pointer {
                is_mutable: *is_mutable,
                pointee: Box::new(self.substitute_type(pointee, params, args)),
            },
            Type::RawPointer { pointee, is_raw } => Type::RawPointer {
                pointee: Box::new(self.substitute_type(pointee, params, args)),
                is_raw: *is_raw,
            },
            Type::Optional { inner } => Type::Optional {
                inner: Box::new(self.substitute_type(inner, params, args)),
            },
            Type::Result { inner } => Type::Result {
                inner: Box::new(self.substitute_type(inner, params, args)),
            },
            Type::Tuple(elements) => Type::Tuple(
                elements
                    .iter()
                    .map(|e| self.substitute_type(e, params, args))
                    .collect(),
            ),
            Type::Matrix {
                element_type,
                dimensions,
            } => Type::Matrix {
                element_type: Box::new(self.substitute_type(element_type, params, args)),
                dimensions: dimensions.clone(),
            },
            Type::Function {
                parameters,
                return_type,
            } => Type::Function {
                parameters: parameters
                    .iter()
                    .map(|p| self.substitute_type(p, params, args))
                    .collect(),
                return_type: Box::new(self.substitute_type(return_type, params, args)),
            },
            Type::Struct { fields } => Type::Struct {
                fields: fields
                    .iter()
                    .map(|(k, v)| (k.clone(), self.substitute_type(v, params, args)))
                    .collect(),
            },
            Type::Enum { variants, order } => Type::Enum {
                variants: variants
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.as_ref().map(|t| self.substitute_type(t, params, args)),
                        )
                    })
                    .collect(),
                order: order.clone(),
            },
            Type::Trait { .. } => ty.clone(),
            Type::Reference { is_mutable, inner } => Type::Reference {
                is_mutable: *is_mutable,
                inner: Box::new(self.substitute_type(inner, params, args)),
            },
            Type::None => ty.clone(),
        }
    }

    pub fn get_variable_type(&self, name: &str) -> Option<&Type> {
        // Look from innermost scope outward, then fall back to globals
        for scope in self.var_scopes.iter().rev() {
            if let Some(t) = scope.get(name) {
                return Some(t);
            }
        }

        // Try module-resolved name
        if let Some(resolved_name) = self.resolve_name(name) {
            if let Some(t) = self.variables.get(&resolved_name) {
                return Some(t);
            }

            // Check in modules
            for (module_path, module_info) in &self.modules {
                if resolved_name.starts_with(module_path) {
                    let local_name = resolved_name
                        .trim_start_matches(module_path)
                        .trim_start_matches("::");
                    if let Some(t) = module_info.variables.get(local_name) {
                        return Some(t);
                    }
                }
            }
        }

        self.variables.get(name)
    }

    pub fn define_function(&mut self, name: String, signature: FunctionSignature) {
        let current_module = self.current_module_full_path();
        if current_module.is_empty() {
            self.functions.insert(name, signature);
        } else {
            // Store in current module
            if let Some(module_info) = self.modules.get_mut(&current_module) {
                module_info
                    .functions
                    .insert(name.clone(), signature.clone());
            }
            // Also store globally with module prefix for compatibility
            let full_name = format!("{}::{}", current_module, name);
            self.functions.insert(full_name, signature);
        }
    }

    pub fn get_function_signature(&self, name: &str) -> Option<&FunctionSignature> {
        // First try direct lookup
        if let Some(sig) = self.functions.get(name) {
            return Some(sig);
        }

        // Try module-resolved name
        if let Some(resolved_name) = self.resolve_name(name) {
            if let Some(sig) = self.functions.get(&resolved_name) {
                return Some(sig);
            }

            // Check in modules
            for (module_path, module_info) in &self.modules {
                if resolved_name.starts_with(module_path) {
                    let local_name = resolved_name
                        .trim_start_matches(module_path)
                        .trim_start_matches("::");
                    if let Some(sig) = module_info.functions.get(local_name) {
                        return Some(sig);
                    }
                }
            }
        }

        None
    }
}

pub fn analyze_program(program: &Program) -> Result<SemanticContext, SemanticError> {
    let mut context = SemanticContext::new();

    // First pass: collect function signatures and type definitions
    for statement in &program.statements {
        collect_definitions(statement, &mut context)?;
    }

    // Second pass: type check all statements
    for statement in &program.statements {
        match analyze_statement(statement, &mut context) {
            Ok(()) => {}
            Err(SemanticError::TypeMismatch { .. }) => {
                // Temporarily treat type mismatches as soft warnings while
                // the type system and generics support are under construction.
            }
            Err(err) => return Err(err),
        }
    }

    Ok(context)
}

fn collect_definitions(
    statement: &Statement,
    context: &mut SemanticContext,
) -> Result<(), SemanticError> {
    match statement {
        Statement::ModuleDecl {
            is_public: _,
            name,
            items,
        } => {
            // Enter module namespace for definition collection
            context.enter_module(name.clone());

            if let Some(stmts) = items {
                for s in stmts {
                    collect_definitions(s, context)?;
                }
            }

            // Exit module namespace
            context.exit_module();
        }
        Statement::ConstDecl {
            name,
            type_params,
            value,
            ..
        } => {
            match value {
                ConstValue::Type(type_def) => {
                    context.types.insert(name.clone(), type_def.clone());
                    context.type_generics.insert(
                        name.clone(),
                        type_params.iter().map(|tp| tp.name.clone()).collect(),
                    );
                    // If this is a trait type definition, register trait info too
                    if let Type::Trait {
                        associated_types,
                        methods,
                    } = type_def
                    {
                        let ti = TraitInfo {
                            associated_types: associated_types.clone(),
                            methods: methods.clone(),
                        };
                        context.traits.insert(name.clone(), ti);
                    }
                }
                ConstValue::Expression(expr) => {
                    // If expression is a function, pre-declare its signature in the function table and as a variable value of function type
                    if let Expression::Function {
                        parameters,
                        return_type,
                        type_params: fn_generics,
                        ..
                    } = expr
                    {
                        let sig = FunctionSignature {
                            parameters: parameters
                                .iter()
                                .map(|p| {
                                    p.param_type.clone().unwrap_or(Type::Identifier {
                                        name: "i64".to_string(),
                                        type_args: vec![],
                                    })
                                })
                                .collect(),
                            return_type: return_type.clone().unwrap_or(Type::Identifier {
                                name: "i64".to_string(),
                                type_args: vec![],
                            }),
                            is_async: false,
                        };
                        context.define_function(name.clone(), sig.clone());
                        let generics: Vec<String> = if !fn_generics.is_empty() {
                            fn_generics.iter().map(|tp| tp.name.clone()).collect()
                        } else {
                            type_params.iter().map(|tp| tp.name.clone()).collect()
                        };
                        if !generics.is_empty() {
                            context.function_generics.insert(name.clone(), generics);
                        }
                        context.define_variable(
                            name.clone(),
                            Type::Function {
                                parameters: sig.parameters.clone(),
                                return_type: Box::new(sig.return_type.clone()),
                            },
                        );
                    } else {
                        // For other expression constants, infer the type during analysis
                        let expr_type = infer_expression_type(expr, context)?;
                        context.define_variable(name.clone(), expr_type);
                    }
                }
                ConstValue::TableDef(table_def) => {
                    // Clone the table definition so we can infer computed column types
                    let mut table_def_copy = table_def.clone();

                    // Infer and set proper types for computed columns
                    infer_computed_column_types(&mut table_def_copy, context)?;

                    // Validate the table definition (now with properly typed computed columns)
                    validate_table_definition(&table_def_copy, context)?;

                    // Insert the updated table definition into context
                    context.tables.insert(name.clone(), table_def_copy);
                }
                ConstValue::SystemDef(system_def) => {
                    // Register system function signature during collection phase
                    let sig = FunctionSignature {
                        parameters: system_def
                            .parameters
                            .iter()
                            .map(|p| match p {
                                SystemParameter::Query {
                                    name: _,
                                    query_spec: _,
                                } => Type::Identifier {
                                    name: "QueryResult".to_string(),
                                    type_args: vec![],
                                },
                                SystemParameter::Resource {
                                    resource_type,
                                    access,
                                    ..
                                } => match access {
                                    ResourceAccess::Immutable => Type::Reference {
                                        is_mutable: false,
                                        inner: Box::new(resource_type.clone()),
                                    },
                                    ResourceAccess::Mutable => Type::Reference {
                                        is_mutable: true,
                                        inner: Box::new(resource_type.clone()),
                                    },
                                    ResourceAccess::Owned => resource_type.clone(),
                                },
                                SystemParameter::Regular { value_type, .. } => value_type.clone(),
                            })
                            .collect(),
                        return_type: system_def.return_type.clone().unwrap_or(Type::None),
                        is_async: system_def.is_async,
                    };
                    context.define_function(name.clone(), sig);
                }
                ConstValue::ComposeDef(_) | ConstValue::DatabaseDef(_) => {
                    // TODO: Add proper handling for compose and database definitions during collection
                }
            }
        }
        Statement::ImplBlock {
            type_params: _,
            trait_name,
            type_name,
            self_type,
            methods,
        } => {
            let is_trait_impl = trait_name.is_some();
            // Prepare impl info bucket
            let impl_info = ImplInfo::default();
            if is_trait_impl {
                context
                    .trait_impls
                    .entry(trait_name.clone().unwrap())
                    .or_default()
                    .insert(type_name.clone(), impl_info);
            } else {
                context.inherent_impls.insert(type_name.clone(), impl_info);
            }

            // Scan methods: register mangled function signatures so later expression calls can resolve
            for m in methods {
                match m {
                    Statement::ConstDecl {
                        name: mname, value, ..
                    } => match value {
                        ConstValue::Expression(Expression::Function {
                            parameters,
                            return_type,
                            ..
                        }) => {
                            // Prepend implicit receiver parameter of type &type_name for inherent methods
                            // Skip for static methods (those with no parameters)
                            let mut params: Vec<Type> = Vec::new();
                            if trait_name.is_none() && !parameters.is_empty() {
                                params.push(Type::Pointer {
                                    is_mutable: false,
                                    pointee: Box::new(self_type.clone()),
                                });
                            }
                            params.extend(parameters.iter().map(|p| {
                                p.param_type.clone().unwrap_or(Type::Identifier {
                                    name: "i64".to_string(),
                                    type_args: vec![],
                                })
                            }));
                            let sig = FunctionSignature {
                                parameters: params,
                                return_type: return_type.clone().unwrap_or(Type::Identifier {
                                    name: "i64".to_string(),
                                    type_args: vec![],
                                }),
                                is_async: false,
                            };
                            let mangled =
                                mangle_method_name(trait_name.as_deref(), type_name, mname);
                            context.define_function(mangled, sig);
                        }
                        ConstValue::Type(_) => {
                            // associated type binding; collected during analysis phase below
                        }
                        ConstValue::Expression(_) => { /* ignore non-function expressions in impl header pass */
                        }
                        ConstValue::TableDef(table_def) => {
                            validate_table_definition(table_def, context)?;
                        }
                        ConstValue::SystemDef(system_def) => {
                            // Validate system function definition in impl block context
                            validate_system_definition(system_def, context)?;
                        }
                        ConstValue::ComposeDef(_) | ConstValue::DatabaseDef(_) => {
                            // TODO: Add validation for compose and database definitions
                        }
                    },
                    Statement::ImplMethod {
                        name: mname,
                        parameters,
                        return_type,
                        ..
                    } => {
                        let sig = FunctionSignature {
                            parameters: parameters
                                .iter()
                                .map(|p| {
                                    p.param_type.clone().unwrap_or(Type::Identifier {
                                        name: "i64".to_string(),
                                        type_args: vec![],
                                    })
                                })
                                .collect(),
                            return_type: return_type.clone().unwrap_or(Type::Identifier {
                                name: "i64".to_string(),
                                type_args: vec![],
                            }),
                            is_async: false,
                        };
                        let mangled = mangle_method_name(trait_name.as_deref(), type_name, mname);
                        context.define_function(mangled, sig);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn analyze_statement(
    statement: &Statement,
    context: &mut SemanticContext,
) -> Result<(), SemanticError> {
    match statement {
        Statement::ModuleDecl {
            is_public: _,
            name,
            items,
        } => {
            // Enter module namespace
            context.enter_module(name.clone());

            if let Some(stmts) = items {
                for s in stmts {
                    analyze_statement(s, context)?;
                }
            }

            // Exit module namespace
            context.exit_module();
        }
        Statement::ConstDecl {
            name,
            type_params,
            type_annotation,
            value,
            ..
        } => {
            if !type_params.is_empty() {
                context.type_generics.insert(
                    name.clone(),
                    type_params.iter().map(|tp| tp.name.clone()).collect(),
                );
            }
            if let ConstValue::Type(Type::Function {
                parameters,
                return_type,
            }) = &value
            {
                let param_types = parameters.clone();
                let ret_type = *return_type.clone();
                context.functions.insert(
                    name.clone(),
                    FunctionSignature {
                        parameters: param_types,
                        return_type: ret_type,
                        is_async: false,
                    },
                );
            }
            match value {
                ConstValue::Expression(Expression::Function {
                    parameters,
                    return_type,
                    body,
                    ..
                }) => {
                    // Analyze function body with parameter bindings and expected return type
                    let prev_vars = context.variables.clone();
                    let prev_ret = context.current_function_return_type.clone();
                    // Bind parameters
                    for p in parameters {
                        let ty = p.param_type.clone().unwrap_or(Type::Identifier {
                            name: "i64".into(),
                            type_args: vec![],
                        });
                        context.define_variable(p.name.clone(), ty);
                    }
                    // Set expected return
                    context.current_function_return_type =
                        Some(return_type.clone().unwrap_or(Type::None));
                    match body {
                        FunctionBody::Block(stmts) => {
                            for s in stmts {
                                analyze_statement(s, context)?;
                            }
                        }
                        FunctionBody::Expression(expr) => {
                            // If body is a block expression, descend into its statements to analyze side-effects and calls
                            if let Expression::Block { statements } = expr.as_ref() {
                                for s in statements {
                                    analyze_statement(s, context)?;
                                }
                            } else {
                                let _ = infer_expression_type(expr, context)?;
                            }
                        }
                    }
                    // Restore
                    context.variables = prev_vars;
                    context.current_function_return_type = prev_ret;
                }
                ConstValue::Expression(other_expr) => {
                    // Non-function const expression: define it as a (module-level) variable with its type
                    let inferred = infer_expression_type(other_expr, context)?;
                    let final_type = if let Some(ann) = type_annotation {
                        ann.clone()
                    } else {
                        inferred
                    };
                    context.define_variable(name.clone(), final_type);
                }
                ConstValue::Type(_) => {
                    // Type aliases/constants are handled during collection; nothing to analyze here
                }
                ConstValue::TableDef(table_def) => {
                    validate_table_definition(table_def, context)?;
                }
                ConstValue::SystemDef(system_def) => {
                    // Perform full semantic analysis of system function
                    analyze_system_definition(system_def, context)?;
                }
                ConstValue::ComposeDef(_) | ConstValue::DatabaseDef(_) => {
                    // TODO: Add semantic analysis for compose and database definitions
                }
            }
        }
        Statement::VariableDecl {
            pattern,
            type_annotation,
            value,
        } => {
            let value_type = infer_expression_type(value, context)?;

            let final_type = if let Some(annotation) = type_annotation {
                // Allow assigning i64 to enum-typed variables (repr i64)
                let enum_i64_ok = match (annotation, &value_type) {
                    (
                        Type::Identifier {
                            name: tn,
                            type_args: _,
                        },
                        Type::Identifier {
                            name: vn,
                            type_args: _,
                        },
                    ) if vn == "i64" => {
                        if let Some(Type::Enum { .. }) = context.types.get(tn) {
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                if !enum_i64_ok {
                    if !types_compatible(annotation, &value_type) {
                        return Err(SemanticError::TypeMismatch {
                            expected: annotation.clone(),
                            found: value_type,
                        });
                    }
                }
                annotation.clone()
            } else {
                value_type
            };

            bind_binding_pattern(pattern, &final_type, context)?;
        }

        Statement::Assignment { target, value, .. } => {
            let target_type = infer_expression_type(target, context)?;
            let value_type = infer_expression_type(value, context)?;

            // Allow assigning i64 to enum-typed variables (repr i64)
            let enum_i64_ok = match (&target_type, &value_type) {
                (
                    Type::Identifier {
                        name: tn,
                        type_args: _,
                    },
                    Type::Identifier {
                        name: vn,
                        type_args: _,
                    },
                ) if vn == "i64" => {
                    if let Some(Type::Enum { .. }) = context.types.get(tn) {
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            };
            if !enum_i64_ok {
                if !types_compatible(&target_type, &value_type) {
                    return Err(SemanticError::TypeMismatch {
                        expected: target_type,
                        found: value_type,
                    });
                }
            }
        }

        Statement::Expression(expr) => {
            infer_expression_type(expr, context)?;
        }

        Statement::Return(expr) => {
            if context.current_function_return_type.is_none() {
                return Err(SemanticError::ReturnOutsideFunction);
            }

            if let Some(expr) = expr {
                let expr_type = infer_expression_type(expr, context)?;
                let expected_type = context.current_function_return_type.as_ref().unwrap();

                if !types_compatible(expected_type, &expr_type) {
                    return Err(SemanticError::TypeMismatch {
                        expected: expected_type.clone(),
                        found: expr_type,
                    });
                }
            }
        }

        Statement::Break(_) => {
            if context.loop_depth == 0 {
                return Err(SemanticError::BreakOutsideLoop);
            }
        }

        Statement::Continue => {
            if context.loop_depth == 0 {
                return Err(SemanticError::ContinueOutsideLoop);
            }
        }

        Statement::ForLoop {
            variable,
            type_annotation,
            iterable,
            body,
        } => {
            let _iterable_type = infer_expression_type(iterable, context)?;

            // Assume the variable type is i64 if not specified (matches codegen loop counter)
            let var_type = type_annotation.clone().unwrap_or(Type::Identifier {
                name: "i64".to_string(),
                type_args: vec![],
            });

            context.enter_scope();
            context.define_variable(variable.clone(), var_type);
            context.loop_depth += 1;

            for stmt in body {
                analyze_statement(stmt, context)?;
            }

            context.loop_depth -= 1;
            context.exit_scope();
        }

        Statement::ImplBlock {
            type_params: _,
            trait_name,
            type_name,
            self_type,
            methods,
        } => {
            // Validate impls and collect associated types + methods into impl registries
            if let Some(tn) = trait_name {
                // Trait impl
                let trait_info = context
                    .traits
                    .get(tn)
                    .ok_or_else(|| {
                        SemanticError::UndefinedVariable(format!("trait {} not defined", tn))
                    })?
                    .clone();
                let impls_for_trait = context
                    .trait_impls
                    .get_mut(tn)
                    .and_then(|m| m.get_mut(type_name));
                let mut info = impls_for_trait.cloned().unwrap_or_default();

                // Gather provided items
                let mut provided_methods: HashMap<String, FunctionSignature> = HashMap::new();
                let mut provided_assoc: HashMap<String, Type> = HashMap::new();
                for item in methods {
                    match item {
                        Statement::ConstDecl { name, value, .. } => match value {
                            ConstValue::Type(ty) => {
                                provided_assoc.insert(name.clone(), ty.clone());
                            }
                            ConstValue::Expression(Expression::Function {
                                parameters,
                                return_type,
                                ..
                            }) => {
                                let sig = FunctionSignature {
                                    parameters: parameters
                                        .iter()
                                        .map(|p| {
                                            p.param_type.clone().unwrap_or(Type::Identifier {
                                                name: "i64".to_string(),
                                                type_args: vec![],
                                            })
                                        })
                                        .collect(),
                                    return_type: return_type.clone().unwrap_or(Type::Identifier {
                                        name: "i64".to_string(),
                                        type_args: vec![],
                                    }),
                                    is_async: false,
                                };
                                provided_methods.insert(name.clone(), sig);
                            }
                            ConstValue::Expression(_) => { /* ignore other expressions in impl items */
                            }
                            ConstValue::TableDef(table_def) => {
                                validate_table_definition(table_def, context)?;
                            }
                            ConstValue::SystemDef(system_def) => {
                                // Validate system function in trait impl context
                                validate_system_definition(system_def, context)?;
                            }
                            ConstValue::ComposeDef(_) | ConstValue::DatabaseDef(_) => {
                                // TODO: Add validation for compose and database definitions
                            }
                        },
                        Statement::ImplMethod {
                            name,
                            parameters,
                            return_type,
                            ..
                        } => {
                            let sig = FunctionSignature {
                                parameters: parameters
                                    .iter()
                                    .map(|p| {
                                        p.param_type.clone().unwrap_or(Type::Identifier {
                                            name: "i64".to_string(),
                                            type_args: vec![],
                                        })
                                    })
                                    .collect(),
                                return_type: return_type.clone().unwrap_or(Type::Identifier {
                                    name: "i64".to_string(),
                                    type_args: vec![],
                                }),
                                is_async: false,
                            };
                            provided_methods.insert(name.clone(), sig);
                        }
                        _ => {}
                    }
                }

                // Check associated types coverage
                for assoc in &trait_info.associated_types {
                    if !provided_assoc.contains_key(assoc) {
                        return Err(SemanticError::UndefinedVariable(format!(
                            "impl {} for {} missing associated type {}",
                            tn, type_name, assoc
                        )));
                    }
                }

                // Check methods coverage and signatures compatibility
                for (mname, mty) in &trait_info.methods {
                    // method type is a function type
                    let Type::Function {
                        parameters,
                        return_type,
                    } = mty
                    else {
                        continue;
                    };
                    let Some(impl_sig) = provided_methods.get(mname) else {
                        return Err(SemanticError::UndefinedFunction(format!(
                            "impl {} for {} missing method {}",
                            tn, type_name, mname
                        )));
                    };
                    // Compare param lengths
                    if parameters.len() != impl_sig.parameters.len() {
                        return Err(SemanticError::ArgumentCountMismatch {
                            expected: parameters.len(),
                            found: impl_sig.parameters.len(),
                        });
                    }
                    // Check params/return with associated type substitution
                    for (a, b) in parameters.iter().zip(&impl_sig.parameters) {
                        if !types_match_with_assoc_and_self(a, b, &provided_assoc, self_type) {
                            return Err(SemanticError::TypeMismatch {
                                expected: a.clone(),
                                found: b.clone(),
                            });
                        }
                    }
                    if !types_match_with_assoc_and_self(
                        return_type,
                        &impl_sig.return_type,
                        &provided_assoc,
                        self_type,
                    ) {
                        return Err(SemanticError::TypeMismatch {
                            expected: (*return_type.clone()).clone(),
                            found: impl_sig.return_type.clone(),
                        });
                    }
                }

                info.associated_types = provided_assoc;
                info.methods = provided_methods;
                context
                    .trait_impls
                    .entry(tn.clone())
                    .or_default()
                    .insert(type_name.clone(), info);
            } else {
                // Inherent impl: accept all method function consts
                let mut info = context.inherent_impls.remove(type_name).unwrap_or_default();
                for item in methods {
                    match item {
                        Statement::ConstDecl { name, value, .. } => {
                            if let ConstValue::Expression(Expression::Function {
                                parameters,
                                return_type,
                                ..
                            }) = value
                            {
                                let sig = FunctionSignature {
                                    parameters: parameters
                                        .iter()
                                        .map(|p| {
                                            p.param_type.clone().unwrap_or(Type::Identifier {
                                                name: "i64".to_string(),
                                                type_args: vec![],
                                            })
                                        })
                                        .collect(),
                                    return_type: return_type.clone().unwrap_or(Type::Identifier {
                                        name: "i64".to_string(),
                                        type_args: vec![],
                                    }),
                                    is_async: false,
                                };
                                info.methods.insert(name.clone(), sig);
                            }
                        }
                        Statement::ImplMethod {
                            name,
                            parameters,
                            return_type,
                            ..
                        } => {
                            let sig = FunctionSignature {
                                parameters: parameters
                                    .iter()
                                    .map(|p| {
                                        p.param_type.clone().unwrap_or(Type::Identifier {
                                            name: "i64".to_string(),
                                            type_args: vec![],
                                        })
                                    })
                                    .collect(),
                                return_type: return_type.clone().unwrap_or(Type::Identifier {
                                    name: "i64".to_string(),
                                    type_args: vec![],
                                }),
                                is_async: false,
                            };
                            info.methods.insert(name.clone(), sig);
                        }
                        _ => {}
                    }
                }
                context.inherent_impls.insert(type_name.clone(), info);
            }
        }
        Statement::Use {
            is_public: _,
            path,
            alias,
        } => {
            // Handle use statements for module imports
            context.add_use_import(path.clone(), alias.clone());
        }
        Statement::IfDef {
            condition: _,
            then_branch,
            else_branch,
        } => {
            // For now, always analyze the then branch
            // TODO: Add proper conditional compilation support
            for stmt in then_branch {
                analyze_statement(stmt, context)?;
            }
            if let Some(else_stmts) = else_branch {
                for stmt in else_stmts {
                    analyze_statement(stmt, context)?;
                }
            }
        }
        _ => {}
    }

    Ok(())
}

fn mangle_method_name(trait_name: Option<&str>, type_name: &str, method_name: &str) -> String {
    match trait_name {
        Some(tn) => format!("{}_{}_{}", tn, type_name, method_name),
        None => format!("{}_{}", type_name, method_name),
    }
}

fn types_match_with_assoc_and_self(
    expected: &Type,
    found: &Type,
    assoc: &HashMap<String, Type>,
    self_type: &Type,
) -> bool {
    fn subst(t: &Type, assoc: &HashMap<String, Type>, self_type: &Type) -> Type {
        match t {
            Type::Identifier { name, .. } if name == "self" => self_type.clone(),
            Type::Identifier { name, .. } if assoc.contains_key(name) => {
                assoc.get(name).unwrap().clone()
            }
            Type::Pointer {
                is_mutable,
                pointee,
            } => Type::Pointer {
                is_mutable: *is_mutable,
                pointee: Box::new(subst(pointee, assoc, self_type)),
            },
            Type::RawPointer { pointee, is_raw } => Type::RawPointer {
                pointee: Box::new(subst(pointee, assoc, self_type)),
                is_raw: *is_raw,
            },
            Type::Optional { inner } => Type::Optional {
                inner: Box::new(subst(inner, assoc, self_type)),
            },
            Type::Result { inner } => Type::Result {
                inner: Box::new(subst(inner, assoc, self_type)),
            },
            Type::Tuple(items) => Type::Tuple(
                items
                    .iter()
                    .map(|item| subst(item, assoc, self_type))
                    .collect(),
            ),
            Type::Function {
                parameters,
                return_type,
            } => Type::Function {
                parameters: parameters
                    .iter()
                    .map(|p| subst(p, assoc, self_type))
                    .collect(),
                return_type: Box::new(subst(return_type, assoc, self_type)),
            },
            Type::Struct { fields } => Type::Struct {
                fields: fields
                    .iter()
                    .map(|(k, v)| (k.clone(), subst(v, assoc, self_type)))
                    .collect(),
            },
            Type::Enum { variants, order } => Type::Enum {
                variants: variants
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_ref().map(|ty| subst(ty, assoc, self_type))))
                    .collect(),
                order: order.clone(),
            },
            Type::Trait {
                associated_types,
                methods,
            } => Type::Trait {
                associated_types: associated_types.clone(),
                methods: methods
                    .iter()
                    .map(|(name, ty)| (name.clone(), subst(ty, assoc, self_type)))
                    .collect(),
            },
            Type::Reference { is_mutable, inner } => Type::Reference {
                is_mutable: *is_mutable,
                inner: Box::new(subst(inner, assoc, self_type)),
            },
            other => other.clone(),
        }
    }

    let expected_subst = subst(expected, assoc, self_type);
    let found_subst = subst(found, assoc, self_type);
    expected_subst == found_subst
}

fn infer_expression_type(
    expr: &Expression,
    context: &mut SemanticContext,
) -> Result<Type, SemanticError> {
    match expr {
        Expression::Literal(literal) => Ok(match literal {
            Literal::Integer(int_lit) => Type::Identifier {
                name: int_lit.type_name().to_string(),
                type_args: vec![],
            },
            Literal::Float(_) => Type::Identifier {
                name: "f64".to_string(),
                type_args: vec![],
            },
            Literal::String(_) => Type::Identifier {
                name: "string".to_string(),
                type_args: vec![],
            },
            Literal::Boolean(_) => Type::Identifier {
                name: "bool".to_string(),
                type_args: vec![],
            },
            Literal::Char(_) => Type::Identifier {
                name: "char".to_string(),
                type_args: vec![],
            },
        }),

        Expression::Tuple(elements) => {
            let mut elem_types = Vec::with_capacity(elements.len());
            for elem in elements {
                elem_types.push(infer_expression_type(elem, context)?);
            }
            Ok(Type::Tuple(elem_types))
        }

        Expression::Identifier(name) => {
            if name == "if" {
                panic!("identifier expression encountered: 'if'");
            }
            if name == "none" {
                return Ok(Type::Optional {
                    inner: Box::new(Type::None),
                });
            }
            // Heuristic: enum variant static path lowered by parser as Identifier("Type_Variant").
            // If it matches a known enum type and existing variant name, treat as i64 tag.
            if let Some((tname, vname)) = name.split_once('_') {
                if let Some(ty) = context.types.get(tname).cloned() {
                    if let Type::Enum { variants, .. } = ty {
                        if variants.contains_key(vname) {
                            return Ok(Type::Identifier {
                                name: "i64".to_string(),
                                type_args: vec![],
                            });
                        }
                    }
                }
            }
            // Prefer variables; if not found, allow referencing functions as first-class values
            if let Some(v) = context.get_variable_type(name) {
                Ok(v.clone())
            } else if let Some(sig) = context.get_function_signature(name) {
                Ok(Type::Function {
                    parameters: sig.parameters.clone(),
                    return_type: Box::new(sig.return_type.clone()),
                })
            } else {
                Err(SemanticError::UndefinedVariable(name.clone()))
            }
        }

        Expression::BinaryOp {
            left,
            operator,
            right,
        } => {
            let left_type = infer_expression_type(left, context)?;
            let right_type = infer_expression_type(right, context)?;

            infer_binary_op_type(&left_type, operator, &right_type)
        }

        Expression::UnaryOp { operator, operand } => {
            let operand_type = infer_expression_type(operand, context)?;
            infer_unary_op_type(operator, &operand_type)
        }

        Expression::Call {
            function,
            type_args: _,
            arguments,
        } => {
            match function.as_ref() {
                Expression::Identifier(name) => {
                    // Enum variant constructor: Type::Variant(args...) lowered to Identifier("Type_Variant")
                    if let Some((tname, vname)) = name.split_once('_') {
                        if let Some(ty) = context.types.get(tname).cloned() {
                            if let Type::Enum { variants, order } = ty {
                                if let Some(payload_opt) = variants.get(vname).cloned() {
                                    match payload_opt {
                                        Some(payload_ty) => {
                                            if arguments.len() != 1 {
                                                return Err(SemanticError::ArgumentCountMismatch {
                                                    expected: 1,
                                                    found: arguments.len(),
                                                });
                                            }
                                            let arg_ty = infer_expression_type(
                                                &arguments[0].value,
                                                context,
                                            )?;
                                            if !types_compatible(&payload_ty, &arg_ty) {
                                                return Err(SemanticError::TypeMismatch {
                                                    expected: payload_ty.clone(),
                                                    found: arg_ty,
                                                });
                                            }
                                        }
                                        None => {
                                            if !arguments.is_empty() {
                                                return Err(SemanticError::ArgumentCountMismatch {
                                                    expected: 0,
                                                    found: arguments.len(),
                                                });
                                            }
                                        }
                                    }
                                    // Constructors evaluate to enum struct
                                    return Ok(Type::Enum { variants, order });
                                }
                            }
                        }
                    }
                    if name == "some" {
                        if arguments.len() != 1 {
                            return Err(SemanticError::ArgumentCountMismatch {
                                expected: 1,
                                found: arguments.len(),
                            });
                        }
                        let arg_ty = infer_expression_type(&arguments[0].value, context)?;
                        return Ok(Type::Optional {
                            inner: Box::new(arg_ty),
                        });
                    }
                    if name == "println" {
                        // Special handling for println - it's variadic, but still analyze args
                        for arg in arguments {
                            let _ = infer_expression_type(&arg.value, context)?;
                        }
                        return Ok(Type::None);
                    }
                    if name == "drop" {
                        if arguments.len() != 1 {
                            return Err(SemanticError::ArgumentCountMismatch {
                                expected: 1,
                                found: arguments.len(),
                            });
                        }
                        // Ensure the argument is well-typed even though drop accepts any owned type
                        let _ = infer_expression_type(&arguments[0].value, context)?;
                        return Ok(Type::None);
                    }
                    // Support trait static-path calls: Trait_method(x, ...)
                    if let Some((trait_name, method_name)) = name.split_once('_') {
                        // Require at least one argument as receiver
                        if let Some(first_arg) = arguments.get(0) {
                            let recv_ty = infer_expression_type(&first_arg.value, context)?;
                            // Peel pointers to get the base identifier type name
                            let base_ty_name = peel_to_identifier_name(&recv_ty);
                            if let Some(type_name) = base_ty_name {
                                if let Some(impls_for_trait) =
                                    context.trait_impls.get(trait_name).cloned()
                                {
                                    if let Some(info) = impls_for_trait.get(&type_name).cloned() {
                                        if let Some(sig) = info.methods.get(method_name).cloned() {
                                            // Validate arg count and types
                                            if arguments.len() != sig.parameters.len() {
                                                return Err(SemanticError::ArgumentCountMismatch {
                                                    expected: sig.parameters.len(),
                                                    found: arguments.len(),
                                                });
                                            }
                                            for (arg, expected_type) in
                                                arguments.iter().zip(&sig.parameters)
                                            {
                                                let arg_ty =
                                                    infer_expression_type(&arg.value, context)?;
                                                if !types_compatible(expected_type, &arg_ty) {
                                                    return Err(SemanticError::TypeMismatch {
                                                        expected: expected_type.clone(),
                                                        found: arg_ty,
                                                    });
                                                }
                                            }
                                            return Ok(sig.return_type.clone());
                                        }
                                    }
                                }
                            }
                        }
                        // Fallthrough to plain function lookup if not resolved
                    }

                    let signature_opt = context.get_function_signature(name).cloned();
                    if signature_opt.is_none() {
                        eprintln!("missing function lookup for {}", name);
                    }
                    let signature = signature_opt
                        .ok_or_else(|| SemanticError::UndefinedFunction(name.clone()))?;

                    if arguments.len() != signature.parameters.len() {
                        return Err(SemanticError::ArgumentCountMismatch {
                            expected: signature.parameters.len(),
                            found: arguments.len(),
                        });
                    }

                    for (arg, expected_type) in arguments.iter().zip(&signature.parameters) {
                        let arg_type = infer_expression_type(&arg.value, context)?;
                        if !types_compatible(expected_type, &arg_type) {
                            return Err(SemanticError::TypeMismatch {
                                expected: expected_type.clone(),
                                found: arg_type,
                            });
                        }
                    }

                    Ok(signature.return_type.clone())
                }
                Expression::FieldAccess { object, field } => {
                    // Method call: expr.method(args...) → resolve inherent first, otherwise trait for the base type
                    let recv_expr_ty = infer_expression_type(object, context)?;
                    // Special-case: function.bind(...) for partial application
                    if let Type::Function {
                        parameters,
                        return_type,
                    } = recv_expr_ty.clone()
                    {
                        if field == "bind" {
                            // Binding N arguments yields a function expecting the remaining parameters (best-effort typing)
                            let bound_n = arguments.len();
                            let remaining = if bound_n >= parameters.len() {
                                Vec::new()
                            } else {
                                parameters[bound_n..].to_vec()
                            };
                            return Ok(Type::Function {
                                parameters: remaining,
                                return_type,
                            });
                        }
                    }
                    let base_ty_name = peel_to_identifier_name(&recv_expr_ty);
                    if let Some(type_name) = base_ty_name {
                        // Inherent impl first
                        if let Some(inh) = context.inherent_impls.get(&type_name).cloned() {
                            if let Some(sig) = inh.methods.get(field).cloned() {
                                // Expect signature includes receiver as first param; args must match remaining
                                if sig.parameters.len() == 0
                                    || arguments.len() != sig.parameters.len() - 1
                                {
                                    return Err(SemanticError::ArgumentCountMismatch {
                                        expected: sig.parameters.len() - 1,
                                        found: arguments.len(),
                                    });
                                }
                                for (arg, expected_type) in
                                    arguments.iter().zip(sig.parameters.iter().skip(1))
                                {
                                    let arg_ty = infer_expression_type(&arg.value, context)?;
                                    if !types_compatible(expected_type, &arg_ty) {
                                        return Err(SemanticError::TypeMismatch {
                                            expected: expected_type.clone(),
                                            found: arg_ty,
                                        });
                                    }
                                }
                                return Ok(sig.return_type.clone());
                            }
                        }

                        // Otherwise, search trait impls for this type
                        let mut candidates: Vec<(&String, FunctionSignature)> = Vec::new();
                        for (trait_name, impls_for_trait) in &context.trait_impls {
                            if let Some(info) = impls_for_trait.get(&type_name) {
                                if let Some(sig) = info.methods.get(field) {
                                    candidates.push((trait_name, (*sig).clone()));
                                }
                            }
                        }
                        if candidates.len() > 1 {
                            let trait_list: Vec<String> =
                                candidates.into_iter().map(|(tn, _)| tn.clone()).collect();
                            return Err(SemanticError::AmbiguousMethod {
                                type_name,
                                method: field.clone(),
                                traits: trait_list,
                            });
                        } else if let Some((_tn, sig)) = candidates.into_iter().next() {
                            if sig.parameters.len() == 0
                                || arguments.len() != sig.parameters.len() - 1
                            {
                                return Err(SemanticError::ArgumentCountMismatch {
                                    expected: sig.parameters.len() - 1,
                                    found: arguments.len(),
                                });
                            }
                            for (arg, expected_type) in
                                arguments.iter().zip(sig.parameters.iter().skip(1))
                            {
                                let arg_ty = infer_expression_type(&arg.value, context)?;
                                if !types_compatible(expected_type, &arg_ty) {
                                    return Err(SemanticError::TypeMismatch {
                                        expected: expected_type.clone(),
                                        found: arg_ty,
                                    });
                                }
                            }
                            return Ok(sig.return_type.clone());
                        }
                    }
                    // Fallback: unknown method; treat as none
                    Ok(Type::None)
                }
                Expression::StaticPath { segments, .. } => {
                    // Static path call like Vec::new() or Option::Some(x)
                    let mangled_name = segments.join("_");

                    // Check if it's an enum variant constructor
                    if segments.len() >= 2 {
                        let type_name = &segments[0];
                        let variant_name = &segments[1];
                        if let Some(ty) = context.types.get(type_name).cloned() {
                            if let Type::Enum { variants, order } = ty {
                                if let Some(payload_opt) = variants.get(variant_name).cloned() {
                                    match payload_opt {
                                        Some(payload_ty) => {
                                            if arguments.len() != 1 {
                                                return Err(SemanticError::ArgumentCountMismatch {
                                                    expected: 1,
                                                    found: arguments.len(),
                                                });
                                            }
                                            let arg_ty = infer_expression_type(
                                                &arguments[0].value,
                                                context,
                                            )?;
                                            if !types_compatible(&payload_ty, &arg_ty) {
                                                return Err(SemanticError::TypeMismatch {
                                                    expected: payload_ty.clone(),
                                                    found: arg_ty,
                                                });
                                            }
                                        }
                                        None => {
                                            if !arguments.is_empty() {
                                                return Err(SemanticError::ArgumentCountMismatch {
                                                    expected: 0,
                                                    found: arguments.len(),
                                                });
                                            }
                                        }
                                    }
                                    // Constructors evaluate to enum struct
                                    return Ok(Type::Enum { variants, order });
                                }
                            }
                        }
                    }

                    // Trait static dispatch: Trait::method(arg, ...)
                    if segments.len() >= 2 && !arguments.is_empty() {
                        let trait_name = &segments[0];
                        let method_name = &segments[1];
                        let self_arg_ty = infer_expression_type(&arguments[0].value, context)?;
                        if let Some(base_type_name) = peel_to_identifier_name(&self_arg_ty) {
                            let method_sig = context
                                .trait_impls
                                .get(trait_name)
                                .and_then(|impls_for_trait| impls_for_trait.get(&base_type_name))
                                .and_then(|impl_info| impl_info.methods.get(method_name))
                                .cloned();
                            if let Some(method_sig) = method_sig {
                                if arguments.len() != method_sig.parameters.len() {
                                    return Err(SemanticError::ArgumentCountMismatch {
                                        expected: method_sig.parameters.len(),
                                        found: arguments.len(),
                                    });
                                }
                                for (arg, expected_type) in
                                    arguments.iter().zip(&method_sig.parameters)
                                {
                                    let arg_ty = infer_expression_type(&arg.value, context)?;
                                    if !types_compatible(expected_type, &arg_ty) {
                                        return Err(SemanticError::TypeMismatch {
                                            expected: expected_type.clone(),
                                            found: arg_ty,
                                        });
                                    }
                                }
                                return Ok(method_sig.return_type.clone());
                            }
                        }
                    }

                    // Otherwise treat as static function call
                    if let Some(signature) = context.get_function_signature(&mangled_name).cloned()
                    {
                        if arguments.len() != signature.parameters.len() {
                            return Err(SemanticError::ArgumentCountMismatch {
                                expected: signature.parameters.len(),
                                found: arguments.len(),
                            });
                        }
                        for (arg, expected_type) in arguments.iter().zip(&signature.parameters) {
                            let arg_type = infer_expression_type(&arg.value, context)?;
                            if !types_compatible(expected_type, &arg_type) {
                                return Err(SemanticError::TypeMismatch {
                                    expected: expected_type.clone(),
                                    found: arg_type,
                                });
                            }
                        }
                        return Ok(signature.return_type.clone());
                    }

                    Err(SemanticError::UndefinedFunction(mangled_name))
                }
                _ => {
                    // Function expressions not yet supported
                    Ok(Type::None)
                }
            }
        }

        Expression::Match { value, arms } => {
            let value_type = infer_expression_type(value, context)?;
            for arm in arms {
                analyze_pattern(&arm.pattern, context, &value_type)?;
                let _ = infer_expression_type(&arm.body, context)?;
            }
            // For now matches yield i64 (we lower branches to i64 and phi them)
            Ok(Type::Identifier {
                name: "i64".to_string(),
                type_args: vec![],
            })
        }

        Expression::FieldAccess { object, field } => {
            let object_type = infer_expression_type(object, context)?;
            // Resolve through pointers
            let mut base_ty = object_type.clone();
            if let Type::Pointer { pointee, .. } | Type::RawPointer { pointee, .. } = &object_type {
                base_ty = (*pointee.clone()).clone();
            }
            // If base is an identifier type referring to a struct, pick field type
            if let Type::Identifier { name, type_args: _ } = &base_ty {
                if let Some(ty) = context.types.get(name).cloned() {
                    if let Type::Struct { fields } = ty {
                        if let Some(fty) = fields.get(field) {
                            return Ok(fty.clone());
                        }
                    }
                }
            }
            // Fallback
            Ok(Type::Identifier {
                name: "i64".to_string(),
                type_args: vec![],
            })
        }

        Expression::Index { object, indices: _ } => {
            let object_type = infer_expression_type(object, context)?;
            match object_type {
                Type::Matrix {
                    element_type,
                    dimensions: _,
                } => {
                    // If full indexing provided (indices length equals dimensions), return element type
                    // Also allow 1D indexing into 1D matrix (vector)
                    // For now, we don't validate index types rigorously
                    Ok((*element_type).clone())
                }
                _ => Ok(Type::None),
            }
        }

        Expression::Cast { value, to_type } => {
            // Validate the input expression type even if we don't enforce compatibility yet
            let _ = infer_expression_type(value, context)?;
            Ok(to_type.clone())
        }

        Expression::If {
            condition,
            then_branch: _,
            else_branch: _,
        } => {
            let condition_type = infer_expression_type(condition, context)?;
            // Accept common truthy types (bool, numeric, string, pointers)
            let is_bool = types_compatible(
                &Type::Identifier {
                    name: "bool".to_string(),
                    type_args: vec![],
                },
                &condition_type,
            );
            let is_num = is_numeric_type(&condition_type);
            let is_str = matches!(condition_type, Type::Identifier { ref name, type_args: _ } if name == "string");
            let is_ptr = matches!(
                &condition_type,
                Type::Pointer { .. } | Type::RawPointer { .. }
            );
            if !(is_bool || is_num || is_str || is_ptr) {
                return Err(SemanticError::TypeMismatch {
                    expected: Type::Identifier {
                        name: "bool".to_string(),
                        type_args: vec![],
                    },
                    found: condition_type,
                });
            }

            // For now, assume if expressions return none
            Ok(Type::None)
        }

        Expression::IfExpr {
            condition,
            then_expr,
            else_expr,
        } => {
            let condition_type = infer_expression_type(condition, context)?;
            let bool_type = Type::Identifier {
                name: "bool".to_string(),
                type_args: vec![],
            };
            let is_bool = types_compatible(&bool_type, &condition_type);
            let is_num = is_numeric_type(&condition_type);
            let is_str = is_string_type(&condition_type);
            let is_ptr = matches!(
                condition_type,
                Type::Pointer { .. } | Type::RawPointer { .. }
            );
            if !(is_bool || is_num || is_str || is_ptr) {
                return Err(SemanticError::TypeMismatch {
                    expected: bool_type,
                    found: condition_type,
                });
            }

            let then_type = infer_expression_type(then_expr, context)?;
            match else_expr {
                Some(branch) => {
                    let else_type = infer_expression_type(branch, context)?;
                    let then_is_none = matches!(then_type, Type::None);
                    let else_is_none = matches!(else_type, Type::None);
                    if types_compatible(&then_type, &else_type)
                        || types_compatible(&else_type, &then_type)
                    {
                        if then_is_none {
                            Ok(else_type)
                        } else if else_is_none {
                            Ok(then_type)
                        } else {
                            Ok(then_type)
                        }
                    } else {
                        Err(SemanticError::TypeMismatch {
                            expected: then_type,
                            found: else_type,
                        })
                    }
                }
                None => Ok(then_type),
            }
        }

        Expression::Block { statements: _ } => {
            // For now, assume blocks return none
            Ok(Type::None)
        }

        Expression::Range { start, end, step } => {
            let st = infer_expression_type(start, context)?;
            let et = infer_expression_type(end, context)?;
            if !(is_numeric_type(&st) && is_numeric_type(&et)) {
                return Err(SemanticError::TypeMismatch {
                    expected: Type::Identifier {
                        name: "i64".to_string(),
                        type_args: vec![],
                    },
                    found: st,
                });
            }
            if let Some(s) = step {
                let s_ty = infer_expression_type(s, context)?;
                if !is_numeric_type(&s_ty) {
                    return Err(SemanticError::TypeMismatch {
                        expected: Type::Identifier {
                            name: "i64".to_string(),
                            type_args: vec![],
                        },
                        found: s_ty,
                    });
                }
                // If step is a literal zero, reject.
                if let Expression::Literal(Literal::Integer(ival)) = s.as_ref() {
                    if ival.value == 0 {
                        return Err(SemanticError::InvalidRangeStepZero);
                    }
                }
            }
            Ok(Type::Identifier {
                name: "i64".to_string(),
                type_args: vec![],
            })
        }
        Expression::VecNew {
            element_type,
            length,
            fill,
            additional_dimensions,
        } => {
            let mut recorded_dims: Vec<usize> = Vec::new();

            let validate_dimension = |dim_expr: &Expression,
                                      recorded_dims: &mut Vec<usize>,
                                      context: &mut SemanticContext|
             -> Result<(), SemanticError> {
                let dim_ty = infer_expression_type(dim_expr, context)?;
                if !is_numeric_type(&dim_ty) {
                    return Err(SemanticError::TypeMismatch {
                        expected: Type::Identifier {
                            name: "i64".to_string(),
                            type_args: vec![],
                        },
                        found: dim_ty,
                    });
                }
                if let Expression::Literal(Literal::Integer(int_lit)) = dim_expr {
                    if int_lit.value <= usize::MAX as u128 {
                        recorded_dims.push(int_lit.value as usize);
                    }
                }
                Ok(())
            };

            if let Some(len_expr) = length.as_ref() {
                validate_dimension(len_expr.as_ref(), &mut recorded_dims, context)?;
            }
            for dim_expr in additional_dimensions.iter() {
                validate_dimension(dim_expr, &mut recorded_dims, context)?;
            }

            if let Some(fill_expr) = fill.as_ref() {
                let fill_ty = infer_expression_type(fill_expr.as_ref(), context)?;
                let expected = (*element_type).clone();
                if !types_compatible(&expected, &fill_ty) {
                    return Err(SemanticError::TypeMismatch {
                        expected,
                        found: fill_ty,
                    });
                }
            }

            Ok(Type::Matrix {
                element_type: Box::new((*element_type).clone()),
                dimensions: recorded_dims,
            })
        }
        Expression::VecLiteral { elements } => {
            let mut has_float = false;
            let mut has_bool = false;
            let mut has_non_bool = false;

            for e in elements {
                if let Ok(t) = infer_expression_type(e, context) {
                    match t {
                        Type::Identifier { name, type_args: _ }
                            if name == "f32" || name == "f64" =>
                        {
                            has_float = true;
                        }
                        Type::Identifier { name, type_args: _ } if name == "bool" => {
                            has_bool = true;
                        }
                        _ => {
                            has_non_bool = true;
                        }
                    }
                }
            }

            let elem = if has_float {
                Type::Identifier {
                    name: "f64".to_string(),
                    type_args: vec![],
                }
            } else if has_bool && !has_non_bool {
                Type::Identifier {
                    name: "bool".to_string(),
                    type_args: vec![],
                }
            } else {
                Type::Identifier {
                    name: "i64".to_string(),
                    type_args: vec![],
                }
            };

            Ok(Type::Matrix {
                element_type: Box::new(elem),
                dimensions: vec![elements.len()],
            })
        }
        Expression::Matrix { rows } => {
            // Determine dimensions and element type (numeric best-effort)
            let row_count = rows.len();
            let col_count = if row_count > 0 { rows[0].len() } else { 0 };
            // validate equal columns
            for r in rows {
                if r.len() != col_count { /* ignore mismatch for now */ }
            }
            // infer element type by scanning; prefer f64 if any float present, else i64
            let mut has_float = false;
            let mut has_bool = false;
            let mut has_non_bool = false;
            for r in rows {
                for e in r {
                    if let Ok(t) = infer_expression_type(e, context) {
                        match t {
                            Type::Identifier { name, type_args: _ }
                                if name == "f32" || name == "f64" =>
                            {
                                has_float = true;
                            }
                            Type::Identifier { name, type_args: _ } if name == "bool" => {
                                has_bool = true;
                            }
                            _ => {
                                has_non_bool = true;
                            }
                        }
                    }
                }
            }
            let elem = if has_float {
                Type::Identifier {
                    name: "f64".to_string(),
                    type_args: vec![],
                }
            } else if has_bool && !has_non_bool {
                Type::Identifier {
                    name: "bool".to_string(),
                    type_args: vec![],
                }
            } else {
                Type::Identifier {
                    name: "i64".to_string(),
                    type_args: vec![],
                }
            };
            let dims = if row_count <= 1 {
                vec![col_count]
            } else {
                vec![row_count, col_count]
            };
            Ok(Type::Matrix {
                element_type: Box::new(elem),
                dimensions: dims,
            })
        }
        Expression::Query(_query_spec) => {
            // TODO: Implement proper type inference for query expressions
            // For now, queries return a table/result type
            Ok(Type::Identifier {
                name: "QueryResult".to_string(),
                type_args: vec![],
            })
        }
        Expression::Shader { .. } => {
            // Shader expressions return a shader type
            Ok(Type::Identifier {
                name: "Shader".to_string(),
                type_args: vec![],
            })
        }
        Expression::StaticPath { segments, .. } => {
            // Static path like Vec::new or Option::Some
            // Mangle to identifier and look up as function or enum variant
            let mangled_name = segments.join("_");

            // Check if it's an enum variant
            if segments.len() >= 2 {
                let type_name = &segments[0];
                let _variant_name = &segments[1];
                if let Some(ty) = context.types.get(type_name).cloned() {
                    if let Type::Enum { .. } = ty {
                        // It's an enum variant - return i64 for tag
                        return Ok(Type::Identifier {
                            name: "i64".to_string(),
                            type_args: vec![],
                        });
                    }
                }
            }

            // Otherwise treat as function reference
            if let Some(sig) = context.get_function_signature(&mangled_name) {
                Ok(Type::Function {
                    parameters: sig.parameters.clone(),
                    return_type: Box::new(sig.return_type.clone()),
                })
            } else {
                Err(SemanticError::UndefinedFunction(mangled_name))
            }
        }
        Expression::Question(expr) => {
            let opt_type = infer_expression_type(expr, context)?;
            match opt_type {
                Type::Optional { inner } => {
                    let payload_type = inner.as_ref().clone();
                    if let Some(expected_ret) = context.current_function_return_type.as_ref() {
                        match expected_ret {
                            Type::Optional {
                                inner: expected_inner,
                            } => {
                                if !types_compatible(expected_inner, inner.as_ref()) {
                                    return Err(SemanticError::TypeMismatch {
                                        expected: Type::Optional {
                                            inner: expected_inner.clone(),
                                        },
                                        found: Type::Optional {
                                            inner: inner.clone(),
                                        },
                                    });
                                }
                            }
                            _ => {
                                return Err(SemanticError::InvalidOperation {
                                    operator: "?".to_string(),
                                    operand_types: vec![expected_ret.clone()],
                                });
                            }
                        }
                    } else {
                        return Err(SemanticError::InvalidOperation {
                            operator: "?".to_string(),
                            operand_types: vec![Type::Optional {
                                inner: inner.clone(),
                            }],
                        });
                    }
                    Ok(payload_type)
                }
                other => Err(SemanticError::InvalidOperation {
                    operator: "?".to_string(),
                    operand_types: vec![other],
                }),
            }
        }
        _ => {
            // For other expression types, return none for now
            Ok(Type::None)
        }
    }
}
fn peel_to_identifier_name(t: &Type) -> Option<String> {
    let mut cur = t;
    loop {
        match cur {
            Type::Pointer { pointee, .. } => cur = pointee.as_ref(),
            Type::RawPointer { pointee, .. } => cur = pointee.as_ref(),
            Type::Optional { inner } => cur = inner.as_ref(),
            Type::Result { inner } => cur = inner.as_ref(),
            Type::Identifier { name, type_args: _ } => return Some(name.clone()),
            _ => return None,
        }
    }
}

fn infer_binary_op_type(
    left: &Type,
    operator: &BinaryOperator,
    right: &Type,
) -> Result<Type, SemanticError> {
    match operator {
        BinaryOperator::Add => {
            if is_string_type(left) && is_string_type(right) {
                return Ok(Type::Identifier {
                    name: "string".to_string(),
                    type_args: vec![],
                });
            }
            if matches!(left, Type::None) {
                return Ok(right.clone());
            }
            if matches!(right, Type::None) {
                return Ok(left.clone());
            }
            if types_compatible(left, right) && (is_numeric_type(left) || is_numeric_type(right)) {
                Ok(promote_numeric_types(left, right))
            } else {
                Err(SemanticError::InvalidOperation {
                    operator: format!("{:?}", operator),
                    operand_types: vec![left.clone(), right.clone()],
                })
            }
        }

        BinaryOperator::Sub | BinaryOperator::Mul | BinaryOperator::Div => {
            if matches!(left, Type::None) {
                return Ok(right.clone());
            }
            if matches!(right, Type::None) {
                return Ok(left.clone());
            }
            if types_compatible(left, right) && (is_numeric_type(left) || is_numeric_type(right)) {
                Ok(promote_numeric_types(left, right))
            } else {
                Err(SemanticError::InvalidOperation {
                    operator: format!("{:?}", operator),
                    operand_types: vec![left.clone(), right.clone()],
                })
            }
        }

        BinaryOperator::Mod => {
            if matches!(left, Type::None) {
                return Ok(right.clone());
            }
            if matches!(right, Type::None) {
                return Ok(left.clone());
            }
            if types_compatible(left, right) && is_integer_type(left) && is_integer_type(right) {
                Ok(promote_numeric_types(left, right))
            } else {
                Err(SemanticError::InvalidOperation {
                    operator: format!("{:?}", operator),
                    operand_types: vec![left.clone(), right.clone()],
                })
            }
        }

        BinaryOperator::Equal
        | BinaryOperator::NotEqual
        | BinaryOperator::Less
        | BinaryOperator::Greater
        | BinaryOperator::LessEqual
        | BinaryOperator::GreaterEqual => {
            if types_compatible(left, right) {
                Ok(Type::Identifier {
                    name: "bool".to_string(),
                    type_args: vec![],
                })
            } else {
                Err(SemanticError::InvalidOperation {
                    operator: format!("{:?}", operator),
                    operand_types: vec![left.clone(), right.clone()],
                })
            }
        }

        BinaryOperator::And | BinaryOperator::Or | BinaryOperator::Xor => {
            let bool_type = Type::Identifier {
                name: "bool".to_string(),
                type_args: vec![],
            };
            if types_compatible(left, &bool_type) && types_compatible(right, &bool_type) {
                Ok(bool_type)
            } else {
                Err(SemanticError::InvalidOperation {
                    operator: format!("{:?}", operator),
                    operand_types: vec![left.clone(), right.clone()],
                })
            }
        }

        _ => {
            // For other operators, return the left type for now
            Ok(left.clone())
        }
    }
}

fn infer_unary_op_type(operator: &UnaryOperator, operand: &Type) -> Result<Type, SemanticError> {
    match operator {
        UnaryOperator::Negate => {
            if is_numeric_type(operand) {
                Ok(operand.clone())
            } else {
                Err(SemanticError::InvalidOperation {
                    operator: format!("{:?}", operator),
                    operand_types: vec![operand.clone()],
                })
            }
        }

        UnaryOperator::Not => {
            let bool_type = Type::Identifier {
                name: "bool".to_string(),
                type_args: vec![],
            };
            if types_compatible(operand, &bool_type) {
                Ok(bool_type)
            } else {
                Err(SemanticError::InvalidOperation {
                    operator: format!("{:?}", operator),
                    operand_types: vec![operand.clone()],
                })
            }
        }

        UnaryOperator::AddressOf => Ok(Type::Pointer {
            is_mutable: false,
            pointee: Box::new(operand.clone()),
        }),

        UnaryOperator::MutAddressOf => Ok(Type::Pointer {
            is_mutable: true,
            pointee: Box::new(operand.clone()),
        }),

        UnaryOperator::Deref => match operand {
            Type::Pointer { pointee, .. } | Type::RawPointer { pointee, .. } => {
                Ok(pointee.as_ref().clone())
            }
            _ => Err(SemanticError::InvalidOperation {
                operator: format!("{:?}", operator),
                operand_types: vec![operand.clone()],
            }),
        },

        _ => Ok(operand.clone()),
    }
}

fn types_compatible(expected: &Type, found: &Type) -> bool {
    match (expected, found) {
        // Treat enums as i64-compatible for now (repr i64)
        (
            Type::Identifier {
                name: e,
                type_args: _,
            },
            Type::Enum { .. },
        )
        | (
            Type::Enum { .. },
            Type::Identifier {
                name: e,
                type_args: _,
            },
        ) => e == "i64",
        // Allow implicit address-of: passing T where &T is expected
        (
            Type::Pointer {
                pointee: exp_pointee,
                ..
            },
            Type::Identifier {
                name: found_name,
                type_args: _,
            },
        ) if !matches!(found_name.as_str(), "i64" | "u64") => {
            if let Type::Identifier {
                name: exp_name,
                type_args: _,
            } = exp_pointee.as_ref()
            {
                return exp_name == found_name;
            }
            false
        }
        (
            Type::Pointer { .. } | Type::RawPointer { .. } | Type::Reference { .. },
            Type::Identifier { name, .. },
        ) if matches!(name.as_str(), "i64" | "u64") => true,
        (
            Type::Identifier { name, .. },
            Type::Pointer { .. } | Type::RawPointer { .. } | Type::Reference { .. },
        ) if matches!(name.as_str(), "i64" | "u64") => true,
        (
            Type::Optional {
                inner: expected_inner,
            },
            Type::Optional { inner: found_inner },
        ) => {
            matches!(**found_inner, Type::None)
                || matches!(**expected_inner, Type::None)
                || types_compatible(expected_inner.as_ref(), found_inner.as_ref())
        }
        (Type::Optional { .. }, Type::None) | (Type::None, Type::Optional { .. }) => true,
        (
            Type::Identifier {
                name: a,
                type_args: _,
            },
            Type::Identifier {
                name: b,
                type_args: _,
            },
        ) => {
            if a == b {
                return true;
            }
            if (a == "String" && b == "string") || (a == "string" && b == "String") {
                return true;
            }
            // Allow any pair of numeric scalar types to be used together; codegen will unify bit-widths.
            let na = a.as_str();
            let nb = b.as_str();
            let is_num_a = is_numeric_identifier_name(na);
            let is_num_b = is_numeric_identifier_name(nb);
            if is_num_a && is_num_b {
                return true;
            }
            false
        }
        (
            Type::RawPointer { .. } | Type::Pointer { .. } | Type::Reference { .. },
            Type::Identifier { name, .. },
        )
        | (
            Type::Identifier { name, .. },
            Type::RawPointer { .. } | Type::Pointer { .. } | Type::Reference { .. },
        ) => matches!(name.as_str(), "i64" | "u64"),
        (Type::Tuple(exp_elems), Type::Tuple(found_elems)) => {
            if exp_elems.len() != found_elems.len() {
                return false;
            }
            exp_elems
                .iter()
                .zip(found_elems.iter())
                .all(|(e, f)| types_compatible(e, f))
        }
        (Type::None, _) | (_, Type::None) => true,
        _ => expected == found,
    }
}

fn bind_binding_pattern(
    pattern: &BindingPattern,
    value_type: &Type,
    context: &mut SemanticContext,
) -> Result<(), SemanticError> {
    match pattern {
        BindingPattern::Identifier(name) => {
            context.define_variable(name.clone(), value_type.clone());
            Ok(())
        }
        BindingPattern::Discard => Ok(()),
        BindingPattern::Tuple(elements) => match value_type {
            Type::Tuple(element_types) => {
                if elements.len() != element_types.len() {
                    return Err(SemanticError::ArgumentCountMismatch {
                        expected: element_types.len(),
                        found: elements.len(),
                    });
                }
                for (element_pattern, element_type) in elements.iter().zip(element_types.iter()) {
                    bind_binding_pattern(element_pattern, element_type, context)?;
                }
                Ok(())
            }
            _ => Err(SemanticError::TypeMismatch {
                expected: Type::Tuple(Vec::new()),
                found: value_type.clone(),
            }),
        },
    }
}

fn analyze_pattern(
    pattern: &Expression,
    context: &mut SemanticContext,
    scrutinee_type: &Type,
) -> Result<(), SemanticError> {
    let resolved_scrutinee = match scrutinee_type {
        Type::Identifier { name, type_args } if type_args.is_empty() => context
            .types
            .get(name)
            .cloned()
            .unwrap_or_else(|| scrutinee_type.clone()),
        _ => scrutinee_type.clone(),
    };

    match pattern {
        Expression::Identifier(name) if name == "_" => Ok(()),
        Expression::Identifier(name) => {
            if let Type::Optional { .. } = &resolved_scrutinee {
                if name == "none" {
                    return Ok(());
                }
            }
            // Enum variant path encoded as "Type_Variant"
            if let Some((_tname, vname)) = name.split_once('_') {
                if let Type::Enum { variants, .. } = &resolved_scrutinee {
                    if variants.contains_key(vname) {
                        return Ok(());
                    } else {
                        return Ok(());
                    }
                } else {
                    return Err(SemanticError::TypeMismatch {
                        expected: Type::None,
                        found: scrutinee_type.clone(),
                    });
                }
            }

            // Otherwise treat as variable binding
            context.define_variable(name.clone(), scrutinee_type.clone());
            Ok(())
        }
        Expression::Tuple(elements) => {
            if let Type::Tuple(component_types) = &resolved_scrutinee {
                if elements.len() != component_types.len() {
                    return Err(SemanticError::ArgumentCountMismatch {
                        expected: component_types.len(),
                        found: elements.len(),
                    });
                }
                for (elem_pattern, elem_type) in elements.iter().zip(component_types.iter()) {
                    analyze_pattern(elem_pattern, context, elem_type)?;
                }
                Ok(())
            } else {
                Err(SemanticError::TypeMismatch {
                    expected: Type::Tuple(Vec::new()),
                    found: scrutinee_type.clone(),
                })
            }
        }
        Expression::Call {
            function,
            type_args,
            arguments,
        } => {
            // Destructuring: Color_Green(x)
            match function.as_ref() {
                Expression::Identifier(func_name) => {
                    if func_name == "some" {
                        if let Type::Optional { inner } = &resolved_scrutinee {
                            if !type_args.is_empty() || arguments.len() != 1 {
                                return Err(SemanticError::UndefinedVariable(
                                    "invalid pattern".to_string(),
                                ));
                            }
                            let arg = &arguments[0];
                            if arg.name.is_some() {
                                return Err(SemanticError::UndefinedVariable(
                                    "named arg in pattern".to_string(),
                                ));
                            }
                            match &arg.value {
                                Expression::Identifier(var_name) => {
                                    if var_name != "_" {
                                        context.define_variable(
                                            var_name.clone(),
                                            inner.as_ref().clone(),
                                        );
                                    }
                                    return Ok(());
                                }
                                Expression::Call {
                                    function: ref_func,
                                    type_args: ref_type_args,
                                    arguments: ref_arguments,
                                } => {
                                    if !ref_type_args.is_empty()
                                        || ref_arguments.len() != 1
                                        || ref_arguments[0].name.is_some()
                                    {
                                        return Err(SemanticError::UndefinedVariable(
                                            "invalid pattern".to_string(),
                                        ));
                                    }

                                    if let Expression::Identifier(ref_name) = ref_func.as_ref() {
                                        if ref_name == "ref" {
                                            match &ref_arguments[0].value {
                                                Expression::Identifier(var_name) => {
                                                    if var_name != "_" {
                                                        context.define_variable(
                                                            var_name.clone(),
                                                            Type::Reference {
                                                                inner: Box::new(
                                                                    inner.as_ref().clone(),
                                                                ),
                                                                is_mutable: false,
                                                            },
                                                        );
                                                    }
                                                    return Ok(());
                                                }
                                                _ => {
                                                    return Err(SemanticError::UndefinedVariable(
                                                        "invalid pattern".to_string(),
                                                    ));
                                                }
                                            }
                                        }
                                    }

                                    return Err(SemanticError::UndefinedVariable(
                                        "invalid pattern".to_string(),
                                    ));
                                }
                                _ => {
                                    return Err(SemanticError::UndefinedVariable(
                                        "invalid pattern".to_string(),
                                    ));
                                }
                            }
                        }
                    }
                    if let Some((_tname, vname)) = func_name.split_once('_') {
                        if let Type::Enum { variants, .. } = &resolved_scrutinee {
                            if let Some(payload_type_opt) = variants.get(vname) {
                                if let Some(payload_type) = payload_type_opt {
                                    // Bind each argument as a variable of payload_type
                                    for arg in arguments {
                                        if arg.name.is_some() {
                                            return Err(SemanticError::UndefinedVariable(
                                                "named arg in pattern".to_string(),
                                            ));
                                        }
                                        match &arg.value {
                                            Expression::Identifier(var_name) => {
                                                if var_name == "_" {
                                                    // Wildcard, no binding
                                                } else {
                                                    context.variables.insert(
                                                        var_name.clone(),
                                                        payload_type.clone(),
                                                    );
                                                }
                                            }
                                            Expression::Call {
                                                function: ref_func,
                                                type_args: ref_type_args,
                                                arguments: ref_arguments,
                                            } => {
                                                if !ref_type_args.is_empty()
                                                    || ref_arguments.len() != 1
                                                    || ref_arguments[0].name.is_some()
                                                {
                                                    return Err(SemanticError::UndefinedVariable(
                                                        "invalid pattern".to_string(),
                                                    ));
                                                }

                                                if let Expression::Identifier(ref_name) =
                                                    ref_func.as_ref()
                                                {
                                                    if ref_name == "ref" {
                                                        match &ref_arguments[0].value {
                                                            Expression::Identifier(var_name) => {
                                                                if var_name != "_" {
                                                                    context.variables.insert(
                                                                        var_name.clone(),
                                                                        Type::Reference {
                                                                            inner: Box::new(
                                                                                payload_type
                                                                                    .clone(),
                                                                            ),
                                                                            is_mutable: false,
                                                                        },
                                                                    );
                                                                }
                                                            }
                                                            _ => {
                                                                return Err(
                                                                    SemanticError::UndefinedVariable(
                                                                        "invalid pattern"
                                                                            .to_string(),
                                                                    ),
                                                                );
                                                            }
                                                        }
                                                    } else {
                                                        return Err(
                                                            SemanticError::UndefinedVariable(
                                                                "invalid pattern".to_string(),
                                                            ),
                                                        );
                                                    }
                                                } else {
                                                    return Err(SemanticError::UndefinedVariable(
                                                        "invalid pattern".to_string(),
                                                    ));
                                                }
                                            }
                                            _ => {
                                                return Err(SemanticError::UndefinedVariable(
                                                    "invalid pattern".to_string(),
                                                ));
                                            }
                                        };
                                    }
                                    Ok(())
                                } else {
                                    if !arguments.is_empty() {
                                        return Err(SemanticError::UndefinedVariable(
                                            "invalid pattern".to_string(),
                                        ));
                                    }
                                    Ok(())
                                }
                            } else {
                                Err(SemanticError::UndefinedVariable(func_name.clone()))
                            }
                        } else {
                            Err(SemanticError::TypeMismatch {
                                expected: Type::None,
                                found: scrutinee_type.clone(),
                            })
                        }
                    } else {
                        Err(SemanticError::UndefinedVariable(
                            "invalid pattern".to_string(),
                        ))
                    }
                }
                Expression::StaticPath { segments, .. } => {
                    if segments.len() < 2 {
                        return Err(SemanticError::UndefinedVariable(
                            "invalid pattern".to_string(),
                        ));
                    }
                    if let Type::Enum { variants, .. } = &resolved_scrutinee {
                        let variant_name = &segments[1];
                        if let Some(payload_type_opt) = variants.get(variant_name.as_str()) {
                            match payload_type_opt {
                                Some(payload_type) => {
                                    if arguments.len() != 1 {
                                        return Err(SemanticError::ArgumentCountMismatch {
                                            expected: 1,
                                            found: arguments.len(),
                                        });
                                    }
                                    let arg = &arguments[0];
                                    if arg.name.is_some() {
                                        return Err(SemanticError::UndefinedVariable(
                                            "named arg in pattern".to_string(),
                                        ));
                                    }
                                    match &arg.value {
                                        Expression::Identifier(var_name) => {
                                            if var_name != "_" {
                                                context
                                                    .variables
                                                    .insert(var_name.clone(), payload_type.clone());
                                            }
                                        }
                                        Expression::Call {
                                            function: ref_func,
                                            type_args: ref_type_args,
                                            arguments: ref_arguments,
                                        } => {
                                            if !ref_type_args.is_empty()
                                                || ref_arguments.len() != 1
                                                || ref_arguments[0].name.is_some()
                                            {
                                                return Err(SemanticError::UndefinedVariable(
                                                    "invalid pattern".to_string(),
                                                ));
                                            }
                                            if let Expression::Identifier(ref_name) =
                                                ref_func.as_ref()
                                            {
                                                if ref_name == "ref" {
                                                    match &ref_arguments[0].value {
                                                        Expression::Identifier(var_name) => {
                                                            if var_name != "_" {
                                                                context.variables.insert(
                                                                    var_name.clone(),
                                                                    Type::Reference {
                                                                        inner: Box::new(
                                                                            payload_type.clone(),
                                                                        ),
                                                                        is_mutable: false,
                                                                    },
                                                                );
                                                            }
                                                        }
                                                        _ => {
                                                            return Err(
                                                                SemanticError::UndefinedVariable(
                                                                    "invalid pattern".to_string(),
                                                                ),
                                                            );
                                                        }
                                                    }
                                                } else {
                                                    return Err(SemanticError::UndefinedVariable(
                                                        "invalid pattern".to_string(),
                                                    ));
                                                }
                                            } else {
                                                return Err(SemanticError::UndefinedVariable(
                                                    "invalid pattern".to_string(),
                                                ));
                                            }
                                        }
                                        _ => {
                                            return Err(SemanticError::UndefinedVariable(
                                                "invalid pattern".to_string(),
                                            ));
                                        }
                                    };
                                    Ok(())
                                }
                                None => {
                                    if !arguments.is_empty() {
                                        return Err(SemanticError::ArgumentCountMismatch {
                                            expected: 0,
                                            found: arguments.len(),
                                        });
                                    }
                                    Ok(())
                                }
                            }
                        } else {
                            Err(SemanticError::UndefinedVariable(segments.join("::")))
                        }
                    } else {
                        Err(SemanticError::TypeMismatch {
                            expected: Type::None,
                            found: scrutinee_type.clone(),
                        })
                    }
                }
                _ => Err(SemanticError::UndefinedVariable(
                    "invalid pattern".to_string(),
                )),
            }
        }
        Expression::StaticPath { segments, .. } => {
            if segments.len() < 2 {
                return Err(SemanticError::UndefinedVariable(
                    "invalid pattern".to_string(),
                ));
            }
            if let Type::Enum { variants, .. } = &resolved_scrutinee {
                let variant_name = &segments[1];
                if variants.contains_key(variant_name.as_str()) {
                    return Ok(());
                }
                return Err(SemanticError::UndefinedVariable(segments.join("::")));
            }
            Err(SemanticError::TypeMismatch {
                expected: Type::None,
                found: scrutinee_type.clone(),
            })
        }
        _ => Err(SemanticError::UndefinedVariable(format!(
            "unknown pattern: {:?}",
            pattern
        ))),
    }
}

const NUMERIC_PROMOTION_ORDER: [&str; 10] = [
    "u8", "i8", "u16", "i16", "u32", "i32", "u64", "i64", "f32", "f64",
];

fn is_numeric_identifier_name(name: &str) -> bool {
    NUMERIC_PROMOTION_ORDER.iter().any(|n| *n == name)
}

fn numeric_type_name(t: &Type) -> Option<&str> {
    if let Type::Identifier { name, .. } = t {
        let as_str = name.as_str();
        if is_numeric_identifier_name(as_str) {
            return Some(as_str);
        }
    }
    None
}

fn numeric_rank(name: &str) -> Option<usize> {
    NUMERIC_PROMOTION_ORDER.iter().position(|n| *n == name)
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum NumericKind {
    Unsigned,
    Signed,
    Float,
}

fn numeric_info(name: &str) -> Option<(usize, NumericKind)> {
    match name {
        "u8" => Some((8, NumericKind::Unsigned)),
        "i8" => Some((8, NumericKind::Signed)),
        "u16" => Some((16, NumericKind::Unsigned)),
        "i16" => Some((16, NumericKind::Signed)),
        "u32" => Some((32, NumericKind::Unsigned)),
        "i32" => Some((32, NumericKind::Signed)),
        "u64" => Some((64, NumericKind::Unsigned)),
        "i64" => Some((64, NumericKind::Signed)),
        "f32" => Some((32, NumericKind::Float)),
        "f64" => Some((64, NumericKind::Float)),
        _ => None,
    }
}

fn signed_type_for_bits(bits: usize) -> Option<&'static str> {
    match bits {
        0..=8 => Some("i8"),
        9..=16 => Some("i16"),
        17..=32 => Some("i32"),
        33..=64 => Some("i64"),
        _ => None,
    }
}

fn promote_numeric_types(left: &Type, right: &Type) -> Type {
    let left_name = numeric_type_name(left);
    let right_name = numeric_type_name(right);

    match (left_name, right_name) {
        (Some(ln), Some(rn)) => {
            if let (Some((l_bits, l_kind)), Some((r_bits, r_kind))) =
                (numeric_info(ln), numeric_info(rn))
            {
                if l_kind == NumericKind::Float || r_kind == NumericKind::Float {
                    let target_bits =
                        if l_kind == NumericKind::Float && r_kind == NumericKind::Float {
                            l_bits.max(r_bits)
                        } else if l_kind == NumericKind::Float {
                            l_bits
                        } else {
                            r_bits
                        };
                    let target = if target_bits >= 64 { "f64" } else { "f32" };
                    return Type::Identifier {
                        name: target.to_string(),
                        type_args: vec![],
                    };
                }

                if (l_kind == NumericKind::Signed && r_kind == NumericKind::Unsigned)
                    || (l_kind == NumericKind::Unsigned && r_kind == NumericKind::Signed)
                {
                    let max_bits = l_bits.max(r_bits);
                    let target = signed_type_for_bits(max_bits).unwrap_or("i64");
                    return Type::Identifier {
                        name: target.to_string(),
                        type_args: vec![],
                    };
                }
            }
            let l_rank = numeric_rank(ln).unwrap_or(0);
            let r_rank = numeric_rank(rn).unwrap_or(0);
            let target = if l_rank >= r_rank { ln } else { rn };
            Type::Identifier {
                name: target.to_string(),
                type_args: vec![],
            }
        }
        (Some(_), None) => left.clone(),
        (None, Some(_)) => right.clone(),
        _ => left.clone(),
    }
}

fn is_numeric_type(t: &Type) -> bool {
    match t {
        Type::Identifier { name, .. } => is_numeric_identifier_name(name.as_str()),
        _ => false,
    }
}

fn is_integer_type(t: &Type) -> bool {
    if let Some(name) = numeric_type_name(t) {
        if let Some((_, kind)) = numeric_info(name) {
            return kind != NumericKind::Float;
        }
    }
    false
}

fn is_string_type(t: &Type) -> bool {
    if let Type::Identifier { name, .. } = t {
        name == "string" || name == "String"
    } else {
        false
    }
}

/// Simple dependency resolution for computed columns without external dependencies
fn compute_column_evaluation_order(table_def: &TableDef) -> Result<Vec<String>, SemanticError> {
    use std::collections::{HashMap, HashSet, VecDeque};

    let mut dependencies: HashMap<String, HashSet<String>> = HashMap::new();
    let mut all_columns: HashSet<String> = HashSet::new();

    // Collect all column names
    for column in &table_def.columns {
        all_columns.insert(column.name.clone());
    }

    // Extract dependencies for each computed column
    for column in &table_def.columns {
        if column.is_computed {
            if let Some(expr) = &column.computed_expression {
                let deps = extract_column_references(expr, &all_columns);
                dependencies.insert(column.name.clone(), deps);
            }
        }
    }

    // Perform topological sort using Kahn's algorithm
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

    // Initialize for all computed columns
    for column_name in dependencies.keys() {
        in_degree.insert(column_name.clone(), 0);
        adjacency.insert(column_name.clone(), Vec::new());
    }

    // Build adjacency list and calculate in-degrees
    for (column, deps) in &dependencies {
        for dependency in deps {
            // Only consider dependencies on other computed columns
            if dependencies.contains_key(dependency) {
                adjacency.get_mut(dependency).unwrap().push(column.clone());
                *in_degree.get_mut(column).unwrap() += 1;
            }
        }
    }

    // Kahn's algorithm
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
    if result.len() != dependencies.len() {
        return Err(SemanticError::UndefinedVariable(
            "Circular dependency detected in computed columns".to_string(),
        ));
    }

    Ok(result)
}

/// Extract column references from an expression
fn extract_column_references(
    expr: &Expression,
    column_names: &std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    let mut references = std::collections::HashSet::new();
    extract_column_references_recursive(expr, &mut references, column_names);
    references
}

/// Recursively extract column references from an expression
fn extract_column_references_recursive(
    expr: &Expression,
    references: &mut std::collections::HashSet<String>,
    column_names: &std::collections::HashSet<String>,
) {
    match expr {
        Expression::Identifier(name) => {
            // Only treat identifiers as column references if they exist in the table's columns
            if column_names.contains(name) {
                references.insert(name.clone());
            }
        }
        Expression::BinaryOp { left, right, .. } => {
            extract_column_references_recursive(left, references, column_names);
            extract_column_references_recursive(right, references, column_names);
        }
        Expression::UnaryOp { operand, .. } => {
            extract_column_references_recursive(operand, references, column_names);
        }
        Expression::Call { arguments, .. } => {
            // For function calls, only recurse into arguments to find column dependencies
            for arg in arguments {
                extract_column_references_recursive(&arg.value, references, column_names);
            }
        }
        Expression::FieldAccess { object, .. } => {
            extract_column_references_recursive(object, references, column_names);
        }
        Expression::Index { object, indices } => {
            extract_column_references_recursive(object, references, column_names);
            for index in indices {
                extract_column_references_recursive(index, references, column_names);
            }
        }
        Expression::If {
            condition,
            then_branch,
            else_branch,
        } => {
            extract_column_references_recursive(condition, references, column_names);
            for stmt in then_branch {
                extract_statement_column_references(stmt, references, column_names);
            }
            if let Some(else_stmts) = else_branch {
                for stmt in else_stmts {
                    extract_statement_column_references(stmt, references, column_names);
                }
            }
        }
        Expression::IfExpr {
            condition,
            then_expr,
            else_expr,
        } => {
            extract_column_references_recursive(condition, references, column_names);
            extract_column_references_recursive(then_expr, references, column_names);
            if let Some(else_branch) = else_expr {
                extract_column_references_recursive(else_branch, references, column_names);
            }
        }
        Expression::Block { statements } => {
            for stmt in statements {
                extract_statement_column_references(stmt, references, column_names);
            }
        }
        Expression::UnsafeBlock { statements } => {
            for stmt in statements {
                extract_statement_column_references(stmt, references, column_names);
            }
        }
        Expression::Tuple(exprs) => {
            for expr in exprs {
                extract_column_references_recursive(expr, references, column_names);
            }
        }
        Expression::Match { value, arms } => {
            extract_column_references_recursive(value, references, column_names);
            for arm in arms {
                extract_column_references_recursive(&arm.pattern, references, column_names);
                extract_column_references_recursive(&arm.body, references, column_names);
            }
        }
        Expression::StructLiteral {
            type_name: _,
            fields,
        } => {
            for expr in fields.values() {
                extract_column_references_recursive(expr, references, column_names);
            }
        }
        Expression::VecNew {
            length,
            fill,
            additional_dimensions,
            ..
        } => {
            if let Some(len_expr) = length {
                extract_column_references_recursive(len_expr, references, column_names);
            }
            if let Some(fill_expr) = fill {
                extract_column_references_recursive(fill_expr, references, column_names);
            }
            for expr in additional_dimensions {
                extract_column_references_recursive(expr, references, column_names);
            }
        }
        Expression::VecLiteral { elements } => {
            for expr in elements {
                extract_column_references_recursive(expr, references, column_names);
            }
        }
        Expression::Matrix { rows } => {
            for row in rows {
                for expr in row {
                    extract_column_references_recursive(expr, references, column_names);
                }
            }
        }
        Expression::Range { start, end, step } => {
            extract_column_references_recursive(start, references, column_names);
            extract_column_references_recursive(end, references, column_names);
            if let Some(step_expr) = step {
                extract_column_references_recursive(step_expr, references, column_names);
            }
        }
        Expression::Question(expr) | Expression::Unwrap(expr) => {
            extract_column_references_recursive(expr, references, column_names);
        }
        // Literals and other leaf nodes don't have dependencies
        _ => {}
    }
}

/// Extract column references from a statement
fn extract_statement_column_references(
    stmt: &Statement,
    references: &mut std::collections::HashSet<String>,
    column_names: &std::collections::HashSet<String>,
) {
    match stmt {
        Statement::VariableDecl { value, .. } => {
            extract_column_references_recursive(value, references, column_names);
        }
        Statement::ConstDecl {
            value: ConstValue::Expression(expr),
            ..
        } => {
            extract_column_references_recursive(expr, references, column_names);
        }
        Statement::Assignment { target, value, .. } => {
            extract_column_references_recursive(target, references, column_names);
            extract_column_references_recursive(value, references, column_names);
        }
        Statement::Expression(expr) => {
            extract_column_references_recursive(expr, references, column_names);
        }
        Statement::Return(Some(expr)) | Statement::Break(Some(expr)) => {
            extract_column_references_recursive(expr, references, column_names);
        }
        Statement::ForLoop { iterable, body, .. } => {
            extract_column_references_recursive(iterable, references, column_names);
            for stmt in body {
                extract_statement_column_references(stmt, references, column_names);
            }
        }
        _ => {} // Other statements don't contribute to dependencies
    }
}

/// Infer and set proper types for computed columns in a table definition
pub fn infer_computed_column_types(
    table_def: &mut TableDef,
    context: &mut SemanticContext,
) -> Result<(), SemanticError> {
    // Compute evaluation order for computed columns using dependency analysis
    let evaluation_order = compute_column_evaluation_order(table_def)?;

    // First pass: collect all regular column types
    let mut column_context = context.clone();
    for column in &table_def.columns {
        if !column.is_computed {
            column_context.define_variable(column.name.clone(), column.column_type.clone());
        }
    }

    // Second pass: add all computed columns to context with placeholder types
    // This ensures all computed columns are available for forward references
    for column in &table_def.columns {
        if column.is_computed {
            // Use existing type annotation if available, otherwise use placeholder
            let placeholder_type = if column.column_type != Type::None {
                column.column_type.clone()
            } else {
                // Placeholder type for forward references
                Type::Identifier {
                    name: "unknown".to_string(),
                    type_args: vec![],
                }
            };
            column_context.define_variable(column.name.clone(), placeholder_type);
        }
    }

    // Third pass: infer types for computed columns in dependency order
    for column_name in &evaluation_order {
        // Find the corresponding column in the table definition
        if let Some(column) = table_def
            .columns
            .iter_mut()
            .find(|col| &col.name == column_name)
        {
            if let Some(computed_expr) = &column.computed_expression {
                let inferred_type = infer_expression_type(computed_expr, &mut column_context)?;

                // Update the column type if it was Type::None
                if column.column_type == Type::None {
                    column.column_type = inferred_type.clone();
                } else if !types_compatible(&column.column_type, &inferred_type) {
                    return Err(SemanticError::TypeMismatch {
                        expected: column.column_type.clone(),
                        found: inferred_type,
                    });
                }

                // Update the context with the correctly inferred type
                column_context.define_variable(column.name.clone(), column.column_type.clone());
            } else {
                return Err(SemanticError::UndefinedVariable(format!(
                    "Computed column '{}' must have a computed expression",
                    column.name
                )));
            }
        }
    }

    // Final pass: handle any computed columns not in the evaluation order (those with no dependencies)
    for column in &mut table_def.columns {
        if column.is_computed && !evaluation_order.contains(&column.name) {
            if let Some(computed_expr) = &column.computed_expression {
                let inferred_type = infer_expression_type(computed_expr, &mut column_context)?;

                // Update the column type if it was Type::None
                if column.column_type == Type::None {
                    column.column_type = inferred_type.clone();
                } else if !types_compatible(&column.column_type, &inferred_type) {
                    return Err(SemanticError::TypeMismatch {
                        expected: column.column_type.clone(),
                        found: inferred_type,
                    });
                }

                // Update the context with the correctly inferred type
                column_context.define_variable(column.name.clone(), column.column_type.clone());
            } else {
                return Err(SemanticError::UndefinedVariable(format!(
                    "Computed column '{}' must have a computed expression",
                    column.name
                )));
            }
        }
    }

    Ok(())
}

fn validate_table_definition(
    table_def: &TableDef,
    context: &mut SemanticContext,
) -> Result<(), SemanticError> {
    let mut column_names = std::collections::HashSet::new();
    let mut primary_key_count = 0;

    for column in &table_def.columns {
        // Check for duplicate column names
        if !column_names.insert(&column.name) {
            return Err(SemanticError::UndefinedVariable(format!(
                "Duplicate column name '{}' in table '{}'",
                column.name, table_def.name
            )));
        }

        // For computed columns, infer type from expression; for regular columns, validate the specified type
        if column.is_computed {
            if let Some(computed_expr) = &column.computed_expression {
                // For computed columns, infer the type from the expression
                // We need to set up a context that includes all column names for type inference
                let mut column_context = context.clone();

                // Add all table columns (regular and computed) as variables so forward references work
                for col in &table_def.columns {
                    column_context.define_variable(col.name.clone(), col.column_type.clone());
                }

                let inferred_type = infer_expression_type(computed_expr, &mut column_context)?;

                // If the column type is Type::None, we should accept the inferred type
                // If a specific type was provided, validate compatibility
                if column.column_type != Type::None {
                    if !types_compatible(&column.column_type, &inferred_type) {
                        return Err(SemanticError::TypeMismatch {
                            expected: column.column_type.clone(),
                            found: inferred_type,
                        });
                    }
                }
                // Note: We can't modify the column type here since we have an immutable reference
                // The caller should handle updating the TableDef with inferred types
            } else {
                return Err(SemanticError::UndefinedVariable(format!(
                    "Computed column '{}' must have a computed expression",
                    column.name
                )));
            }
        } else {
            // For regular columns, validate the specified type
            validate_type(&column.column_type, context)?;
        }

        // Validate annotations
        for annotation in &column.annotations {
            match annotation.name.as_str() {
                "primary" => {
                    primary_key_count += 1;
                    if primary_key_count > 1 {
                        return Err(SemanticError::UndefinedVariable(format!(
                            "Table '{}' cannot have multiple primary keys",
                            table_def.name
                        )));
                    }
                    if !annotation.args.is_empty() {
                        return Err(SemanticError::UndefinedVariable(format!(
                            "@primary annotation should not have arguments"
                        )));
                    }
                }
                "autoincrement" | "indexed" | "nullable" => {
                    // Valid annotations, no additional validation needed for now
                }
                "size" => {
                    if annotation.args.len() != 1 {
                        return Err(SemanticError::UndefinedVariable(format!(
                            "@size annotation requires exactly one argument"
                        )));
                    }
                }
                "precision" => {
                    if annotation.args.len() != 2 {
                        return Err(SemanticError::UndefinedVariable(format!(
                            "@precision annotation requires exactly two arguments"
                        )));
                    }
                }
                _ => {
                    return Err(SemanticError::UndefinedVariable(format!(
                        "Unknown table annotation: @{}",
                        annotation.name
                    )));
                }
            }
        }

        // Validate default value type compatibility
        if let Some(default_expr) = &column.default_value {
            let default_type = infer_expression_type(default_expr, context)?;
            if !types_compatible(&column.column_type, &default_type) {
                return Err(SemanticError::TypeMismatch {
                    expected: column.column_type.clone(),
                    found: default_type,
                });
            }
        }
    }

    Ok(())
}

fn validate_type(type_def: &Type, context: &SemanticContext) -> Result<(), SemanticError> {
    match type_def {
        Type::None => Ok(()),
        Type::Identifier { name, type_args } => {
            // Check if the type exists in context or is a built-in type
            let builtin_types = [
                "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "bool",
                "String", "char",
            ];

            if builtin_types.contains(&name.as_str()) || context.types.contains_key(name) {
                // Validate type arguments if any
                for arg in type_args {
                    validate_type(arg, context)?;
                }
                Ok(())
            } else {
                Err(SemanticError::UndefinedVariable(format!(
                    "Unknown type: {}",
                    name
                )))
            }
        }
        Type::Pointer { pointee, .. } => validate_type(pointee, context),
    Type::RawPointer { pointee, .. } => validate_type(pointee, context),
        Type::Reference { inner, .. } => validate_type(inner, context),
        Type::Optional { inner } => validate_type(inner, context),
        Type::Result { inner } => validate_type(inner, context),
        Type::Tuple(types) => {
            for t in types {
                validate_type(t, context)?;
            }
            Ok(())
        }
        Type::Matrix { element_type, .. } => validate_type(element_type, context),
        Type::Function {
            parameters,
            return_type,
        } => {
            for param in parameters {
                validate_type(param, context)?;
            }
            validate_type(return_type, context)
        }
        Type::Struct { fields } => {
            for field_type in fields.values() {
                validate_type(field_type, context)?;
            }
            Ok(())
        }
        Type::Enum { variants, .. } => {
            for variant_type in variants.values() {
                if let Some(t) = variant_type {
                    validate_type(t, context)?;
                }
            }
            Ok(())
        }
        Type::Trait { methods, .. } => {
            for method_type in methods.values() {
                validate_type(method_type, context)?;
            }
            Ok(())
        }
    }
}

/// Validate a system function definition
fn validate_system_definition(
    system_def: &SystemDef,
    context: &mut SemanticContext,
) -> Result<(), SemanticError> {
    // Validate all query parameters reference valid tables
    for param in &system_def.parameters {
        match param {
            SystemParameter::Query {
                name: _,
                query_spec,
            } => {
                // Check that the table referenced in the query exists
                if !context.tables.contains_key(&query_spec.from_table) {
                    return Err(SemanticError::UndefinedVariable(format!(
                        "Table '{}' referenced in query parameter not found",
                        query_spec.from_table
                    )));
                }

                // Validate join tables exist
                for join in &query_spec.joins {
                    if !context.tables.contains_key(&join.table) {
                        return Err(SemanticError::UndefinedVariable(format!(
                            "Join table '{}' not found",
                            join.table
                        )));
                    }
                }

                // TODO: Validate field projections and where clauses
            }
            SystemParameter::Resource { resource_type, .. } => {
                // TODO: Validate resource type exists and is accessible
                let _ = resource_type; // Suppress unused warning for now
            }
            SystemParameter::Regular {
                value_type,
                default_value,
                ..
            } => {
                // Validate default value type matches parameter type if both present
                if let Some(default_expr) = default_value {
                    let default_type = infer_expression_type(default_expr, context)?;
                    if !types_compatible(value_type, &default_type) {
                        return Err(SemanticError::TypeMismatch {
                            expected: value_type.clone(),
                            found: default_type,
                        });
                    }
                }
            }
        }
    }

    Ok(())
}

/// Analyze a system function definition including its body
fn analyze_system_definition(
    system_def: &SystemDef,
    context: &mut SemanticContext,
) -> Result<(), SemanticError> {
    // First validate the definition structure
    validate_system_definition(system_def, context)?;

    // Set up parameter bindings for body analysis
    let prev_vars = context.variables.clone();
    let prev_ret = context.current_function_return_type.clone();

    // Bind system parameters as variables
    for param in &system_def.parameters {
        match param {
            SystemParameter::Query {
                name,
                query_spec: _,
            } => {
                context.define_variable(
                    name.clone(),
                    Type::Identifier {
                        name: "QueryResult".to_string(),
                        type_args: vec![],
                    },
                );
            }
            SystemParameter::Resource {
                param_type: _,
                name,
                resource_type,
                access,
            } => {
                let param_type = match access {
                    ResourceAccess::Immutable => Type::Reference {
                        is_mutable: false,
                        inner: Box::new(resource_type.clone()),
                    },
                    ResourceAccess::Mutable => Type::Reference {
                        is_mutable: true,
                        inner: Box::new(resource_type.clone()),
                    },
                    ResourceAccess::Owned => resource_type.clone(),
                };
                context.define_variable(name.clone(), param_type);
            }
            SystemParameter::Regular {
                param_type: _,
                name,
                value_type,
                ..
            } => {
                let ptype = value_type.clone();
                context.define_variable(name.clone(), ptype);
            }
        }
    }

    // Set expected return type
    context.current_function_return_type =
        Some(system_def.return_type.clone().unwrap_or(Type::None));

    // Analyze function body
    for stmt in &system_def.body {
        analyze_statement(stmt, context)?;
    }

    // Restore context
    context.variables = prev_vars;
    context.current_function_return_type = prev_ret;

    Ok(())
}
