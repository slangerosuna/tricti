use crate::ast::*;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct SemanticContext {
    pub variables: HashMap<String, Type>,
    pub functions: HashMap<String, FunctionSignature>,
    pub types: HashMap<String, Type>,
    pub current_function_return_type: Option<Type>,
    pub in_loop: bool,
    // Traits and impls
    pub traits: HashMap<String, TraitInfo>,
    pub trait_impls: HashMap<String, HashMap<String, ImplInfo>>, // trait -> (type -> impl)
    pub inherent_impls: HashMap<String, ImplInfo>,               // type -> impl
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

#[derive(Debug)]
pub enum SemanticError {
    UndefinedVariable(String),
    UndefinedFunction(String),
    TypeMismatch { expected: Type, found: Type },
    InvalidOperation { operator: String, operand_types: Vec<Type> },
    ReturnOutsideFunction,
    BreakOutsideLoop,
    ArgumentCountMismatch { expected: usize, found: usize },
    AmbiguousMethod { type_name: String, method: String, traits: Vec<String> },
    InvalidRangeStepZero,
}

impl SemanticContext {
    pub fn new() -> Self {
        let mut context = SemanticContext {
            variables: HashMap::new(),
            functions: HashMap::new(),
            types: HashMap::new(),
            current_function_return_type: None,
            in_loop: false,
            traits: HashMap::new(),
            trait_impls: HashMap::new(),
            inherent_impls: HashMap::new(),
        };
        
        // Add built-in functions
        context.functions.insert("println".to_string(), FunctionSignature {
            parameters: vec![], // Variadic, we'll handle this specially
            return_type: Type::None,
            is_async: false,
        });
        
        // Add built-in types
        context.types.insert("i32".to_string(), Type::Identifier("i32".to_string()));
        context.types.insert("i64".to_string(), Type::Identifier("i64".to_string()));
        context.types.insert("f32".to_string(), Type::Identifier("f32".to_string()));
        context.types.insert("f64".to_string(), Type::Identifier("f64".to_string()));
        context.types.insert("bool".to_string(), Type::Identifier("bool".to_string()));
        context.types.insert("string".to_string(), Type::Identifier("string".to_string()));
        
        context
    }
    
    pub fn enter_scope(&mut self) {
        // For now, we'll use a simple approach without proper scoping
        // In a full implementation, you'd want to maintain a scope stack
    }
    
    pub fn exit_scope(&mut self) {
        // For now, we'll use a simple approach without proper scoping
    }
    
    pub fn define_variable(&mut self, name: String, var_type: Type) {
        self.variables.insert(name, var_type);
    }
    
    pub fn get_variable_type(&self, name: &str) -> Option<&Type> {
        self.variables.get(name)
    }
    
    pub fn define_function(&mut self, name: String, signature: FunctionSignature) {
        self.functions.insert(name, signature);
    }
    
    pub fn get_function_signature(&self, name: &str) -> Option<&FunctionSignature> {
        self.functions.get(name)
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
        analyze_statement(statement, &mut context)?;
    }
    
    Ok(context)
}

fn collect_definitions(statement: &Statement, context: &mut SemanticContext) -> Result<(), SemanticError> {
    match statement {
        Statement::ConstDecl { name, value, .. } => {
            match value {
                ConstValue::Type(type_def) => {
                    context.types.insert(name.clone(), type_def.clone());
                    // If this is a trait type definition, register trait info too
                    if let Type::Trait { associated_types, methods } = type_def {
                        let ti = TraitInfo { associated_types: associated_types.clone(), methods: methods.clone() };
                        context.traits.insert(name.clone(), ti);
                    }
                }
                ConstValue::Expression(expr) => {
                    // If expression is a function, pre-declare its signature in the function table and as a variable value of function type
                    if let Expression::Function { parameters, return_type, .. } = expr {
                        let sig = FunctionSignature {
                            parameters: parameters.iter().map(|p| p.param_type.clone().unwrap_or(Type::Identifier("i64".to_string()))).collect(),
                            return_type: return_type.clone().unwrap_or(Type::Identifier("i64".to_string())),
                            is_async: false,
                        };
                        context.define_function(name.clone(), sig.clone());
                        context.define_variable(name.clone(), Type::Function { parameters: sig.parameters.clone(), return_type: Box::new(sig.return_type.clone()) });
                    } else {
                        // For other expression constants, infer the type during analysis
                        let expr_type = infer_expression_type(expr, context)?;
                        context.define_variable(name.clone(), expr_type);
                    }
                }
            }
        }
        Statement::ImplBlock { trait_name, type_name, methods } => {
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
                if let Statement::ConstDecl { name: mname, value, .. } = m {
                    match value {
                        ConstValue::Expression(Expression::Function { parameters, return_type, .. }) => {
                            // Prepend implicit receiver parameter of type &type_name for inherent methods
                            let mut params: Vec<Type> = Vec::new();
                            if trait_name.is_none() {
                                params.push(Type::Pointer { is_mutable: false, pointee: Box::new(Type::Identifier(type_name.clone())) });
                            }
                            params.extend(parameters.iter().map(|p| p.param_type.clone().unwrap_or(Type::Identifier("i64".to_string()))));
                            let sig = FunctionSignature {
                                parameters: params,
                                return_type: return_type.clone().unwrap_or(Type::Identifier("i64".to_string())),
                                is_async: false,
                            };
                            let mangled = mangle_method_name(trait_name.as_deref(), type_name, mname);
                            context.define_function(mangled, sig);
                        }
                        ConstValue::Type(_) => {
                            // associated type binding; collected during analysis phase below
                        }
                        ConstValue::Expression(_) => { /* ignore non-function expressions in impl header pass */ }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn analyze_statement(statement: &Statement, context: &mut SemanticContext) -> Result<(), SemanticError> {
    match statement {
        Statement::ConstDecl { name: _, type_annotation: _, value } => {
            if let ConstValue::Expression(Expression::Function { parameters, return_type, body, .. }) = value {
                // Analyze function body with parameter bindings and expected return type
                let prev_vars = context.variables.clone();
                let prev_ret = context.current_function_return_type.clone();
                // Bind parameters
                for p in parameters {
                    let ty = p.param_type.clone().unwrap_or(Type::Identifier("i64".into()));
                    context.define_variable(p.name.clone(), ty);
                }
                // Set expected return
                context.current_function_return_type = Some(return_type.clone().unwrap_or(Type::None));
                match body {
                    FunctionBody::Block(stmts) => {
                        for s in stmts { analyze_statement(s, context)?; }
                    }
                    FunctionBody::Expression(expr) => {
                        // If body is a block expression, descend into its statements to analyze side-effects and calls
                        if let Expression::Block { statements } = expr.as_ref() {
                            for s in statements { analyze_statement(s, context)?; }
                        } else {
                            let _ = infer_expression_type(expr, context)?;
                        }
                    }
                }
                // Restore
                context.variables = prev_vars;
                context.current_function_return_type = prev_ret;
            }
        }
        Statement::VariableDecl { name, type_annotation, value } => {
            let value_type = infer_expression_type(value, context)?;
            
            let final_type = if let Some(annotation) = type_annotation {
                if !types_compatible(annotation, &value_type) {
                    return Err(SemanticError::TypeMismatch {
                        expected: annotation.clone(),
                        found: value_type,
                    });
                }
                annotation.clone()
            } else {
                value_type
            };
            
            context.define_variable(name.clone(), final_type);
        }
        
        Statement::Assignment { target, value, .. } => {
            let target_type = infer_expression_type(target, context)?;
            let value_type = infer_expression_type(value, context)?;
            
            if !types_compatible(&target_type, &value_type) {
                return Err(SemanticError::TypeMismatch {
                    expected: target_type,
                    found: value_type,
                });
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
            if !context.in_loop {
                return Err(SemanticError::BreakOutsideLoop);
            }
        }
        
        Statement::ForLoop { variable, type_annotation, iterable, body } => {
            let _iterable_type = infer_expression_type(iterable, context)?;
            
            // Assume the variable type is i64 if not specified (matches codegen loop counter)
            let var_type = type_annotation.clone().unwrap_or(Type::Identifier("i64".to_string()));
            
            context.enter_scope();
            context.define_variable(variable.clone(), var_type);
            context.in_loop = true;
            
            for stmt in body {
                analyze_statement(stmt, context)?;
            }
            
            context.in_loop = false;
            context.exit_scope();
        }
        
        Statement::ImplBlock { trait_name, type_name, methods } => {
            // Validate impls and collect associated types + methods into impl registries
            if let Some(tn) = trait_name {
                // Trait impl
                let trait_info = context.traits.get(tn).ok_or_else(|| SemanticError::UndefinedVariable(format!("trait {} not defined", tn)))?.clone();
                let impls_for_trait = context.trait_impls.get_mut(tn).and_then(|m| m.get_mut(type_name));
                let mut info = impls_for_trait.cloned().unwrap_or_default();

                // Gather provided items
                let mut provided_methods: HashMap<String, FunctionSignature> = HashMap::new();
                let mut provided_assoc: HashMap<String, Type> = HashMap::new();
                for item in methods {
                    if let Statement::ConstDecl { name, value, .. } = item {
                        match value {
                            ConstValue::Type(ty) => {
                                provided_assoc.insert(name.clone(), ty.clone());
                            }
                            ConstValue::Expression(Expression::Function { parameters, return_type, .. }) => {
                                let sig = FunctionSignature {
                                    parameters: parameters.iter().map(|p| p.param_type.clone().unwrap_or(Type::Identifier("i64".to_string()))).collect(),
                                    return_type: return_type.clone().unwrap_or(Type::Identifier("i64".to_string())),
                                    is_async: false,
                                };
                                provided_methods.insert(name.clone(), sig);
                            }
                            ConstValue::Expression(_) => { /* ignore other expressions in impl items */ }
                        }
                    }
                }

                // Check associated types coverage
                for assoc in &trait_info.associated_types {
                    if !provided_assoc.contains_key(assoc) {
                        return Err(SemanticError::UndefinedVariable(format!("impl {} for {} missing associated type {}", tn, type_name, assoc)));
                    }
                }

                // Check methods coverage and signatures compatibility
                for (mname, mty) in &trait_info.methods {
                    // method type is a function type
                    let Type::Function { parameters, return_type } = mty else { continue };
                    let Some(impl_sig) = provided_methods.get(mname) else {
                        return Err(SemanticError::UndefinedFunction(format!("impl {} for {} missing method {}", tn, type_name, mname)));
                    };
                    // Compare param lengths
                    if parameters.len() != impl_sig.parameters.len() {
                        return Err(SemanticError::ArgumentCountMismatch { expected: parameters.len(), found: impl_sig.parameters.len() });
                    }
                    // Check params/return with associated type substitution
                    for (a, b) in parameters.iter().zip(&impl_sig.parameters) {
                        if !types_match_with_assoc_and_self(a, b, &provided_assoc, type_name) {
                            return Err(SemanticError::TypeMismatch { expected: a.clone(), found: b.clone() });
                        }
                    }
                    if !types_match_with_assoc_and_self(return_type, &impl_sig.return_type, &provided_assoc, type_name) {
                        return Err(SemanticError::TypeMismatch { expected: (*return_type.clone()).clone(), found: impl_sig.return_type.clone() });
                    }
                }

                info.associated_types = provided_assoc;
                info.methods = provided_methods;
                context.trait_impls.entry(tn.clone()).or_default().insert(type_name.clone(), info);
            } else {
                // Inherent impl: accept all method function consts
                let mut info = context.inherent_impls.remove(type_name).unwrap_or_default();
                for item in methods {
                    if let Statement::ConstDecl { name, value, .. } = item {
                        if let ConstValue::Expression(Expression::Function { parameters, return_type, .. }) = value {
                            let sig = FunctionSignature {
                                parameters: parameters.iter().map(|p| p.param_type.clone().unwrap_or(Type::Identifier("i64".to_string()))).collect(),
                                return_type: return_type.clone().unwrap_or(Type::Identifier("i64".to_string())),
                                is_async: false,
                            };
                            info.methods.insert(name.clone(), sig);
                        }
                    }
                }
                context.inherent_impls.insert(type_name.clone(), info);
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

fn types_match_with_assoc_and_self(expected: &Type, found: &Type, assoc: &HashMap<String, Type>, self_type: &str) -> bool {
    fn subst(t: &Type, assoc: &HashMap<String, Type>, self_type: &str) -> Type {
        match t {
            Type::Identifier(a) if a == "self" => Type::Identifier(self_type.to_string()),
            Type::Identifier(a) if assoc.contains_key(a) => assoc.get(a).unwrap().clone(),
            Type::Pointer { is_mutable, pointee } => Type::Pointer { is_mutable: *is_mutable, pointee: Box::new(subst(pointee, assoc, self_type)) },
            Type::RawPointer { pointee } => Type::RawPointer { pointee: Box::new(subst(pointee, assoc, self_type)) },
            Type::Optional { inner } => Type::Optional { inner: Box::new(subst(inner, assoc, self_type)) },
            Type::Result { inner } => Type::Result { inner: Box::new(subst(inner, assoc, self_type)) },
            Type::Function { parameters, return_type } => Type::Function { parameters: parameters.iter().map(|p| subst(p, assoc, self_type)).collect(), return_type: Box::new(subst(return_type, assoc, self_type)) },
            other => other.clone(),
        }
    }

    let e = subst(expected, assoc, self_type);
    let f = subst(found, assoc, self_type);
    e == f
}

fn infer_expression_type(expr: &Expression, context: &SemanticContext) -> Result<Type, SemanticError> {
    match expr {
        Expression::Literal(literal) => {
            Ok(match literal {
                Literal::Integer(_) => Type::Identifier("i64".to_string()),
                Literal::Float(_) => Type::Identifier("f64".to_string()),
                Literal::String(_) => Type::Identifier("string".to_string()),
                Literal::Boolean(_) => Type::Identifier("bool".to_string()),
            })
        }
        
        Expression::Identifier(name) => {
            // Prefer variables; if not found, allow referencing functions as first-class values
            if let Some(v) = context.get_variable_type(name) {
                Ok(v.clone())
            } else if let Some(sig) = context.get_function_signature(name) {
                Ok(Type::Function { parameters: sig.parameters.clone(), return_type: Box::new(sig.return_type.clone()) })
            } else {
                Err(SemanticError::UndefinedVariable(name.clone()))
            }
        }
        
        Expression::BinaryOp { left, operator, right } => {
            let left_type = infer_expression_type(left, context)?;
            let right_type = infer_expression_type(right, context)?;
            
            infer_binary_op_type(&left_type, operator, &right_type)
        }
        
        Expression::UnaryOp { operator, operand } => {
            let operand_type = infer_expression_type(operand, context)?;
            infer_unary_op_type(operator, &operand_type)
        }
        
        Expression::Call { function, arguments } => {
            match function.as_ref() {
                Expression::Identifier(name) => {
                    if name == "println" {
                        // Special handling for println - it's variadic, but still analyze args
                        for arg in arguments { let _ = infer_expression_type(&arg.value, context)?; }
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
                                if let Some(impls_for_trait) = context.trait_impls.get(trait_name) {
                                    if let Some(info) = impls_for_trait.get(&type_name) {
                                        if let Some(sig) = info.methods.get(method_name) {
                                            // Validate arg count and types
                                            if arguments.len() != sig.parameters.len() {
                                                return Err(SemanticError::ArgumentCountMismatch { expected: sig.parameters.len(), found: arguments.len() });
                                            }
                                            for (arg, expected_type) in arguments.iter().zip(&sig.parameters) {
                                                let arg_ty = infer_expression_type(&arg.value, context)?;
                                                if !types_compatible(expected_type, &arg_ty) {
                                                    return Err(SemanticError::TypeMismatch { expected: expected_type.clone(), found: arg_ty });
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

                    let signature = context.get_function_signature(name)
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
                    if let Type::Function { parameters, return_type } = recv_expr_ty.clone() {
                        if field == "bind" {
                            // Binding N arguments yields a function expecting the remaining parameters (best-effort typing)
                            let bound_n = arguments.len();
                            let remaining = if bound_n >= parameters.len() { Vec::new() } else { parameters[bound_n..].to_vec() };
                            return Ok(Type::Function { parameters: remaining, return_type });
                        }
                    }
                    let base_ty_name = peel_to_identifier_name(&recv_expr_ty);
                    if let Some(type_name) = base_ty_name {
                        // Inherent impl first
                        if let Some(inh) = context.inherent_impls.get(&type_name) {
                            if let Some(sig) = inh.methods.get(field) {
                                // Expect signature includes receiver as first param; args must match remaining
                                if sig.parameters.len() == 0 || arguments.len() != sig.parameters.len() - 1 {
                                    return Err(SemanticError::ArgumentCountMismatch { expected: sig.parameters.len() - 1, found: arguments.len() });
                                }
                                for (arg, expected_type) in arguments.iter().zip(sig.parameters.iter().skip(1)) {
                                    let arg_ty = infer_expression_type(&arg.value, context)?;
                                    if !types_compatible(expected_type, &arg_ty) {
                                        return Err(SemanticError::TypeMismatch { expected: expected_type.clone(), found: arg_ty });
                                    }
                                }
                                return Ok(sig.return_type.clone());
                            }
                        }

                        // Otherwise, search trait impls for this type
                        let mut candidates: Vec<(&String, &FunctionSignature)> = Vec::new();
                        for (trait_name, impls_for_trait) in &context.trait_impls {
                            if let Some(info) = impls_for_trait.get(&type_name) {
                                if let Some(sig) = info.methods.get(field) {
                                    candidates.push((trait_name, sig));
                                }
                            }
                        }
                        if candidates.len() > 1 {
                            let trait_list: Vec<String> = candidates.into_iter().map(|(tn, _)| tn.clone()).collect();
                            return Err(SemanticError::AmbiguousMethod { type_name, method: field.clone(), traits: trait_list });
                        } else if let Some((_tn, sig)) = candidates.into_iter().next() {
                            if sig.parameters.len() == 0 || arguments.len() != sig.parameters.len() - 1 {
                                return Err(SemanticError::ArgumentCountMismatch { expected: sig.parameters.len() - 1, found: arguments.len() });
                            }
                            for (arg, expected_type) in arguments.iter().zip(sig.parameters.iter().skip(1)) {
                                let arg_ty = infer_expression_type(&arg.value, context)?;
                                if !types_compatible(expected_type, &arg_ty) {
                                    return Err(SemanticError::TypeMismatch { expected: expected_type.clone(), found: arg_ty });
                                }
                            }
                            return Ok(sig.return_type.clone());
                        }
                    }
                    // Fallback: unknown method; treat as none
                    Ok(Type::None)
                }
                _ => {
                    // Function expressions not yet supported
                    Ok(Type::None)
                }
            }
        }
        
    Expression::FieldAccess { object, field } => {
            let object_type = infer_expression_type(object, context)?;
            // Resolve through pointers
            let mut base_ty = object_type.clone();
            if let Type::Pointer { pointee, .. } | Type::RawPointer { pointee } = &object_type {
                base_ty = (*pointee.clone()).clone();
            }
            // If base is an identifier type referring to a struct, pick field type
            if let Type::Identifier(ref name) = base_ty {
                if let Some(Type::Struct { fields }) = context.types.get(name) {
                    if let Some(fty) = fields.get(field) {
                        return Ok(fty.clone());
                    }
                }
            }
            // Fallback
            Ok(Type::Identifier("i64".to_string()))
        }
        
        Expression::Index { object, indices: _ } => {
            let object_type = infer_expression_type(object, context)?;
            match object_type {
                Type::Matrix { element_type, dimensions: _ } => {
                    // If full indexing provided (indices length equals dimensions), return element type
                    // Also allow 1D indexing into 1D matrix (vector)
                    // For now, we don't validate index types rigorously
                    Ok((*element_type).clone())
                }
                _ => Ok(Type::None),
            }
        }
        
        Expression::If { condition, then_branch: _, else_branch: _ } => {
            let condition_type = infer_expression_type(condition, context)?;
            // Accept common truthy types (bool, numeric, string, pointers)
            let is_bool = types_compatible(&Type::Identifier("bool".to_string()), &condition_type);
            let is_num = is_numeric_type(&condition_type);
            let is_str = matches!(condition_type, Type::Identifier(ref s) if s == "string");
            let is_ptr = matches!(condition_type, Type::Pointer { .. } | Type::RawPointer { .. });
            if !(is_bool || is_num || is_str || is_ptr) {
                return Err(SemanticError::TypeMismatch {
                    expected: Type::Identifier("bool".to_string()),
                    found: condition_type,
                });
            }

            // For now, assume if expressions return none
            Ok(Type::None)
        }
        
        Expression::Block { statements: _ } => {
            // For now, assume blocks return none
            Ok(Type::None)
        }
        
        Expression::Range { start, end, step } => {
            let st = infer_expression_type(start, context)?;
            let et = infer_expression_type(end, context)?;
            if !(is_numeric_type(&st) && is_numeric_type(&et)) {
                return Err(SemanticError::TypeMismatch { expected: Type::Identifier("i64".to_string()), found: st });
            }
            if let Some(s) = step {
                let s_ty = infer_expression_type(s, context)?;
                if !is_numeric_type(&s_ty) {
                    return Err(SemanticError::TypeMismatch { expected: Type::Identifier("i64".to_string()), found: s_ty });
                }
                // If step is a literal zero, reject.
                if let Expression::Literal(Literal::Integer(ival)) = s.as_ref() {
                    if *ival == 0 { return Err(SemanticError::InvalidRangeStepZero); }
                }
            }
            Ok(Type::Identifier("i64".to_string()))
        }
        Expression::Matrix { rows } => {
            // Determine dimensions and element type (numeric best-effort)
            let row_count = rows.len();
            let col_count = if row_count > 0 { rows[0].len() } else { 0 };
            // validate equal columns
            for r in rows { if r.len() != col_count { /* ignore mismatch for now */ } }
            // infer element type by scanning; prefer f64 if any float present, else i64
            let mut has_float = false;
            for r in rows {
                for e in r {
                    if let Ok(t) = infer_expression_type(e, context) {
                        if matches!(t, Type::Identifier(ref s) if s == "f32" || s == "f64") { has_float = true; }
                    }
                }
            }
            let elem = if has_float { Type::Identifier("f64".to_string()) } else { Type::Identifier("i64".to_string()) };
            let dims = if row_count <= 1 { vec![col_count] } else { vec![row_count, col_count] };
            Ok(Type::Matrix { element_type: Box::new(elem), dimensions: dims })
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
            Type::RawPointer { pointee } => cur = pointee.as_ref(),
            Type::Optional { inner } => cur = inner.as_ref(),
            Type::Result { inner } => cur = inner.as_ref(),
            Type::Identifier(name) => return Some(name.clone()),
            _ => return None,
        }
    }
}

fn infer_binary_op_type(left: &Type, operator: &BinaryOperator, right: &Type) -> Result<Type, SemanticError> {
    match operator {
        BinaryOperator::Add | BinaryOperator::Sub | BinaryOperator::Mul | BinaryOperator::Div | BinaryOperator::Mod => {
            if types_compatible(left, right) && is_numeric_type(left) {
                Ok(left.clone())
            } else {
                Err(SemanticError::InvalidOperation {
                    operator: format!("{:?}", operator),
                    operand_types: vec![left.clone(), right.clone()],
                })
            }
        }
        
        BinaryOperator::Equal | BinaryOperator::NotEqual | 
        BinaryOperator::Less | BinaryOperator::Greater | 
        BinaryOperator::LessEqual | BinaryOperator::GreaterEqual => {
            if types_compatible(left, right) {
                Ok(Type::Identifier("bool".to_string()))
            } else {
                Err(SemanticError::InvalidOperation {
                    operator: format!("{:?}", operator),
                    operand_types: vec![left.clone(), right.clone()],
                })
            }
        }
        
        BinaryOperator::And | BinaryOperator::Or | BinaryOperator::Xor => {
            let bool_type = Type::Identifier("bool".to_string());
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
            let bool_type = Type::Identifier("bool".to_string());
            if types_compatible(operand, &bool_type) {
                Ok(bool_type)
            } else {
                Err(SemanticError::InvalidOperation {
                    operator: format!("{:?}", operator),
                    operand_types: vec![operand.clone()],
                })
            }
        }
        
        UnaryOperator::AddressOf => {
            Ok(Type::Pointer {
                is_mutable: false,
                pointee: Box::new(operand.clone()),
            })
        }
        
        UnaryOperator::MutAddressOf => {
            Ok(Type::Pointer {
                is_mutable: true,
                pointee: Box::new(operand.clone()),
            })
        }
        
        UnaryOperator::Deref => {
            match operand {
                Type::Pointer { pointee, .. } | Type::RawPointer { pointee } => {
                    Ok(pointee.as_ref().clone())
                }
                _ => Err(SemanticError::InvalidOperation {
                    operator: format!("{:?}", operator),
                    operand_types: vec![operand.clone()],
                })
            }
        }
        
        _ => Ok(operand.clone()),
    }
}

fn types_compatible(expected: &Type, found: &Type) -> bool {
    match (expected, found) {
        // Allow implicit address-of: passing T where &T is expected
        (
            Type::Pointer { pointee: exp_pointee, .. },
            Type::Identifier(found_name),
        ) => {
            if let Type::Identifier(exp_name) = exp_pointee.as_ref() {
                return exp_name == found_name;
            }
            false
        }
        (Type::Identifier(a), Type::Identifier(b)) => {
            if a == b { return true; }
            // Allow any pair of numeric scalar types to be used together; codegen will unify bit-widths.
            let na = a.as_str();
            let nb = b.as_str();
            let is_num_a = matches!(na, "i32"|"i64"|"u16"|"u32"|"u64"|"f32"|"f64");
            let is_num_b = matches!(nb, "i32"|"i64"|"u16"|"u32"|"u64"|"f32"|"f64");
            if is_num_a && is_num_b { return true; }
            false
        }
        (Type::None, _) | (_, Type::None) => true,
        _ => expected == found,
    }
}

fn is_numeric_type(t: &Type) -> bool {
    match t {
        Type::Identifier(name) => {
            matches!(name.as_str(), "i32" | "i64" | "f32" | "f64" | "u16" | "u32" | "u64")
        }
        _ => false,
    }
}
