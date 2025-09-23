use crate::ast::*;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{InitializationConfig, Target};
use inkwell::types::{BasicTypeEnum, IntType, FloatType, StructType};
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue, BasicMetadataValueEnum};
use inkwell::{AddressSpace, IntPredicate, FloatPredicate};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum CodegenError {
    UndefinedVariable(String),
    UndefinedFunction(String),
    TypeConversionError(String),
    InvalidOperation(String),
    CompilationError(String),
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CodegenError::UndefinedVariable(name) => write!(f, "Undefined variable: {}", name),
            CodegenError::UndefinedFunction(name) => write!(f, "Undefined function: {}", name),
            CodegenError::TypeConversionError(msg) => write!(f, "Type conversion error: {}", msg),
            CodegenError::InvalidOperation(msg) => write!(f, "Invalid operation: {}", msg),
            CodegenError::CompilationError(msg) => write!(f, "Compilation error: {}", msg),
        }
    }
}

impl Error for CodegenError {}

pub struct CodeGenerator<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    current_function: Option<FunctionValue<'ctx>>,
    semantic: crate::semantic::SemanticContext,
    struct_types: HashMap<String, (StructType<'ctx>, Vec<String>)>, // name -> (llvm struct, field order)
    current_impl_struct: Option<String>,
}

impl<'ctx> CodeGenerator<'ctx> {
    pub fn new(context: &'ctx Context, semantic_context: crate::semantic::SemanticContext) -> Result<Self, CodegenError> {
        let module = context.create_module("main");
        
        let mut generator = CodeGenerator {
            context,
            module,
            builder: context.create_builder(),
            variables: HashMap::new(),
            functions: HashMap::new(),
            current_function: None,
            semantic: semantic_context,
            struct_types: HashMap::new(),
            current_impl_struct: None,
        };
        
        generator.declare_external_functions()?;
        generator.build_struct_types()?;
        
        Ok(generator)
    }

    #[allow(dead_code)]
    fn get_int_type_of(&self, v: BasicValueEnum<'ctx>) -> Option<IntType<'ctx>> {
        if v.is_int_value() { Some(v.into_int_value().get_type()) } else { None }
    }

    // Resolve the semantic struct name for a variable (peels pointers/optionals/results)
    fn semantic_struct_name_of_var(&self, var_name: &str) -> Option<String> {
        use crate::ast::Type as AstType;
        fn peel<'a>(t: &'a AstType) -> &'a AstType {
            match t {
                AstType::Pointer { pointee, .. } => peel(pointee),
                AstType::RawPointer { pointee } => peel(pointee),
                AstType::Optional { inner } => peel(inner),
                AstType::Result { inner } => peel(inner),
                other => other,
            }
        }
        if let Some(t) = self.semantic.get_variable_type(var_name) {
            match peel(t) {
                AstType::Identifier(name) => return Some(name.clone()),
                _ => {}
            }
        }
        // Fallback: infer from current function's local variable LLVM type
        if let Some((_, bte)) = self.variables.get(var_name) {
            if let BasicTypeEnum::StructType(st) = bte {
                if let Some((name, _)) = self.struct_types.iter().find(|(_, (llvm_st, _))| llvm_st == st) {
                    return Some(name.clone());
                }
            }
        }
        None
    }

    fn unify_ints(
        &self,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
    ) -> Result<(inkwell::values::IntValue<'ctx>, inkwell::values::IntValue<'ctx>, IntType<'ctx>), CodegenError> {
        let l = left.into_int_value();
        let r = right.into_int_value();
        let lt = l.get_type();
        let rt = r.get_type();
        let lbw = lt.get_bit_width();
        let rbw = rt.get_bit_width();
        if lbw == rbw { return Ok((l, r, lt)); }
        let (dst_bw, dst_ty) = if lbw > rbw { (lbw, lt) } else { (rbw, rt) };
        let l_cast = if lbw == dst_bw { l } else if lbw == 1 { self.builder.build_int_z_extend(l, dst_ty, "zext").map_err(|e| CodegenError::CompilationError(e.to_string()))? } else { self.builder.build_int_s_extend(l, dst_ty, "sext").map_err(|e| CodegenError::CompilationError(e.to_string()))? };
        let r_cast = if rbw == dst_bw { r } else if rbw == 1 { self.builder.build_int_z_extend(r, dst_ty, "zext").map_err(|e| CodegenError::CompilationError(e.to_string()))? } else { self.builder.build_int_s_extend(r, dst_ty, "sext").map_err(|e| CodegenError::CompilationError(e.to_string()))? };
        Ok((l_cast, r_cast, dst_ty))
    }

    // Map our AST Type or identifier name to an LLVM BasicTypeEnum (subset only)
    fn map_identifier_type(&self, name: &str) -> Option<BasicTypeEnum<'ctx>> {
        if let Some((st, _)) = self.struct_types.get(name) {
            return Some((*st).into());
        }
        match name {
            // signed ints
            "i32" => Some(self.context.i32_type().into()),
            "i64" => Some(self.context.i64_type().into()),
            // unsigned ints (use same bit width int; signedness matters only in ops)
            "u16" => Some(self.context.i16_type().into()),
            "u32" => Some(self.context.i32_type().into()),
            "u64" => Some(self.context.i64_type().into()),
            // bool
            "bool" => Some(self.context.bool_type().into()),
            // string as i8*
            "str" | "string" | "String" => Some(self.context.ptr_type(AddressSpace::default()).into()),
            _ => None,
        }
    }

    fn map_ast_type(&self, ty: &Type) -> Option<BasicTypeEnum<'ctx>> {
        match ty {
            Type::Identifier(name) => self.map_identifier_type(name),
            Type::Pointer { .. } | Type::RawPointer { .. } => Some(self.context.ptr_type(AddressSpace::default()).into()),
            Type::Optional { inner } => self.map_ast_type(inner),
            Type::Result { inner } => self.map_ast_type(inner),
            // Struct literal type appears in type expressions; prefer named identifier usage elsewhere
            Type::Struct { .. } => None,
            // Not yet supported; default None so callers can fall back
            _ => None,
        }
    }

    fn build_struct_types(&mut self) -> Result<(), CodegenError> {
        // First pass: create opaque named structs for all struct types in semantics
        for (name, ty) in &self.semantic.types {
            if let Type::Struct { .. } = ty {
                let st = self.context.opaque_struct_type(name);
                self.struct_types.insert(name.clone(), (st, Vec::new()));
            }
        }
        // Second pass: set bodies based on fields (sorted by name for stability)
        let keys: Vec<String> = self.struct_types.keys().cloned().collect();
        for name in keys {
            if let Some(Type::Struct { fields }) = self.semantic.types.get(&name) {
                let mut field_names: Vec<String> = fields.keys().cloned().collect();
                field_names.sort();
                let mut field_types: Vec<BasicTypeEnum<'ctx>> = Vec::new();
                for fname in &field_names {
                    if let Some(f_ty) = fields.get(fname).and_then(|t| self.map_ast_type(t)) {
                        field_types.push(f_ty);
                    } else {
                        // default to i64 when unknown
                        field_types.push(self.context.i64_type().into());
                    }
                }
                if let Some((st, _)) = self.struct_types.get_mut(&name) {
                    st.set_body(&field_types, false);
                    *self.struct_types.get_mut(&name).unwrap() = (*st, field_names);
                }
            }
        }
        Ok(())
    }

    fn cast_basic_to_type(&self, value: BasicValueEnum<'ctx>, dest_ty: BasicTypeEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match dest_ty {
            BasicTypeEnum::IntType(int_ty) => self.cast_to_int(value, int_ty).map(|v| v.into()),
            BasicTypeEnum::FloatType(float_ty) => self.cast_to_float(value, float_ty).map(|v| v.into()),
            BasicTypeEnum::PointerType(ptr_ty) => self.cast_to_ptr(value, ptr_ty).map(|v| v.into()),
            _ => Ok(value),
        }
    }

    fn cast_to_int(&self, value: BasicValueEnum<'ctx>, int_ty: IntType<'ctx>) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        if value.is_int_value() {
            let iv = value.into_int_value();
            let src_bw = iv.get_type().get_bit_width();
            let dst_bw = int_ty.get_bit_width();
            if src_bw == dst_bw { Ok(iv) }
            else if src_bw < dst_bw {
                // zero extend for 1-bit (bool); sign-extend otherwise as a heuristic
                if src_bw == 1 {
                    self.builder.build_int_z_extend(iv, int_ty, "zext").map_err(|e| CodegenError::CompilationError(e.to_string()))
                } else {
                    self.builder.build_int_s_extend(iv, int_ty, "sext").map_err(|e| CodegenError::CompilationError(e.to_string()))
                }
            } else {
                self.builder.build_int_truncate(iv, int_ty, "trunc").map_err(|e| CodegenError::CompilationError(e.to_string()))
            }
        } else if value.is_float_value() {
            self.builder.build_float_to_signed_int(value.into_float_value(), int_ty, "ftosi").map_err(|e| CodegenError::CompilationError(e.to_string()))
        } else if value.is_pointer_value() {
            self.builder.build_ptr_to_int(value.into_pointer_value(), int_ty, "ptoi").map_err(|e| CodegenError::CompilationError(e.to_string()))
        } else {
            Ok(int_ty.const_zero())
        }
    }

    fn cast_to_float(&self, value: BasicValueEnum<'ctx>, float_ty: FloatType<'ctx>) -> Result<inkwell::values::FloatValue<'ctx>, CodegenError> {
        if value.is_float_value() {
            let fv = value.into_float_value();
            // If types differ (f32 vs f64), convert
            if fv.get_type() == float_ty { Ok(fv) } else { self.builder.build_float_cast(fv, float_ty, "fcast").map_err(|e| CodegenError::CompilationError(e.to_string())) }
        } else if value.is_int_value() {
            self.builder.build_signed_int_to_float(value.into_int_value(), float_ty, "sitofp").map_err(|e| CodegenError::CompilationError(e.to_string()))
        } else {
            Ok(float_ty.const_zero())
        }
    }

    fn cast_to_ptr(&self, value: BasicValueEnum<'ctx>, ptr_ty: inkwell::types::PointerType<'ctx>) -> Result<inkwell::values::PointerValue<'ctx>, CodegenError> {
        if value.is_pointer_value() {
            let pv = value.into_pointer_value();
            if pv.get_type() == ptr_ty { Ok(pv) } else { Ok(self.builder.build_pointer_cast(pv, ptr_ty, "pcast").map_err(|e| CodegenError::CompilationError(e.to_string()))?) }
        } else if value.is_int_value() {
            self.builder.build_int_to_ptr(value.into_int_value(), ptr_ty, "itop").map_err(|e| CodegenError::CompilationError(e.to_string()))
        } else {
            // Allocate a null pointer
            Ok(ptr_ty.const_null())
        }
    }
    
    fn declare_external_functions(&mut self) -> Result<(), CodegenError> {
        // Declare printf function
        let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        
        let printf_type = i32_type.fn_type(&[i8_ptr_type.into()], true);
        let printf_fn = self.module.add_function("printf", printf_type, None);
        self.functions.insert("printf".to_string(), printf_fn);
        
        // Declare puts function for simple string printing
        let puts_type = i32_type.fn_type(&[i8_ptr_type.into()], false);
        let puts_fn = self.module.add_function("puts", puts_type, None);
        self.functions.insert("puts".to_string(), puts_fn);
        
        Ok(())
    }
    
    pub fn generate_program(&mut self, program: &Program) -> Result<(), CodegenError> {
    // First declare and define user functions from const decls
    self.declare_and_define_functions(program)?;

        // Look for a top-level function named `main` and emit its body.
        let mut main_body: Option<&Vec<Statement>> = None;
        for stmt in &program.statements {
            if let Statement::ConstDecl { name, value, .. } = stmt {
                if name == "main" {
                    if let ConstValue::Expression(Expression::Function { body, .. }) = value {
                        match body {
                            FunctionBody::Block(stmts) => main_body = Some(stmts),
                            FunctionBody::Expression(inner) => {
                                if let Expression::Block { statements } = inner.as_ref() {
                                    main_body = Some(statements);
                                }
                            }
                        }
                    }
                }
            }
        }

        let main_type = self.context.i32_type().fn_type(&[], false);
        let main_function = self.module.add_function("main", main_type, None);
        let basic_block = self.context.append_basic_block(main_function, "entry");
        self.builder.position_at_end(basic_block);
        self.current_function = Some(main_function);
    self.variables.clear();

    if let Some(stmts) = main_body {
            for s in stmts {
                // Best-effort: generate only supported statements
                let _ = self.generate_statement(s);
            }
        } else {
            // Fallback: try to emit top-level statements (simple subset)
            for statement in &program.statements {
                let _ = self.generate_statement(statement);
            }
        }

        let zero = self.context.i32_type().const_int(0, false);
        self.builder.build_return(Some(&zero))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        Ok(())
    }
    
    fn generate_statement(&mut self, statement: &Statement) -> Result<(), CodegenError> {
        match statement {
            Statement::VariableDecl { name, type_annotation, value } => {
                // Struct literal special-case when annotated with a known struct type
                if let (Some(Type::Identifier(struct_name)), Expression::StructLiteral { fields }) = (type_annotation, value) {
                    // limit borrow scope and clone needed data
                    let (st_copy, order_clone) = if let Some((st, order)) = self.struct_types.get(struct_name) {
                        (*st, order.clone())
                    } else { (self.context.opaque_struct_type("__missing"), Vec::new()) };
                    if let Some((_st, _)) = self.struct_types.get(struct_name) {
                        let sval = self.build_struct_literal_value(struct_name, fields, st_copy, &order_clone)?;
                        let alloca = self.create_entry_block_alloca(name, st_copy.into())?;
                        self.builder.build_store(alloca, sval)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.variables.insert(name.clone(), (alloca, st_copy.into()));
                        return Ok(());
                    }
                }

                let mut value_result = self.generate_expression(value)?;
                let target_ty = if let Some(ty) = type_annotation { self.map_ast_type(ty).unwrap_or(value_result.get_type()) } else { value_result.get_type() };
                if value_result.get_type() != target_ty { value_result = self.cast_basic_to_type(value_result, target_ty)?; }
                let alloca = self.create_entry_block_alloca(name, target_ty)?;
                self.builder.build_store(alloca, value_result).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.variables.insert(name.clone(), (alloca, target_ty));
            }
            
            Statement::Assignment { target, value, .. } => {
                let value_result = self.generate_expression(value)?;

                match target {
                    Expression::Identifier(name) => {
                        let (variable, var_ty) = self
                            .variables
                            .get(name)
                            .ok_or_else(|| CodegenError::UndefinedVariable(name.clone()))?;
                        // Cast value to variable's type if needed
                        let casted = if value_result.get_type() != *var_ty {
                            self.cast_basic_to_type(value_result, *var_ty)?
                        } else {
                            value_result
                        };
                        self.builder
                            .build_store(*variable, casted)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    }
                    Expression::FieldAccess { object, field } => {
                        // Support simple var.field assignment
                        if let Expression::Identifier(base_name) = object.as_ref() {
                            if let Some((base_ptr, base_ty)) = self.variables.get(base_name) {
                                if let BasicTypeEnum::StructType(st) = base_ty {
                                    // Find struct layout and field index
                                    if let Some((_, (_llvm_st, order))) = self
                                        .struct_types
                                        .iter()
                                        .find(|(_, (llvm_st, _))| llvm_st == st)
                                    {
                                        if let Some(pos) = order.iter().position(|n| n == field) {
                                            let fty = st
                                                .get_field_type_at_index(pos as u32)
                                                .ok_or_else(|| CodegenError::InvalidOperation("field index out of range".to_string()))?;
                                            // Cast assigned value to field type
                                            let casted = match fty {
                                                BasicTypeEnum::IntType(it) => self.cast_to_int(value_result, it)?.into(),
                                                BasicTypeEnum::FloatType(ft) => self.cast_to_float(value_result, ft)?.into(),
                                                BasicTypeEnum::PointerType(pt) => self.cast_to_ptr(value_result, pt)?.into(),
                                                _ => value_result,
                                            };
                                            let fld_ptr = self
                                                .builder
                                                .build_struct_gep(*st, *base_ptr, pos as u32, "fldw")
                                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            self.builder
                                                .build_store(fld_ptr, casted)
                                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                        }
                                    }
                                } else if base_ty.is_pointer_type() {
                                    // Pointer to struct: use semantic type to locate struct layout and store via loaded pointer
                                    if let Some(struct_name) = self.semantic_struct_name_of_var(base_name) {
                                        if let Some((st, order)) = self.struct_types.get(&struct_name) {
                                            if let Some(pos) = order.iter().position(|n| n == field) {
                                                let fty = st
                                                    .get_field_type_at_index(pos as u32)
                                                    .ok_or_else(|| CodegenError::InvalidOperation("field index out of range".to_string()))?;
                                                let casted = match fty {
                                                    BasicTypeEnum::IntType(it) => self.cast_to_int(value_result, it)?.into(),
                                                    BasicTypeEnum::FloatType(ft) => self.cast_to_float(value_result, ft)?.into(),
                                                    BasicTypeEnum::PointerType(pt) => self.cast_to_ptr(value_result, pt)?.into(),
                                                    _ => value_result,
                                                };
                                                let loaded_ptr = self
                                                    .builder
                                                    .build_load(*base_ty, *base_ptr, "derefptr")
                                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                                let fld_ptr = self
                                                    .builder
                                                    .build_struct_gep(*st, loaded_ptr.into_pointer_value(), pos as u32, "fldw")
                                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                                self.builder
                                                    .build_store(fld_ptr, casted)
                                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            
            Statement::ForLoop { variable, iterable, body, .. } => {
                // Support: for i in N and for i in start:end[:step]
                let (init_val_opt, end_val_opt, step_val_opt) = match iterable {
                    Expression::Literal(Literal::Integer(n)) => (Some(self.context.i64_type().const_zero()), Some(self.context.i64_type().const_int(*n as u64, false)), Some(self.context.i64_type().const_int(1, false))),
                    Expression::Identifier(name) => {
                        if let Some((ptr, ty)) = self.variables.get(name) {
                            if let BasicTypeEnum::IntType(i_ty) = ty {
                                let v = self.builder.build_load(*ty, *ptr, &format!("{}_end", name)).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let iv = self.cast_to_int(v, *i_ty)?;
                                (Some(self.context.i64_type().const_zero()), Some(iv), Some(self.context.i64_type().const_int(1, false)))
                            } else { (None, None, None) }
                        } else { (None, None, None) }
                    }
                    Expression::Range { start, end, step } => {
                        // Try to detect constant negative step from AST (UnaryOp::Negate of integer)
                        let _step_ast_is_neg = matches!(step.as_deref(), Some(Expression::UnaryOp { operator: UnaryOperator::Negate, .. }));
                        let s = self.generate_expression(start)?;
                        let e = self.generate_expression(end)?;
                        let sv = step.as_ref().map(|x| self.generate_expression(x)).transpose()?;
                        let step_val_any = sv.unwrap_or(self.context.i64_type().const_int(1, false).into());
                        let step_i64 = self.cast_to_int(step_val_any, self.context.i64_type())?;
                        // Stash a marker in the high bit of step when AST says it's negative? Not needed; we'll recompute sign below and also carry the AST hint via a side channel
                        // We can't return extra flag here, so compute sign later using both is_const and AST hint.
                        (Some(self.cast_to_int(s, self.context.i64_type())?), Some(self.cast_to_int(e, self.context.i64_type())?), Some(step_i64))
                    }
                    _ => (None, None, None),
                };

                if let (Some(init_val), Some(end_val), Some(step_val)) = (init_val_opt, end_val_opt, step_val_opt) {
                    // Determine if step is a constant negative value (for comparator selection)
                    let step_is_const_neg = step_val.is_const() && step_val.get_zero_extended_constant().map(|v| (v as i64) < 0).unwrap_or(false);
                    // allocate loop var
                    let i_allo = self.create_entry_block_alloca(variable, self.context.i64_type().into())?;
                    self.builder.build_store(i_allo, init_val).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let prev = self.variables.insert(variable.clone(), (i_allo, self.context.i64_type().into()));

                    let current_fn = self.current_function.ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
                    let cond_bb = self.context.append_basic_block(current_fn, "for.cond");
                    let body_bb = self.context.append_basic_block(current_fn, "for.body");
                    let inc_bb = self.context.append_basic_block(current_fn, "for.inc");
                    let end_bb = self.context.append_basic_block(current_fn, "for.end");

                    // jump to cond
                    self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    // cond
                    self.builder.position_at_end(cond_bb);
                    let i64_bte: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                    let cur_i = self.builder.build_load(i64_bte, i_allo, "i")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                    // Condition depends on step sign. If step is a known negative constant, use cur_i > end; if unknown at compile-time, branch on step<0 to choose comparator.
                    // Prefer constant sign if available; else if non-const but AST hinted negative via unary, treat as negative
                    let step_ast_neg_hint = match iterable { Expression::Range { step, .. } => matches!(step.as_deref(), Some(Expression::UnaryOp { operator: UnaryOperator::Negate, .. })), _ => false };
                    let cmp = if step_is_const_neg || step_ast_neg_hint {
                        self.builder.build_int_compare(IntPredicate::SGT, cur_i, end_val, "forcmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    } else if step_val.is_const() {
                        self.builder.build_int_compare(IntPredicate::SLT, cur_i, end_val, "forcmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    } else {
                        // dynamic step: choose comparator blocks
                        let current_fn = self.current_function.ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
                        let neg_bb = self.context.append_basic_block(current_fn, "for.cond.neg");
                        let pos_bb = self.context.append_basic_block(current_fn, "for.cond.pos");
                        let join_bb = self.context.append_basic_block(current_fn, "for.cond.join");
                        let is_neg = self.builder.build_int_compare(IntPredicate::SLT, step_val, self.context.i64_type().const_zero(), "isneg").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder.build_conditional_branch(is_neg, neg_bb, pos_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        // neg path
                        self.builder.position_at_end(neg_bb);
                        let cmp_neg = self.builder.build_int_compare(IntPredicate::SGT, cur_i, end_val, "cmpneg").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder.build_unconditional_branch(join_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let neg_end = self.builder.get_insert_block().unwrap();
                        // pos path
                        self.builder.position_at_end(pos_bb);
                        let cmp_pos = self.builder.build_int_compare(IntPredicate::SLT, cur_i, end_val, "cmppos").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder.build_unconditional_branch(join_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let pos_end = self.builder.get_insert_block().unwrap();
                        // join
                        self.builder.position_at_end(join_bb);
                        let phi = self.builder.build_phi(self.context.bool_type(), "forcmpphi").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let neg_bv: BasicValueEnum<'ctx> = cmp_neg.into();
                        let pos_bv: BasicValueEnum<'ctx> = cmp_pos.into();
                        phi.add_incoming(&[(&neg_bv, neg_end), (&pos_bv, pos_end)]);
                        phi.as_basic_value().into_int_value()
                    };
                    self.builder.build_conditional_branch(cmp, body_bb, end_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    // body
                    self.builder.position_at_end(body_bb);
                    for s in body { let _ = self.generate_statement(s); }
                    if body_bb.get_terminator().is_none() {
                        self.builder.build_unconditional_branch(inc_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    }

                    // inc
                    self.builder.position_at_end(inc_bb);
                    let cur_i2 = self.builder.build_load(i64_bte, i_allo, "i")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                    let next = self.builder.build_int_add(cur_i2, step_val, "inc")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder.build_store(i_allo, next).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    // end
                    self.builder.position_at_end(end_bb);
                    if let Some(prev_binding) = prev { self.variables.insert(variable.clone(), prev_binding); } else { self.variables.remove(variable); }
                } else {
                    // unsupported iterable; skip
                }
            }

            Statement::Expression(expr) => {
                self.generate_expression(expr)?;
            }
            
            Statement::Return(expr) => {
                if let Some(expr) = expr {
                    let value = self.generate_expression(expr)?;
                    self.builder.build_return(Some(&value))
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                } else {
                    self.builder.build_return(None)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                }
            }
            
            _ => {
                // Other statements not implemented yet
            }
        }
        
        Ok(())
    }

    fn build_struct_literal_value(
        &mut self,
        struct_name: &str,
        expr_fields: &HashMap<String, Expression>,
        st: StructType<'ctx>,
        order: &Vec<String>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        // Allocate temp struct
        let tmp = self.create_entry_block_alloca(&format!("{}_tmp", struct_name), st.into())?;
        // Initialize each field by order
        for (i, fname) in order.iter().enumerate() {
            let fty = st.get_field_type_at_index(i as u32).ok_or_else(|| CodegenError::InvalidOperation("field index out of range".to_string()))?;
            let val = if let Some(expr) = expr_fields.get(fname) {
                self.generate_expression(expr)?
            } else {
                // default zero value
                match fty {
                    BasicTypeEnum::IntType(it) => it.const_zero().into(),
                    BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
                    BasicTypeEnum::PointerType(pt) => pt.const_zero().into(),
                    BasicTypeEnum::StructType(st2) => {
                        // nested struct: zero-initialize by storing null ptr pattern (not ideal); leave as undef zero via an alloca-load path could be added
                        let zero_alloca = self.create_entry_block_alloca("zero_struct", st2.into())?;
                        let bte: BasicTypeEnum<'ctx> = st2.into();
                        self.builder.build_load(bte, zero_alloca, "zst").map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    }
                    _ => self.context.i64_type().const_zero().into(),
                }
            };
            // Cast to field type if needed
            let casted = match fty {
                BasicTypeEnum::IntType(it) => self.cast_to_int(val, it)?.into(),
                BasicTypeEnum::FloatType(ft) => self.cast_to_float(val, ft)?.into(),
                BasicTypeEnum::PointerType(pt) => self.cast_to_ptr(val, pt)?.into(),
                _ => val,
            };
            let gep = self.builder.build_struct_gep(st, tmp, i as u32, "fldw").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            self.builder.build_store(gep, casted).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        }
        // Load as value
    let bte: BasicTypeEnum<'ctx> = st.into();
    let loaded = self.builder.build_load(bte, tmp, "tmp_load").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        Ok(loaded)
    }
    
    fn generate_expression(&mut self, expr: &Expression) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match expr {
            Expression::Literal(literal) => self.generate_literal(literal),
            
            Expression::Identifier(name) => {
                let (ptr, ty) = self.variables.get(name)
                    .ok_or_else(|| CodegenError::UndefinedVariable(name.clone()))?;
                self.builder.build_load(*ty, *ptr, name)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))
            }
            
            Expression::FieldAccess { object, field } => {
                // Support identifier.field; include special-case for `self` inside impl methods
                if let Expression::Identifier(var_name) = object.as_ref() {
                    if var_name == "self" {
                        // If we're in an impl method and `self` is a pointer to the impl struct, use that layout
                        if let Some(struct_name) = &self.current_impl_struct {
                            if let Some((st, order)) = self.struct_types.get(struct_name) {
                                if let Some((ptr, var_ty)) = self.variables.get(var_name) {
                                    // var_ty is pointer type; load the actual pointer to struct
                                    if var_ty.is_pointer_type() {
                                        let loaded_ptr = self.builder.build_load(*var_ty, *ptr, "selfloadptr")
                                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                        if let Some(pos) = order.iter().position(|n| n == field) {
                                            let fty = st.get_field_type_at_index(pos as u32)
                                                .ok_or_else(|| CodegenError::InvalidOperation("field index out of range".to_string()))?;
                                            let field_ptr = self.builder.build_struct_gep(*st, loaded_ptr.into_pointer_value(), pos as u32, "fld")
                                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            let val = self.builder.build_load(fty, field_ptr, "fldv")
                                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            return Ok(val);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some((ptr, var_ty)) = self.variables.get(var_name) {
                        if let BasicTypeEnum::StructType(st) = var_ty {
                            // Find struct name by matching st against registry
                            let (idx, fty) = if let Some((_, (_llvm_st, order))) = self.struct_types.iter().find(|(_, (llvm_st, _))| llvm_st == st) {
                                if let Some(pos) = order.iter().position(|n| n == field) {
                                    let fty = st.get_field_type_at_index(pos as u32).ok_or_else(|| CodegenError::InvalidOperation("field index out of range".to_string()))?;
                                    (pos as u32, fty)
                                } else { return Ok(self.context.i64_type().const_zero().into()); }
                            } else { return Ok(self.context.i64_type().const_zero().into()); };
                            let field_ptr = self.builder.build_struct_gep(*st, *ptr, idx, "fld").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let val = self.builder.build_load(fty, field_ptr, "fldv").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            Ok(val)
                        } else if var_ty.is_pointer_type() {
                            // Use semantic struct type to compute GEP on loaded pointer
                            if let Some(struct_name) = self.semantic_struct_name_of_var(var_name) {
                                if let Some((st, order)) = self.struct_types.get(&struct_name) {
                                    if let Some(pos) = order.iter().position(|n| n == field) {
                                        let loaded_ptr = self.builder.build_load(*var_ty, *ptr, "derefptr").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                        let fty = st.get_field_type_at_index(pos as u32).ok_or_else(|| CodegenError::InvalidOperation("field index out of range".to_string()))?;
                                        let field_ptr = self.builder.build_struct_gep(*st, loaded_ptr.into_pointer_value(), pos as u32, "fld").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                        let val = self.builder.build_load(fty, field_ptr, "fldv").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                        return Ok(val);
                                    }
                                }
                            }
                            Ok(self.context.i64_type().const_zero().into())
                        } else {
                            Ok(self.context.i64_type().const_zero().into())
                        }
                    } else {
                        Ok(self.context.i64_type().const_zero().into())
                    }
                } else {
                    Ok(self.context.i64_type().const_zero().into())
                }
            }
            
            Expression::Block { statements } => {
                let slice: &[Statement] = &statements[..];
                if let Some((last, prefix)) = slice.split_last() {
                    for s in prefix { let _ = self.generate_statement(s); }
                    if let Statement::Expression(expr) = last {
                        return self.generate_expression(expr);
                    } else {
                        let _ = self.generate_statement(last);
                    }
                }
                Ok(self.context.i64_type().const_zero().into())
            }

            Expression::If { condition, then_branch, else_branch } => {
                // Evaluate condition to i1
                let cond_val = self.generate_expression(condition)?;
                let cond_bool = if cond_val.is_int_value() {
                    let zero = cond_val.get_type().into_int_type().const_zero();
                    self.builder.build_int_compare(IntPredicate::NE, cond_val.into_int_value(), zero, "ifcond")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                } else if cond_val.is_float_value() {
                    let zero = cond_val.get_type().into_float_type().const_zero();
                    self.builder.build_float_compare(FloatPredicate::ONE, cond_val.into_float_value(), zero, "ifcond")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                } else if cond_val.is_pointer_value() {
                    self.builder.build_is_not_null(cond_val.into_pointer_value(), "ifcond")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                } else {
                    self.context.bool_type().const_zero()
                };

                let current_fn = self.current_function.ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
                let then_bb = self.context.append_basic_block(current_fn, "then");
                let else_bb = self.context.append_basic_block(current_fn, "else");
                let cont_bb = self.context.append_basic_block(current_fn, "ifcont");

                // Always branch to an explicit else block for SSA merging
                self.builder
                    .build_conditional_branch(cond_bool, then_bb, else_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                // then branch
                self.builder.position_at_end(then_bb);
                let then_val: BasicValueEnum<'ctx> = if then_branch.is_empty() {
                    self.context.i64_type().const_zero().into()
                } else {
                    let (prefix, last) = then_branch.split_at(then_branch.len() - 1);
                    for s in prefix { let _ = self.generate_statement(s); }
                    if let Some(Statement::Expression(expr)) = last.first() {
                        let v = self.generate_expression(expr)?;
                        self.cast_to_int(v, self.context.i64_type())?.into()
                    } else {
                        self.context.i64_type().const_zero().into()
                    }
                };
                if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                    self.builder
                        .build_unconditional_branch(cont_bb)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                }
                let then_end_bb = self.builder.get_insert_block().unwrap();

                // else branch
                self.builder.position_at_end(else_bb);
                let else_val: BasicValueEnum<'ctx> = if let Some(else_stmts) = else_branch {
                    if else_stmts.is_empty() {
                        self.context.i64_type().const_zero().into()
                    } else {
                        let (prefix, last) = else_stmts.split_at(else_stmts.len() - 1);
                        for s in prefix { let _ = self.generate_statement(s); }
                        if let Some(Statement::Expression(expr)) = last.first() {
                            let v = self.generate_expression(expr)?;
                            self.cast_to_int(v, self.context.i64_type())?.into()
                        } else {
                            self.context.i64_type().const_zero().into()
                        }
                    }
                } else {
                    // default false branch value when no else provided
                    self.context.i64_type().const_zero().into()
                };
                if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                    self.builder
                        .build_unconditional_branch(cont_bb)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                }
                let else_end_bb = self.builder.get_insert_block().unwrap();

                // continuation with phi merge
                self.builder.position_at_end(cont_bb);
                let phi = self
                    .builder
                    .build_phi(self.context.i64_type(), "iftmp")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                phi.add_incoming(&[(&then_val, then_end_bb), (&else_val, else_end_bb)]);
                Ok(phi.as_basic_value())
            }

            Expression::BinaryOp { left, operator, right } => {
                let left_val = self.generate_expression(left)?;
                let right_val = self.generate_expression(right)?;
                self.generate_binary_op(left_val, operator, right_val)
            }
            
            Expression::UnaryOp { operator, operand } => {
                let operand_val = self.generate_expression(operand)?;
                self.generate_unary_op(operator, operand_val)
            }
            
            Expression::Call { function, arguments } => {
                self.generate_call(function, arguments)
            }
            
            _ => {
                // Other expressions not implemented yet
                Ok(self.context.i64_type().const_zero().into())
            }
        }
    }
    
    fn generate_literal(&self, literal: &Literal) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match literal {
            Literal::Integer(value) => {
                Ok(self.context.i64_type().const_int(*value as u64, false).into())
            }
            
            Literal::Float(value) => {
                Ok(self.context.f64_type().const_float(*value).into())
            }
            
            Literal::Boolean(value) => {
                Ok(self.context.bool_type().const_int(*value as u64, false).into())
            }
            
            Literal::String(value) => {
                let string_val = self.builder.build_global_string_ptr(value, "str")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                Ok(string_val.as_pointer_value().into())
            }
        }
    }
    
    fn generate_binary_op(
        &mut self,
        left: BasicValueEnum<'ctx>,
        operator: &BinaryOperator,
        right: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match operator {
            BinaryOperator::Add => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self.builder.build_int_add(l, r, "addtmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self.builder.build_float_add(
                        left.into_float_value(),
                        right.into_float_value(),
                        "addtmp"
                    ).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation("Invalid types for addition".to_string()))
                }
            }
            
            BinaryOperator::Sub => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self.builder.build_int_sub(l, r, "subtmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self.builder.build_float_sub(
                        left.into_float_value(),
                        right.into_float_value(),
                        "subtmp"
                    ).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation("Invalid types for subtraction".to_string()))
                }
            }
            
            BinaryOperator::Mul => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self.builder.build_int_mul(l, r, "multmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self.builder.build_float_mul(
                        left.into_float_value(),
                        right.into_float_value(),
                        "multmp"
                    ).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation("Invalid types for multiplication".to_string()))
                }
            }
            
            BinaryOperator::Div => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self.builder.build_int_signed_div(l, r, "divtmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self.builder.build_float_div(
                        left.into_float_value(),
                        right.into_float_value(),
                        "divtmp"
                    ).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation("Invalid types for division".to_string()))
                }
            }
            
            BinaryOperator::Equal => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self.builder.build_int_compare(IntPredicate::EQ, l, r, "eqtmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self.builder.build_float_compare(
                        FloatPredicate::OEQ,
                        left.into_float_value(),
                        right.into_float_value(),
                        "eqtmp"
                    ).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation("Invalid types for equality comparison".to_string()))
                }
            }
            
            BinaryOperator::Less => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self.builder.build_int_compare(IntPredicate::SLT, l, r, "lttmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self.builder.build_float_compare(
                        FloatPredicate::OLT,
                        left.into_float_value(),
                        right.into_float_value(),
                        "lttmp"
                    ).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation("Invalid types for less than comparison".to_string()))
                }
            }
            
            _ => {
                Err(CodegenError::InvalidOperation(format!("Binary operator {:?} not implemented", operator)))
            }
        }
    }
    
    fn generate_unary_op(
        &mut self,
        operator: &UnaryOperator,
        operand: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match operator {
            UnaryOperator::Negate => {
                if operand.is_int_value() {
                    let result = self.builder.build_int_neg(operand.into_int_value(), "negtmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if operand.is_float_value() {
                    let result = self.builder.build_float_neg(operand.into_float_value(), "negtmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation("Invalid type for negation".to_string()))
                }
            }
            
            UnaryOperator::Not => {
                if operand.is_int_value() {
                    let result = self.builder.build_not(operand.into_int_value(), "nottmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation("Invalid type for logical not".to_string()))
                }
            }
            
            _ => {
                Err(CodegenError::InvalidOperation(format!("Unary operator {:?} not implemented", operator)))
            }
        }
    }
    
    fn generate_call(
        &mut self,
        function: &Expression,
        arguments: &[Argument],
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match function {
            Expression::Identifier(func_name) => {
                if func_name == "println" {
                    return self.generate_println_call(arguments);
                }

                // Deterministic: exact name, with support for trait static-path calls Trait_method(x, ...)
                let mut resolved: Option<FunctionValue<'ctx>> = self.functions.get(func_name).cloned();
                if resolved.is_none() {
                    if let Some((trait_name, method_name)) = func_name.split_once('_') {
                        if let Some(first) = arguments.get(0) {
                            if let Expression::Identifier(var) = &first.value {
                                if let Some(ty_name) = self.semantic_struct_name_of_var(var) {
                                    let mangled = format!("{}_{}_{}", trait_name, ty_name, method_name);
                                    resolved = self.functions.get(&mangled).cloned();
                                }
                            }
                        }
                    }
                }
                let function_val = match resolved { Some(f) => f, None => { return Ok(self.context.i64_type().const_zero().into()); } };

                // Build arguments, casting to declared metadata param types
                let param_metas = function_val.get_type().get_param_types();
                let mut arg_values: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
                for (i, arg) in arguments.iter().enumerate() {
                    // Heuristic: if callee expects a pointer for the first param and the argument is a local variable,
                    // pass its address (pointer) instead of loading the value.
                    if i == 0 {
                        if let Some(inkwell::types::BasicMetadataTypeEnum::PointerType(expect_pt)) = param_metas.get(i) {
                            if let Expression::Identifier(var) = &arg.value {
                                if let Some((ptr, _ty)) = self.variables.get(var) {
                                    let casted_ptr = self
                                        .builder
                                        .build_pointer_cast(*ptr, *expect_pt, "addrarg")
                                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    arg_values.push(casted_ptr.into());
                                    continue;
                                }
                            }
                        }
                    }

                    let value = self.generate_expression(&arg.value)?;
                    let casted_meta: BasicMetadataValueEnum<'ctx> = if let Some(meta_ty) = param_metas.get(i) {
                        match meta_ty {
                            inkwell::types::BasicMetadataTypeEnum::IntType(it) => self.cast_to_int(value, *it)?.into(),
                            inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => self.cast_to_float(value, *ft)?.into(),
                            inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => self.cast_to_ptr(value, *pt)?.into(),
                            _ => self.cast_to_int(value, self.context.i64_type())?.into(),
                        }
                    } else {
                        self.cast_to_int(value, self.context.i64_type())?.into()
                    };
                    arg_values.push(casted_meta);
                }
                // pad with zeros/nulls for missing args
                for i in arguments.len()..param_metas.len() {
                    let pad: BasicMetadataValueEnum<'ctx> = match param_metas[i] {
                        inkwell::types::BasicMetadataTypeEnum::IntType(it) => it.const_zero().into(),
                        inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => ft.const_zero().into(),
                        inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => pt.const_zero().into(),
                        _ => self.context.i64_type().const_zero().into(),
                    };
                    arg_values.push(pad);
                }

                let result = self
                    .builder
                    .build_call(function_val, &arg_values, "calltmp")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                if let Some(value) = result.try_as_basic_value().left() {
                    Ok(value)
                } else {
                    // Void function
                    Ok(self.context.i64_type().const_zero().into())
                }
            }
            Expression::FieldAccess { object, field } => {
                // Method call lowering: expr.method(args) => Type_method(self, args)
                // Only support when expr is an identifier bound to a known struct
                if let Expression::Identifier(var_name) = object.as_ref() {
                    if let Some((base_ptr, base_ty)) = self.variables.get(var_name) {
                        if let BasicTypeEnum::StructType(st) = base_ty {
                            if let Some((struct_name, (_llvm_st, _))) = self
                                .struct_types
                                .iter()
                                .find(|(_, (llvm_st, _))| llvm_st == st)
                            {
                                // Try inherent impl first
                                let mangled_inherent = format!("{}_{}", struct_name, field);
                                let mut selected_fn: Option<FunctionValue<'ctx>> = self.functions.get(&mangled_inherent).cloned();

                                // If not found, try trait impls for this struct type deterministically
                                if selected_fn.is_none() {
                                    // Collect candidate trait mangled names that have this method for this type
                                    let mut candidates: Vec<String> = Vec::new();
                                    for (trait_name, impls_for_trait) in &self.semantic.trait_impls {
                                        if let Some(info) = impls_for_trait.get(struct_name) {
                                            if info.methods.contains_key(field) {
                                                candidates.push(format!("{}_{}_{}", trait_name, struct_name, field));
                                            }
                                        }
                                    }
                                    // Choose lexicographically smallest for determinism if multiple traits match
                                    if let Some(best) = candidates.into_iter().min() {
                                        if let Some(fv) = self.functions.get(&best).cloned() {
                                            selected_fn = Some(fv);
                                        }
                                    }
                                }

                                if let Some(function_val) = selected_fn {
                                    // Prepare args: first receiver, then others cast to param types
                                    let param_metas = function_val.get_type().get_param_types();
                                    let mut arg_values: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
                                    // Receiver arg based on first param meta
                                    if let Some(first_meta) = param_metas.get(0) {
                                        let recv_meta: BasicMetadataValueEnum<'ctx> = match first_meta {
                                            inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => {
                                                let casted = self
                                                    .builder
                                                    .build_pointer_cast(*base_ptr, *pt, "selfpcast")
                                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                                casted.into()
                                            }
                                            inkwell::types::BasicMetadataTypeEnum::StructType(_st_meta) => {
                                                let bte: BasicTypeEnum<'ctx> = (*st).into();
                                                let loaded = self
                                                    .builder
                                                    .build_load(bte, *base_ptr, "selfload")
                                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                                loaded.into()
                                            }
                                            inkwell::types::BasicMetadataTypeEnum::IntType(it) => {
                                                let bte: BasicTypeEnum<'ctx> = (*st).into();
                                                let loaded = self
                                                    .builder
                                                    .build_load(bte, *base_ptr, "selfload")
                                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                                self.cast_to_int(loaded, *it)?.into()
                                            }
                                            inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => {
                                                let bte: BasicTypeEnum<'ctx> = (*st).into();
                                                let loaded = self
                                                    .builder
                                                    .build_load(bte, *base_ptr, "selfload")
                                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                                self.cast_to_float(loaded, *ft)?.into()
                                            }
                                            _ => (*base_ptr).into(),
                                        };
                                        arg_values.push(recv_meta);
                                    }
                                    // Other args
                                    for (i, arg) in arguments.iter().enumerate() {
                                        let value = self.generate_expression(&arg.value)?;
                                        let casted_meta: BasicMetadataValueEnum<'ctx> = if let Some(meta_ty) = param_metas.get(i + 1) {
                                            match meta_ty {
                                                inkwell::types::BasicMetadataTypeEnum::IntType(it) => self.cast_to_int(value, *it)?.into(),
                                                inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => self.cast_to_float(value, *ft)?.into(),
                                                inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => self.cast_to_ptr(value, *pt)?.into(),
                                                _ => self.cast_to_int(value, self.context.i64_type())?.into(),
                                            }
                                        } else {
                                            self.cast_to_int(value, self.context.i64_type())?.into()
                                        };
                                        arg_values.push(casted_meta);
                                    }
                                    // Pad if necessary
                                    for i in (arguments.len() + 1)..param_metas.len() {
                                        let pad: BasicMetadataValueEnum<'ctx> = match param_metas[i] {
                                            inkwell::types::BasicMetadataTypeEnum::IntType(it) => it.const_zero().into(),
                                            inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => ft.const_zero().into(),
                                            inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => pt.const_zero().into(),
                                            _ => self.context.i64_type().const_zero().into(),
                                        };
                                        arg_values.push(pad);
                                    }

                                    let result = self
                                        .builder
                                        .build_call(function_val, &arg_values, "calltmp")
                                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    if let Some(value) = result.try_as_basic_value().left() {
                                        return Ok(value);
                                    } else {
                                        return Ok(self.context.i64_type().const_zero().into());
                                    }
                                }
                            }
                        }
                    }
                }
                // Fallback when we can't lower method
                Ok(self.context.i64_type().const_zero().into())
            }
            _ => Err(CodegenError::InvalidOperation("Function calls on expressions not supported yet".to_string())),
        }
    }
    
    fn generate_println_call(&mut self, arguments: &[Argument]) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if arguments.is_empty() {
            // Just print a newline
            let newline_str = self.builder.build_global_string_ptr("\n", "newline")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            
            let printf_fn = self.functions.get("printf").unwrap();
            let args = vec![newline_str.as_pointer_value().into()];
            
            self.builder.build_call(*printf_fn, &args, "printf_call")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        } else {
            // Generate format string and arguments
            let mut format_parts = Vec::new();
            let mut printf_args: Vec<BasicValueEnum<'ctx>> = Vec::new();
            
            for (i, arg) in arguments.iter().enumerate() {
                let value = self.generate_expression(&arg.value)?;
                
                if value.is_int_value() && value.into_int_value().get_type().get_bit_width() == 1 {
                    // bool -> "true"/"false" via %s
                    let iv = value.into_int_value();
                    format_parts.push("%s");
                    let t = self.builder.build_global_string_ptr("true", "true_str").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let f = self.builder.build_global_string_ptr("false", "false_str").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let sel_ptr = self.builder.build_select(iv, t.as_pointer_value(), f.as_pointer_value(), "boolstr").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    printf_args.push(sel_ptr.into());
                } else if value.is_int_value() {
                    // widen to i64 for %lld
                    let iv = value.into_int_value();
                    let bw = iv.get_type().get_bit_width();
                    let widened: BasicValueEnum<'ctx> = if bw < 64 {
                        self.builder.build_int_s_extend(iv, self.context.i64_type(), "ext").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into()
                    } else { iv.into() };
                    format_parts.push("%lld");
                    printf_args.push(widened);
                } else if value.is_float_value() {
                    format_parts.push("%f");
                    printf_args.push(value.into());
                } else if value.is_pointer_value() {
                    format_parts.push("%s");
                    printf_args.push(value.into());
                } else {
                    // default to integer 0 widened to i64
                    format_parts.push("%lld");
                    printf_args.push(self.context.i64_type().const_zero().into());
                }
                
                if i < arguments.len() - 1 {
                    format_parts.push(" ");
                }
            }
            
            format_parts.push("\n");
            let format_string = format_parts.join("");
            
            let format_str_ptr = self.builder.build_global_string_ptr(&format_string, "format")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            
            let mut all_args: Vec<BasicValueEnum<'ctx>> = vec![format_str_ptr.as_pointer_value().into()];
            all_args.extend(printf_args);
            
            let all_args_metadata: Vec<_> = all_args.into_iter().map(|v| v.into()).collect();
            
            let printf_fn = self.functions.get("printf").unwrap();
            self.builder.build_call(*printf_fn, &all_args_metadata, "printf_call")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        }
        
        Ok(self.context.i32_type().const_int(0, false).into())
    }
    
    fn create_entry_block_alloca(
        &self,
        name: &str,
        ty: BasicTypeEnum<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let current_function = self.current_function
            .ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
        
        let builder = self.context.create_builder();
        let entry = current_function.get_first_basic_block()
            .ok_or_else(|| CodegenError::CompilationError("No entry block".to_string()))?;
        
        match entry.get_first_instruction() {
            Some(first_instr) => builder.position_before(&first_instr),
            None => builder.position_at_end(entry),
        }
        
        builder.build_alloca(ty, name)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))
    }
    
    pub fn print_ir(&self) {
        self.module.print_to_stderr();
    }
    
    pub fn write_object_file(&self, filename: &str) -> Result<(), CodegenError> {
    // Ensure LLVM targets are initialized
    Target::initialize_all(&InitializationConfig::default());
    let target_triple = inkwell::targets::TargetMachine::get_default_triple();
        let target = inkwell::targets::Target::from_triple(&target_triple)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        
        let target_machine = target
            .create_target_machine(
                &target_triple,
                "generic",
                "",
                inkwell::OptimizationLevel::Default,
                inkwell::targets::RelocMode::Default,
                inkwell::targets::CodeModel::Default,
            )
            .ok_or_else(|| CodegenError::CompilationError("Failed to create target machine".to_string()))?;
        
        target_machine
            .write_to_file(&self.module, inkwell::targets::FileType::Object, filename.as_ref())
            .map_err(|e| CodegenError::CompilationError(e.to_string()))
    }

    fn declare_and_define_functions(&mut self, program: &Program) -> Result<(), CodegenError> {
        // First pass: declare all functions with mapped param/ret types (skip top-level main; we'll build the real C entry separately)
        for stmt in &program.statements {
            match stmt {
                Statement::ConstDecl { name, value, .. } => {
                    if name == "main" { continue; }
                    if let ConstValue::Expression(Expression::Function { parameters, return_type, .. }) = value {
                        let ret_ty = return_type.as_ref().and_then(|t| self.map_ast_type(t)).unwrap_or(self.context.i64_type().into());
                        let param_tys_bte: Vec<BasicTypeEnum<'ctx>> = parameters.iter()
                            .map(|p| p.param_type.as_ref().and_then(|t| self.map_ast_type(t)).unwrap_or(self.context.i64_type().into()))
                            .collect();
                        let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> = param_tys_bte.iter().map(|t| (*t).into()).collect();
                        let fn_type = match ret_ty {
                            BasicTypeEnum::IntType(it) => it.fn_type(&param_meta, false),
                            BasicTypeEnum::FloatType(ft) => ft.fn_type(&param_meta, false),
                            BasicTypeEnum::PointerType(pt) => pt.fn_type(&param_meta, false),
                            _ => self.context.i64_type().fn_type(&param_meta, false),
                        };
                        let f = self.module.add_function(name.as_str(), fn_type, None);
                        self.functions.insert(name.clone(), f);
                    }
                }
                Statement::ImplBlock { trait_name, type_name, methods } => {
                    for m in methods {
                        if let Statement::ConstDecl { name: mname, value, .. } = m {
                            if let ConstValue::Expression(Expression::Function { parameters, return_type, .. }) = value {
                                let mangled = match trait_name {
                                    Some(tn) => format!("{}_{}_{}", tn, type_name, mname),
                                    None => format!("{}_{}", type_name, mname),
                                };
                                let ret_ty = return_type.as_ref().and_then(|t| self.map_ast_type(t)).unwrap_or(self.context.i64_type().into());
                                let param_tys_bte: Vec<BasicTypeEnum<'ctx>> = parameters.iter()
                                    .map(|p| p.param_type.as_ref().and_then(|t| self.map_ast_type(t)).unwrap_or(self.context.i64_type().into()))
                                    .collect();
                                let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> = param_tys_bte.iter().map(|t| (*t).into()).collect();
                                let fn_type = match ret_ty {
                                    BasicTypeEnum::IntType(it) => it.fn_type(&param_meta, false),
                                    BasicTypeEnum::FloatType(ft) => ft.fn_type(&param_meta, false),
                                    BasicTypeEnum::PointerType(pt) => pt.fn_type(&param_meta, false),
                                    _ => self.context.i64_type().fn_type(&param_meta, false),
                                };
                                let f = self.module.add_function(&mangled, fn_type, None);
                                self.functions.insert(mangled, f);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Second pass: define bodies
        for stmt in &program.statements {
            match stmt {
                Statement::ConstDecl { name, value, .. } => {
                    if name == "main" { continue; }
                    if let ConstValue::Expression(Expression::Function { parameters, body, return_type, .. }) = value {
                        let f = match self.functions.get(name) { Some(f) => *f, None => continue };
                    let entry = self.context.append_basic_block(f, "entry");
                    self.builder.position_at_end(entry);
                    let prev_fn = self.current_function;
                    let prev_vars = std::mem::take(&mut self.variables);
                    self.current_function = Some(f);

                    // Bind params to allocas
                    for (i, param) in f.get_param_iter().enumerate() {
                        let p_name = parameters.get(i).map(|p| p.name.clone()).unwrap_or(format!("arg{}", i));
                        let p_ty = parameters.get(i)
                            .and_then(|p| p.param_type.as_ref())
                            .and_then(|t| self.map_ast_type(t))
                            .unwrap_or(self.context.i64_type().into());
                        let alloca = self.create_entry_block_alloca(&p_name, p_ty)?;
                        self.builder.build_store(alloca, param).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.variables.insert(p_name, (alloca, p_ty));
                    }

                    match body {
                        FunctionBody::Expression(expr) => {
                            let v = self.generate_expression(expr)?;
                            let ret_ty = return_type.as_ref().and_then(|t| self.map_ast_type(t)).unwrap_or(self.context.i64_type().into());
                            let casted = self.cast_basic_to_type(v, ret_ty)?;
                            self.builder.build_return(Some(&casted)).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        }
                        FunctionBody::Block(stmts) => {
                            // Execute all statements; if the last is an expression, return its value
                            let mut last_expr_value: Option<BasicValueEnum<'ctx>> = None;
                            let slice: &[Statement] = &stmts[..];
                            if let Some((last, prefix)) = slice.split_last() {
                                for s in prefix { let _ = self.generate_statement(s); }
                                match last {
                                    Statement::Expression(expr) => {
                                        let v = self.generate_expression(expr)?;
                                        last_expr_value = Some(v);
                                    }
                                    other => { let _ = self.generate_statement(other); }
                                }
                            }
                            let ret_ty = return_type.as_ref().and_then(|t| self.map_ast_type(t)).unwrap_or(self.context.i64_type().into());
                            let ret_val: BasicValueEnum<'ctx> = if let Some(v) = last_expr_value {
                                self.cast_basic_to_type(v, ret_ty)?
                            } else {
                                match ret_ty {
                                    BasicTypeEnum::IntType(it) => it.const_zero().into(),
                                    BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
                                    BasicTypeEnum::PointerType(pt) => pt.const_zero().into(),
                                    _ => self.context.i64_type().const_zero().into(),
                                }
                            };
                            self.builder.build_return(Some(&ret_val)).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        }
                    }

                        // Restore state for next
                        self.variables = prev_vars;
                        self.current_function = prev_fn;
                    }
                }
                Statement::ImplBlock { trait_name, type_name, methods } => {
                    for m in methods {
                        if let Statement::ConstDecl { name: mname, value, .. } = m {
                            if let ConstValue::Expression(Expression::Function { parameters, body, return_type, .. }) = value {
                                let mangled = match trait_name {
                                    Some(tn) => format!("{}_{}_{}", tn, type_name, mname),
                                    None => format!("{}_{}", type_name, mname),
                                };
                                let f = match self.functions.get(&mangled) { Some(f) => *f, None => continue };
                                let entry = self.context.append_basic_block(f, "entry");
                                self.builder.position_at_end(entry);
                                let prev_fn = self.current_function;
                                let prev_impl_struct = self.current_impl_struct.clone();
                                let prev_vars = std::mem::take(&mut self.variables);
                                self.current_function = Some(f);
                                // Set current impl struct name for `self` handling
                                self.current_impl_struct = Some(type_name.clone());

                                // Bind params to allocas
                                for (i, param) in f.get_param_iter().enumerate() {
                                    let p_name = parameters.get(i).map(|p| p.name.clone()).unwrap_or(format!("arg{}", i));
                                    let p_ty = parameters.get(i)
                                        .and_then(|p| p.param_type.as_ref())
                                        .and_then(|t| self.map_ast_type(t))
                                        .unwrap_or(self.context.i64_type().into());
                                    let alloca = self.create_entry_block_alloca(&p_name, p_ty)?;
                                    self.builder.build_store(alloca, param).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.variables.insert(p_name, (alloca, p_ty));
                                }

                                match body {
                                    FunctionBody::Expression(expr) => {
                                        let v = self.generate_expression(expr)?;
                                        let ret_ty = return_type.as_ref().and_then(|t| self.map_ast_type(t)).unwrap_or(self.context.i64_type().into());
                                        let casted = self.cast_basic_to_type(v, ret_ty)?;
                                        self.builder.build_return(Some(&casted)).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    }
                                    FunctionBody::Block(stmts) => {
                                        let mut last_expr_value: Option<BasicValueEnum<'ctx>> = None;
                                        let slice: &[Statement] = &stmts[..];
                                        if let Some((last, prefix)) = slice.split_last() {
                                            for s in prefix { let _ = self.generate_statement(s); }
                                            match last {
                                                Statement::Expression(expr) => {
                                                    let v = self.generate_expression(expr)?;
                                                    last_expr_value = Some(v);
                                                }
                                                other => { let _ = self.generate_statement(other); }
                                            }
                                        }
                                        let ret_ty = return_type.as_ref().and_then(|t| self.map_ast_type(t)).unwrap_or(self.context.i64_type().into());
                                        let ret_val: BasicValueEnum<'ctx> = if let Some(v) = last_expr_value { self.cast_basic_to_type(v, ret_ty)? } else { match ret_ty { BasicTypeEnum::IntType(it) => it.const_zero().into(), BasicTypeEnum::FloatType(ft) => ft.const_zero().into(), BasicTypeEnum::PointerType(pt) => pt.const_zero().into(), _ => self.context.i64_type().const_zero().into(), } };
                                        self.builder.build_return(Some(&ret_val)).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    }
                                }

                                // Restore state for next
                                self.variables = prev_vars;
                                self.current_function = prev_fn;
                                self.current_impl_struct = prev_impl_struct;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }
}
