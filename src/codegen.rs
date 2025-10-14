use crate::ast::{
    Argument, BinaryOperator, BindingPattern, ConstValue, Expression, FunctionBody, IntegerLiteral,
    Literal, MatchArm, Program, ResourceAccess, Statement, SystemParameter, Type, UnaryOperator,
};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{InitializationConfig, Target};
use inkwell::types::{BasicType, BasicTypeEnum, FloatType, IntType, StructType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, GlobalValue, IntValue,
    PointerValue, StructValue,
};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate};
use std::collections::{HashMap, HashSet};
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

const UNSIGNED_TYPE_NAMES: [&str; 4] = ["u8", "u16", "u32", "u64"];

struct LoopContext<'ctx> {
    continue_bb: BasicBlock<'ctx>,
    break_bb: BasicBlock<'ctx>,
}

struct LoopScope<'ctx> {
    stack: *mut Vec<LoopContext<'ctx>>,
}

impl<'ctx> LoopScope<'ctx> {
    fn new(
        stack: &mut Vec<LoopContext<'ctx>>,
        continue_bb: BasicBlock<'ctx>,
        break_bb: BasicBlock<'ctx>,
    ) -> Self {
        stack.push(LoopContext {
            continue_bb,
            break_bb,
        });
        Self {
            stack: stack as *mut Vec<LoopContext<'ctx>>,
        }
    }
}

impl<'ctx> Drop for LoopScope<'ctx> {
    fn drop(&mut self) {
        unsafe {
            (*self.stack).pop();
        }
    }
}

pub struct CodeGenerator<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    current_function: Option<FunctionValue<'ctx>>,
    current_function_return_ast: Option<crate::ast::Type>,
    semantic: crate::semantic::SemanticContext,
    struct_types: HashMap<String, (StructType<'ctx>, Vec<String>)>, // name -> (llvm struct, field order)
    current_impl_struct: Option<String>,
    // Track vector lengths for variables bound to 1D matrix literals
    vector_lengths: HashMap<String, u64>,
    // Track matrix rank (1 for 1D vector, 2+ for multi-dim) per variable name
    matrix_rank: HashMap<String, usize>,
    // When true, emit a freestanding runtime entry (_start) and do not emit a C main().
    runtime_mode: bool,
    // Local variable types for current function
    local_types: HashMap<String, crate::ast::Type>,
    unsigned_variables: HashSet<String>,
    // Struct type for enum representation { tag: i64, payload: i64 }
    enum_struct: Option<StructType<'ctx>>,
    // Cached struct type for Vec values (data: i64*, len: i64, capacity: i64)
    vector_struct_type: Option<StructType<'ctx>>,
    loop_stack: Vec<LoopContext<'ctx>>,
    current_binary_context: Option<String>,
    owned_locals: Vec<String>,
    command_line_argc_global: Option<GlobalValue<'ctx>>,
    command_line_argv_global: Option<GlobalValue<'ctx>>,
}

impl<'ctx> CodeGenerator<'ctx> {
    pub fn new(
        context: &'ctx Context,
        semantic_context: crate::semantic::SemanticContext,
    ) -> Result<Self, CodegenError> {
        let module = context.create_module("main");

        let mut generator = CodeGenerator {
            context,
            module,
            builder: context.create_builder(),
            variables: HashMap::new(),
            functions: HashMap::new(),
            current_function: None,
            current_function_return_ast: None,
            semantic: semantic_context,
            struct_types: HashMap::new(),
            current_impl_struct: None,
            vector_lengths: HashMap::new(),
            matrix_rank: HashMap::new(),
            runtime_mode: false,
            local_types: HashMap::new(),
            unsigned_variables: HashSet::new(),
            enum_struct: None,
            vector_struct_type: None,
            loop_stack: Vec::new(),
            current_binary_context: None,
            owned_locals: Vec::new(),
            command_line_argc_global: None,
            command_line_argv_global: None,
        };

        generator.declare_external_functions()?;
        generator.build_struct_types()?;

        Ok(generator)
    }

    // Build a return only if the current insert block has no terminator yet
    fn try_build_return(
        &self,
        val: Option<&dyn inkwell::values::BasicValue<'ctx>>,
    ) -> Result<(), CodegenError> {
        if let Some(block) = self.builder.get_insert_block() {
            if block.get_terminator().is_none() {
                self.builder
                    .build_return(val)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            }
        }
        Ok(())
    }

    fn branch_to(
        &self,
        target: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<(), CodegenError> {
        if let Some(current_block) = self.builder.get_insert_block() {
            if current_block.get_terminator().is_none() {
                self.builder
                    .build_unconditional_branch(target)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            }
        }
        Ok(())
    }

    fn statements_contain_return(statements: &[Statement]) -> bool {
        statements
            .iter()
            .any(|stmt| Self::statement_contains_return(stmt))
    }

    fn statement_contains_return(stmt: &Statement) -> bool {
        match stmt {
            Statement::Return(_) => true,
            Statement::Expression(expr) => Self::expression_contains_return(expr),
            Statement::VariableDecl { value, .. } | Statement::Assignment { value, .. } => {
                Self::expression_contains_return(value)
            }
            Statement::ForLoop { body, .. }
            | Statement::ModuleDecl {
                items: Some(body), ..
            }
            | Statement::ImplBlock { methods: body, .. } => Self::statements_contain_return(body),
            Statement::ImplMethod { body, .. } => match body {
                FunctionBody::Expression(expr) => Self::expression_contains_return(expr),
                FunctionBody::Block(stmts) => Self::statements_contain_return(stmts),
            },
            Statement::IfDef {
                then_branch,
                else_branch,
                ..
            } => {
                Self::statements_contain_return(then_branch)
                    || else_branch
                        .as_ref()
                        .map_or(false, |branch| Self::statements_contain_return(branch))
            }
            _ => false,
        }
    }

    fn expression_contains_return(expr: &Expression) -> bool {
        match expr {
            Expression::Block { statements } | Expression::UnsafeBlock { statements } => {
                Self::statements_contain_return(statements)
            }
            Expression::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::statements_contain_return(then_branch)
                    || else_branch
                        .as_ref()
                        .map_or(false, |branch| Self::statements_contain_return(branch))
            }
            Expression::IfExpr {
                then_expr,
                else_expr,
                ..
            } => {
                Self::expression_contains_return(then_expr)
                    || else_expr
                        .as_ref()
                        .map_or(false, |expr| Self::expression_contains_return(expr))
            }
            Expression::Loop { body } => Self::statements_contain_return(body),
            Expression::Match { arms, .. } => arms
                .iter()
                .any(|arm| Self::expression_contains_return(&arm.body)),
            Expression::Question(inner) | Expression::Unwrap(inner) => {
                Self::expression_contains_return(inner)
            }
            _ => false,
        }
    }

    fn const_int_from_literal(
        &self,
        literal: &IntegerLiteral,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let value = literal.value;
        if value > u64::MAX as u128 {
            return Err(CodegenError::InvalidOperation(format!(
                "integer literal {} exceeds supported range for codegen",
                literal.raw
            )));
        }
        let bit_width = literal.bit_width();
        let target_ty = if bit_width <= 8 {
            self.context.i8_type()
        } else if bit_width <= 16 {
            self.context.i16_type()
        } else if bit_width <= 32 {
            self.context.i32_type()
        } else {
            self.context.i64_type()
        };
        Ok(target_ty.const_int(value as u64, false))
    }

    fn get_pattern_tag(
        &self,
        pattern: &Expression,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        match pattern {
            Expression::Identifier(name) => {
                if name == "none" {
                    let tag = self.context.i64_type().const_zero();
                    eprintln!(
                        "pattern tag for none: {}",
                        tag.get_zero_extended_constant().unwrap_or(999)
                    );
                    return Ok(tag);
                }
                if let Some((tname, vname)) = name.split_once('_') {
                    if let Some(Type::Enum { variants, order }) = self.semantic.types.get(tname) {
                        if variants.contains_key(vname) {
                            let idx = order.iter().position(|s| s == vname).unwrap_or(0) as u64;
                            let tag = self.context.i64_type().const_int(idx, false);
                            eprintln!("pattern tag for {}_{}: {}", tname, vname, idx);
                            return Ok(tag);
                        }
                    }
                }
                Err(CodegenError::UndefinedVariable(name.clone()))
            }
            Expression::StaticPath { segments, .. } => {
                if segments.len() >= 2 {
                    let type_name = &segments[0];
                    let variant_name = &segments[1];
                    if let Some(Type::Enum { order, .. }) = self.semantic.types.get(type_name) {
                        if let Some(idx) = order.iter().position(|s| s == variant_name) {
                            return Ok(self.context.i64_type().const_int(idx as u64, false));
                        }
                    }
                }
                Err(CodegenError::UndefinedVariable(segments.join("::")))
            }
            Expression::StructLiteral { type_name, .. } => {
                if let Some(name) = type_name {
                    let base = name.strip_suffix("_struct").unwrap_or(name);
                    if let Some((enum_name, variant_part)) = base.split_once('_') {
                        if let Some(Type::Enum { order, .. }) = self.semantic.types.get(enum_name) {
                            if let Some(idx) = order.iter().position(|s| s == variant_part) {
                                return Ok(self.context.i64_type().const_int(idx as u64, false));
                            }
                        }
                    }
                }
                Err(CodegenError::CompilationError(format!(
                    "Unsupported struct literal pattern: {:?}",
                    type_name
                )))
            }
            Expression::Call {
                function,
                type_args: _,
                ..
            } => {
                match function.as_ref() {
                    Expression::Identifier(func_name) => {
                        if func_name == "some" {
                            return Ok(self.context.i64_type().const_int(1, false));
                        }
                        if func_name == "ok" {
                            return Ok(self.context.i64_type().const_zero());
                        }
                        if func_name == "err" {
                            return Ok(self.context.i64_type().const_int(1, false));
                        }
                        if let Some((tname, vname)) = func_name.split_once('_') {
                            if let Some(Type::Enum { variants, order }) =
                                self.semantic.types.get(tname)
                            {
                                if variants.contains_key(vname) {
                                    let idx =
                                        order.iter().position(|s| s == vname).unwrap_or(0) as u64;
                                    return Ok(self.context.i64_type().const_int(idx, false));
                                }
                            }
                        }
                    }
                    Expression::StaticPath { segments, .. } => {
                        if segments.len() >= 2 {
                            let type_name = &segments[0];
                            let variant_name = &segments[1];
                            if let Some(Type::Enum { order, .. }) =
                                self.semantic.types.get(type_name)
                            {
                                if let Some(idx) = order.iter().position(|s| s == variant_name) {
                                    return Ok(self
                                        .context
                                        .i64_type()
                                        .const_int(idx as u64, false));
                                }
                            }
                        }
                    }
                    _ => {}
                }
                Err(CodegenError::CompilationError(
                    "Invalid pattern".to_string(),
                ))
            }
            _ => Err(CodegenError::CompilationError(format!(
                "Unsupported pattern: {:?}",
                pattern
            ))),
        }
    }

    fn pattern_requires_payload(&self, pattern: &Expression) -> bool {
        match pattern {
            Expression::StructLiteral { fields, .. } => !fields.is_empty(),
            Expression::Call {
                function,
                arguments,
                ..
            } => {
                if arguments.is_empty() {
                    return false;
                }
                match function.as_ref() {
                    Expression::Identifier(func_name) => {
                        if func_name == "some" || func_name == "ok" || func_name == "err" {
                            return true;
                        }
                        if let Some((tname, vname)) = func_name.split_once('_') {
                            if let Some(Type::Enum { variants, .. }) =
                                self.semantic.types.get(tname)
                            {
                                return variants
                                    .get(vname)
                                    .map(|payload| payload.is_some())
                                    .unwrap_or(false);
                            }
                        }
                        false
                    }
                    Expression::StaticPath { segments, .. } => {
                        if segments.len() >= 2 {
                            let type_name = &segments[0];
                            let variant_name = &segments[1];
                            if let Some(Type::Enum { variants, .. }) =
                                self.semantic.types.get(type_name)
                            {
                                return variants
                                    .get(variant_name)
                                    .map(|payload| payload.is_some())
                                    .unwrap_or(false);
                            }
                        }
                        false
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Enable or disable freestanding runtime mode. When enabled, the codegen will:
    /// - Declare user `main` function as `tricti_main` (not emit a C `main`)
    /// - Emit a `_start` symbol that calls `tricti_main` and then `exit(code)`
    pub fn enable_runtime_mode(&mut self, enabled: bool) {
        self.runtime_mode = enabled;
    }

    // Build LLVM struct types for all user-declared struct types in semantics
    fn build_struct_types(&mut self) -> Result<(), CodegenError> {
        use crate::ast::Type as AstType;
        let _ = self.ensure_enum_struct_type();
        // Iterate semantic types and create LLVM struct definitions
        for (name, ty) in &self.semantic.types {
            if let AstType::Struct { fields } = ty {
                // Determine a stable field order (insertion order) so we can map names -> indices
                let order: Vec<String> = fields.keys().cloned().collect();

                // Create or fetch an opaque struct with this name, then set its body
                let st = self.context.opaque_struct_type(name);
                let mut llvm_fields: Vec<BasicTypeEnum<'ctx>> = Vec::with_capacity(order.len());
                for fname in &order {
                    if let Some(fty) = fields.get(fname) {
                        // Map field type to an LLVM type; default to i64 when unknown
                        let bte = self
                            .map_ast_type(fty)
                            .unwrap_or(self.context.i64_type().into());
                        llvm_fields.push(bte);
                    } else {
                        llvm_fields.push(self.context.i64_type().into());
                    }
                }
                st.set_body(&llvm_fields, false);
                self.struct_types.insert(name.clone(), (st, order));
                eprintln!("registered struct type {}", name);
            }
        }
        Ok(())
    }

    // Try to map an AST type to a concrete LLVM BasicTypeEnum; fallback to i64
    fn map_ast_type(&self, t: &crate::ast::Type) -> Option<BasicTypeEnum<'ctx>> {
        use crate::ast::Type as AstType;
        match t {
            AstType::Identifier { name, type_args: _ } => match name.as_str() {
                "i8" | "u8" => Some(self.context.i8_type().into()),
                "i16" | "u16" => Some(self.context.i16_type().into()),
                "i32" => Some(self.context.i32_type().into()),
                "u32" => Some(self.context.i32_type().into()),
                "i64" => Some(self.context.i64_type().into()),
                "u64" => Some(self.context.i64_type().into()),
                "f32" => Some(self.context.f32_type().into()),
                "f64" => Some(self.context.f64_type().into()),
                "bool" => Some(self.context.bool_type().into()),
                "char" => Some(self.context.i32_type().into()),
                "string" | "String" | "str" => {
                    Some(self.context.ptr_type(AddressSpace::default()).into())
                }
                _ => {
                    // Fallback: if identifier names a known struct, return its LLVM struct type
                    if let Some((st, _order)) = self.struct_types.get(name) {
                        Some((*st).into())
                    } else {
                        None
                    }
                }
            },
            AstType::Pointer { .. } | AstType::RawPointer { .. } | AstType::Reference { .. } => {
                Some(self.context.ptr_type(AddressSpace::default()).into())
            }
            AstType::Optional { .. } | AstType::Result { .. } => {
                self.enum_struct.map(|st| st.into())
            }
            AstType::Tuple(elems) => {
                let element_types: Vec<BasicTypeEnum<'ctx>> = elems
                    .iter()
                    .map(|elem_ty| {
                        self.map_ast_type(elem_ty)
                            .unwrap_or_else(|| self.context.i64_type().into())
                    })
                    .collect();

                if element_types.is_empty() {
                    Some(self.context.struct_type(&[], false).into())
                } else {
                    Some(self.context.struct_type(&element_types, false).into())
                }
            }
            AstType::Struct { .. } | AstType::Trait { .. } | AstType::Function { .. } => None,
            AstType::Enum { .. } => Some(self.enum_struct.unwrap().into()),
            AstType::Matrix { .. } => {
                if let Some(st) = self.vector_struct_type {
                    Some(st.into())
                } else if let Some((st, _)) = self.struct_types.get("Vec") {
                    Some((*st).into())
                } else {
                    Some(self.context.ptr_type(AddressSpace::default()).into())
                }
            }
            AstType::None => Some(self.context.i64_type().into()),
        }
    }

    // Cast a BasicValueEnum to another BasicTypeEnum best-effort
    fn cast_basic_to_type(
        &self,
        v: BasicValueEnum<'ctx>,
        target: BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        Ok(match target {
            BasicTypeEnum::IntType(it) => self.cast_to_int(v, it)?.into(),
            BasicTypeEnum::FloatType(ft) => self.cast_to_float(v, ft)?.into(),
            BasicTypeEnum::PointerType(pt) => self.cast_to_ptr(v, pt)?.into(),
            _ => v,
        })
    }

    fn ensure_bool_value(
        &self,
        value: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let bool_ty = self.context.bool_type();
        let casted = self.cast_basic_to_type(value, bool_ty.into())?;
        if casted.is_int_value() {
            Ok(casted.into_int_value())
        } else {
            Err(CodegenError::InvalidOperation(
                "Expected boolean-compatible value".to_string(),
            ))
        }
    }

    fn ensure_vector_struct_type(&mut self) -> Result<StructType<'ctx>, CodegenError> {
        if let Some(st) = self.vector_struct_type {
            return Ok(st);
        }

        if let Some((st, _)) = self.struct_types.get("Vec") {
            let st = *st;
            self.vector_struct_type = Some(st);
            return Ok(st);
        }

        let st = self.context.opaque_struct_type("Vec");
        st.set_body(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i64_type().into(),
                self.context.i64_type().into(),
            ],
            false,
        );
        self.struct_types.insert(
            "Vec".to_string(),
            (
                st,
                vec![
                    "data".to_string(),
                    "len".to_string(),
                    "capacity".to_string(),
                ],
            ),
        );
        self.vector_struct_type = Some(st);
        Ok(st)
    }

    fn ensure_enum_struct_type(&mut self) -> StructType<'ctx> {
        if let Some(st) = self.enum_struct {
            st
        } else {
            let st = self.context.opaque_struct_type("EnumRepr");
            st.set_body(
                &[
                    self.context.i64_type().into(),
                    self.context.i64_type().into(),
                ],
                false,
            );
            self.enum_struct = Some(st);
            st
        }
    }

    fn ensure_command_line_struct(&mut self) -> Result<(StructType<'ctx>, u32), CodegenError> {
        if let Some((st, order)) = self.struct_types.get("CommandLine") {
            let idx = order.iter().position(|f| f == "args").ok_or_else(|| {
                CodegenError::CompilationError("CommandLine.args field missing".to_string())
            })? as u32;
            return Ok((*st, idx));
        }

        let vec_struct = self.ensure_vector_struct_type()?;
        let st = self.context.opaque_struct_type("CommandLine");
        st.set_body(&[vec_struct.into()], false);
        self.struct_types
            .insert("CommandLine".to_string(), (st, vec!["args".to_string()]));
        Ok((st, 0))
    }

    fn ensure_path_struct(&mut self) -> Result<(StructType<'ctx>, u32), CodegenError> {
        if let Some((st, order)) = self.struct_types.get("Path") {
            let idx = order.iter().position(|f| f == "path").ok_or_else(|| {
                CodegenError::CompilationError("Path.path field missing".to_string())
            })? as u32;
            return Ok((*st, idx));
        }

        let string_ty = self.context.ptr_type(AddressSpace::default());
        let st = self.context.opaque_struct_type("Path");
        st.set_body(&[string_ty.into()], false);
        self.struct_types
            .insert("Path".to_string(), (st, vec!["path".to_string()]));
        Ok((st, 0))
    }

    fn ensure_struct_type_by_name(
        &mut self,
        name: &str,
    ) -> Result<(StructType<'ctx>, Vec<String>), CodegenError> {
        if let Some((st, order)) = self.struct_types.get(name) {
            return Ok((*st, order.clone()));
        }

        if let Some(crate::ast::Type::Struct { fields }) = self.semantic.types.get(name) {
            let order: Vec<String> = fields.keys().cloned().collect();
            let mut llvm_fields: Vec<BasicTypeEnum<'ctx>> = Vec::with_capacity(order.len());
            for fname in &order {
                let field_ty = fields.get(fname).ok_or_else(|| {
                    CodegenError::CompilationError(format!(
                        "missing field '{}' while building struct {}",
                        fname, name
                    ))
                })?;
                let llvm_ty = self
                    .map_ast_type(field_ty)
                    .unwrap_or(self.context.i64_type().into());
                llvm_fields.push(llvm_ty);
            }
            let st = self.context.opaque_struct_type(name);
            st.set_body(&llvm_fields, false);
            self.struct_types
                .insert(name.to_string(), (st, order.clone()));
            return Ok((st, order));
        }

        let trimmed = name.strip_suffix("_struct").unwrap_or(name);
        if let Some((enum_name, variant_name)) = trimmed.rsplit_once('_') {
            if let Some(Type::Enum { variants, .. }) = self.semantic.types.get(enum_name) {
                if let Some(Some(payload_ty)) = variants.get(variant_name) {
                    let resolved = self.semantic.resolve_type(payload_ty);
                    if let Type::Struct { fields } = resolved {
                        let order: Vec<String> = fields.keys().cloned().collect();
                        let mut llvm_fields: Vec<BasicTypeEnum<'ctx>> =
                            Vec::with_capacity(order.len());
                        for fname in &order {
                            let field_ty = fields.get(fname).ok_or_else(|| {
                                CodegenError::CompilationError(format!(
                                    "missing field '{}' while building struct {}",
                                    fname, name
                                ))
                            })?;
                            let llvm_ty = self
                                .map_ast_type(field_ty)
                                .unwrap_or(self.context.i64_type().into());
                            llvm_fields.push(llvm_ty);
                        }
                        let st = self.context.opaque_struct_type(name);
                        st.set_body(&llvm_fields, false);
                        self.struct_types
                            .insert(name.to_string(), (st, order.clone()));
                        return Ok((st, order));
                    }
                }
            }
        }

        Err(CodegenError::CompilationError(format!(
            "unknown struct type {}",
            name
        )))
    }
    fn vector_field_indices(&mut self) -> Result<(StructType<'ctx>, u32, u32, u32), CodegenError> {
        let st = self.ensure_vector_struct_type()?;
        let order = self
            .struct_types
            .get("Vec")
            .map(|(_, order)| order.clone())
            .ok_or_else(|| {
                CodegenError::InvalidOperation("Vec struct type not registered".to_string())
            })?;
        let data_idx = order.iter().position(|n| n == "data").ok_or_else(|| {
            CodegenError::InvalidOperation("Vec struct missing data field".to_string())
        })? as u32;
        let len_idx = order.iter().position(|n| n == "len").ok_or_else(|| {
            CodegenError::InvalidOperation("Vec struct missing len field".to_string())
        })? as u32;
        let cap_idx = order.iter().position(|n| n == "capacity").ok_or_else(|| {
            CodegenError::InvalidOperation("Vec struct missing capacity field".to_string())
        })? as u32;
        Ok((st, data_idx, len_idx, cap_idx))
    }

    fn try_generate_vector_method_call(
        &mut self,
        var_name: &str,
        field: &str,
        arguments: &[Argument],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let (alloca, stored_ty) = match self.variables.get(var_name) {
            Some((alloca, stored_ty)) => (*alloca, *stored_ty),
            None => return Ok(None),
        };

        let i64_ty = self.context.i64_type();
        let zero_i64 = i64_ty.const_zero();
        let (vec_struct, data_idx, len_idx, cap_idx) = self.vector_field_indices()?;

        let stored_struct = match stored_ty {
            BasicTypeEnum::StructType(st) => st,
            _ => return Ok(None),
        };

        if stored_struct != vec_struct {
            return Ok(None);
        }

        let data_field_ptr = self
            .builder
            .build_struct_gep(vec_struct, alloca, data_idx, "vec_data_ptr")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let len_ptr = self
            .builder
            .build_struct_gep(vec_struct, alloca, len_idx, "vec_len_ptr")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let cap_ptr = self
            .builder
            .build_struct_gep(vec_struct, alloca, cap_idx, "vec_cap_ptr")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        let elem_ptr_ty = self.context.ptr_type(AddressSpace::default());
        let data_ptr = self
            .builder
            .build_load(elem_ptr_ty, data_field_ptr, "vec_data_load")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();

        match field {
            "len" if arguments.is_empty() => {
                let len_val = self
                    .builder
                    .build_load(i64_ty, len_ptr, "vec_len")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                Ok(Some(len_val))
            }
            "is_empty" if arguments.is_empty() => {
                let len_val = self
                    .builder
                    .build_load(i64_ty, len_ptr, "vec_len")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_int_value();
                let is_empty = self
                    .builder
                    .build_int_compare(IntPredicate::EQ, len_val, zero_i64, "vec_is_empty")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                Ok(Some(is_empty.into()))
            }
            "push" if arguments.len() == 1 => {
                let elem_raw = self.generate_expression(&arguments[0].value)?;
                let elem_i64 = self.cast_to_int(elem_raw, i64_ty)?;

                let len_val_initial = self
                    .builder
                    .build_load(i64_ty, len_ptr, "vec_len")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_int_value();
                let cap_val_initial = self
                    .builder
                    .build_load(i64_ty, cap_ptr, "vec_cap")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_int_value();
                let needs_grow = self
                    .builder
                    .build_int_compare(
                        IntPredicate::UGE,
                        len_val_initial,
                        cap_val_initial,
                        "vec_need_grow",
                    )
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                let current_fn = self.current_function.ok_or_else(|| {
                    CodegenError::CompilationError(
                        "vector method called outside of function".to_string(),
                    )
                })?;
                let grow_bb = self.context.append_basic_block(current_fn, "vec.push.grow");
                let push_cont_bb = self.context.append_basic_block(current_fn, "vec.push.cont");

                self.builder
                    .build_conditional_branch(needs_grow, grow_bb, push_cont_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                // Grow block
                self.builder.position_at_end(grow_bb);
                let cap_is_zero = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        cap_val_initial,
                        zero_i64,
                        "vec_cap_is_zero",
                    )
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let double_cap = self
                    .builder
                    .build_int_mul(
                        cap_val_initial,
                        i64_ty.const_int(2, false),
                        "vec_double_cap",
                    )
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let base_cap = self
                    .builder
                    .build_select::<BasicValueEnum, IntValue>(
                        cap_is_zero,
                        i64_ty.const_int(4, false).into(),
                        double_cap.into(),
                        "vec_new_cap",
                    )
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_int_value();
                let len_plus_one = self
                    .builder
                    .build_int_add(
                        len_val_initial,
                        i64_ty.const_int(1, false),
                        "vec_len_plus_one",
                    )
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let cap_too_small = self
                    .builder
                    .build_int_compare(
                        IntPredicate::ULT,
                        base_cap,
                        len_plus_one,
                        "vec_cap_too_small",
                    )
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let adjusted_cap = self
                    .builder
                    .build_select::<BasicValueEnum, IntValue>(
                        cap_too_small,
                        len_plus_one.into(),
                        base_cap.into(),
                        "vec_cap_adjusted",
                    )
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_int_value();

                let elem_size = i64_ty.const_int(8, false);
                let size_bytes = self
                    .builder
                    .build_int_mul(adjusted_cap, elem_size, "vec_realloc_size")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                let alloc_fn = self
                    .functions
                    .get("alloc")
                    .cloned()
                    .ok_or_else(|| CodegenError::UndefinedFunction("alloc".to_string()))?;
                let new_call = self
                    .builder
                    .build_call(alloc_fn, &[size_bytes.into()], "vec_realloc")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let new_raw_ptr = new_call
                    .try_as_basic_value()
                    .left()
                    .ok_or_else(|| {
                        CodegenError::InvalidOperation(
                            "alloc returned void when pointer expected".to_string(),
                        )
                    })?
                    .into_pointer_value();

                let new_data_ptr = self
                    .builder
                    .build_pointer_cast(new_raw_ptr, elem_ptr_ty, "vec_realloc_cast")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                // Copy existing elements
                let copy_idx_alloca =
                    self.create_entry_block_alloca("vec_copy_idx", i64_ty.into())?;
                self.builder
                    .build_store(copy_idx_alloca, zero_i64)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                let copy_cond_bb = self
                    .context
                    .append_basic_block(current_fn, "vec.push.copy.cond");
                let copy_body_bb = self
                    .context
                    .append_basic_block(current_fn, "vec.push.copy.body");
                let copy_inc_bb = self
                    .context
                    .append_basic_block(current_fn, "vec.push.copy.inc");
                let copy_end_bb = self
                    .context
                    .append_basic_block(current_fn, "vec.push.copy.end");

                self.builder
                    .build_unconditional_branch(copy_cond_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                self.builder.position_at_end(copy_cond_bb);
                let copy_idx = self
                    .builder
                    .build_load(i64_ty, copy_idx_alloca, "vec_copy_idx")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_int_value();
                let copy_cmp = self
                    .builder
                    .build_int_compare(IntPredicate::ULT, copy_idx, len_val_initial, "vec_copy_cmp")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.builder
                    .build_conditional_branch(copy_cmp, copy_body_bb, copy_end_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                self.builder.position_at_end(copy_body_bb);
                let src_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(i64_ty, data_ptr, &[copy_idx], "vec_copy_src")
                }
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let src_val = self
                    .builder
                    .build_load(i64_ty, src_ptr, "vec_copy_val")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let dst_ptr = unsafe {
                    self.builder.build_in_bounds_gep(
                        i64_ty,
                        new_data_ptr,
                        &[copy_idx],
                        "vec_copy_dst",
                    )
                }
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.builder
                    .build_store(dst_ptr, src_val)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(copy_inc_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                self.builder.position_at_end(copy_inc_bb);
                let next_idx = self
                    .builder
                    .build_int_add(copy_idx, i64_ty.const_int(1, false), "vec_copy_next")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.builder
                    .build_store(copy_idx_alloca, next_idx)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(copy_cond_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                self.builder.position_at_end(copy_end_bb);
                self.builder
                    .build_store(data_field_ptr, new_data_ptr)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.builder
                    .build_store(cap_ptr, adjusted_cap)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(push_cont_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                // Continue block after optional growth
                self.builder.position_at_end(push_cont_bb);
                let data_ptr_after = self
                    .builder
                    .build_load(elem_ptr_ty, data_field_ptr, "vec_data_after")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_pointer_value();
                let len_val = self
                    .builder
                    .build_load(i64_ty, len_ptr, "vec_len_after")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_int_value();

                let elem_ptr = unsafe {
                    self.builder.build_in_bounds_gep(
                        i64_ty,
                        data_ptr_after,
                        &[len_val],
                        "vec_push_slot",
                    )
                }
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.builder
                    .build_store(elem_ptr, elem_i64)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let len_next = self
                    .builder
                    .build_int_add(len_val, i64_ty.const_int(1, false), "vec_len_next")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.builder
                    .build_store(len_ptr, len_next)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                Ok(Some(zero_i64.into()))
            }
            "get" if arguments.len() == 1 => {
                let enum_ty = self.enum_struct.ok_or_else(|| {
                    CodegenError::InvalidOperation(
                        "Optional enum representation unavailable".to_string(),
                    )
                })?;

                let idx_raw = self.generate_expression(&arguments[0].value)?;
                let idx_i64 = self.cast_to_int(idx_raw, i64_ty)?;

                let len_val = self
                    .builder
                    .build_load(i64_ty, len_ptr, "vec_len")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_int_value();

                let result_alloca =
                    self.create_entry_block_alloca("vec_get_res", enum_ty.into())?;

                let idx_ge_len = self
                    .builder
                    .build_int_compare(IntPredicate::UGE, idx_i64, len_val, "vec_get_oob")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                let current_fn = self.current_function.ok_or_else(|| {
                    CodegenError::CompilationError(
                        "vector method called outside of function".to_string(),
                    )
                })?;
                let none_bb = self.context.append_basic_block(current_fn, "vec.get.none");
                let some_bb = self.context.append_basic_block(current_fn, "vec.get.some");
                let merge_bb = self.context.append_basic_block(current_fn, "vec.get.merge");

                self.builder
                    .build_conditional_branch(idx_ge_len, none_bb, some_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                self.builder.position_at_end(none_bb);
                let none_struct = enum_ty.const_named_struct(&[zero_i64.into(), zero_i64.into()]);
                self.builder
                    .build_store(result_alloca, none_struct)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(merge_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let _none_end_bb = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(some_bb);
                let elem_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(i64_ty, data_ptr, &[idx_i64], "vec_get_ptr")
                }
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let elem_val = self
                    .builder
                    .build_load(i64_ty, elem_ptr, "vec_get_val")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_int_value();
                let tagged = self
                    .builder
                    .build_insert_value(
                        enum_ty.get_undef(),
                        i64_ty.const_int(1, false),
                        0,
                        "vec_get_tag",
                    )
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();
                let with_payload = self
                    .builder
                    .build_insert_value(tagged, elem_val, 1, "vec_get_payload")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();
                self.builder
                    .build_store(result_alloca, with_payload)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                self.builder
                    .build_unconditional_branch(merge_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let _some_end_bb = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(merge_bb);
                let loaded = self
                    .builder
                    .build_load(enum_ty, result_alloca, "vec_get_result")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                Ok(Some(loaded))
            }
            _ => Ok(None),
        }
    }

    fn int_type_for_width(&self, bits: u32) -> IntType<'ctx> {
        if bits <= 8 {
            self.context.i8_type()
        } else if bits <= 16 {
            self.context.i16_type()
        } else if bits <= 32 {
            self.context.i32_type()
        } else {
            self.context.i64_type()
        }
    }

    fn is_unsigned_type(&self, ty: &crate::ast::Type) -> bool {
        match ty {
            crate::ast::Type::Identifier { name, .. } => {
                UNSIGNED_TYPE_NAMES.iter().any(|t| *t == name.as_str())
            }
            _ => false,
        }
    }

    fn record_unsigned_binding(&mut self, name: &str, ty: &crate::ast::Type) {
        if self.is_unsigned_type(ty) {
            self.unsigned_variables.insert(name.to_string());
        } else {
            self.unsigned_variables.remove(name);
        }
    }

    fn owned_type_of(&self, name: &str) -> Option<crate::ast::Type> {
        self.local_types
            .get(name)
            .cloned()
            .or_else(|| self.semantic.get_variable_type(name).cloned())
    }

    fn default_value_for_type(&self, ty: BasicTypeEnum<'ctx>) -> BasicValueEnum<'ctx> {
        match ty {
            BasicTypeEnum::IntType(it) => it.const_zero().into(),
            BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
            BasicTypeEnum::PointerType(pt) => pt.const_zero().into(),
            BasicTypeEnum::StructType(st) => st.const_zero().as_basic_value_enum(),
            BasicTypeEnum::ArrayType(at) => at.const_zero().as_basic_value_enum(),
            BasicTypeEnum::VectorType(vt) => vt.const_zero().as_basic_value_enum(),
            BasicTypeEnum::ScalableVectorType(svt) => svt.const_zero().as_basic_value_enum(),
        }
    }

    fn drop_target_type_name(&self, ty: &crate::ast::Type) -> Option<String> {
        use crate::ast::Type as AstType;
        match ty {
            AstType::Identifier { name, .. } => Some(name.clone()),
            AstType::RawPointer { pointee, is_raw } => {
                if *is_raw {
                    None
                } else {
                    self.drop_target_type_name(pointee)
                }
            }
            AstType::Optional { inner } | AstType::Result { inner } => {
                self.drop_target_type_name(inner)
            }
            AstType::Pointer { .. } | AstType::Reference { .. } => None,
            _ => None,
        }
    }

    fn drop_function_for_type(&self, ty: &crate::ast::Type) -> Option<String> {
        let type_name = self.drop_target_type_name(ty)?;
        let fn_name = format!("Drop_{}_drop", type_name);
        if self.functions.contains_key(&fn_name) {
            Some(fn_name)
        } else {
            None
        }
    }

    fn type_is_owned(&self, ty: &crate::ast::Type) -> bool {
        self.drop_function_for_type(ty).is_some()
    }

    fn track_owned_binding(&mut self, name: &str) {
        if self.current_function.is_none() {
            return;
        }
        if let Some(current_fn) = self.current_function {
            let fname = current_fn.get_name().to_string_lossy();
            if fname.contains("_drop") {
                return;
            }
        }
        if self.owned_locals.iter().any(|n| n == name) {
            return;
        }
        if let Some(ty) = self.owned_type_of(name) {
            if self.type_is_owned(&ty) {
                self.owned_locals.push(name.to_string());
            }
        }
    }

    fn unregister_owned(&mut self, name: &str) {
        if let Some(pos) = self.owned_locals.iter().rposition(|n| n == name) {
            self.owned_locals.remove(pos);
        }
    }

    fn emit_drop_for_variable(&mut self, name: &str) -> Result<(), CodegenError> {
        if self.current_function.is_none() {
            return Ok(());
        }
        let ty = match self.owned_type_of(name) {
            Some(t) => t,
            None => return Ok(()),
        };
        let drop_fn_name = match self.drop_function_for_type(&ty) {
            Some(n) => n,
            None => return Ok(()),
        };
        let drop_fn = match self.functions.get(&drop_fn_name).cloned() {
            Some(f) => f,
            None => return Ok(()),
        };
        let (ptr, _stored_ty) = match self.variables.get(name) {
            Some((alloca, ty)) => (*alloca, *ty),
            None => return Ok(()),
        };
        let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
        for (idx, param_ty) in drop_fn.get_type().get_param_types().iter().enumerate() {
            match param_ty {
                inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => {
                    if idx == 0 {
                        let casted = self
                            .builder
                            .build_pointer_cast(ptr, *pt, &format!("{}_drop_ptrcast_{}", name, idx))
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        call_args.push(casted.into());
                    } else {
                        call_args.push(pt.const_zero().into());
                    }
                }
                inkwell::types::BasicMetadataTypeEnum::IntType(it) => {
                    call_args.push(it.const_zero().into());
                }
                inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => {
                    call_args.push(ft.const_zero().into());
                }
                _ => {
                    call_args.push(self.context.i64_type().const_zero().into());
                }
            }
        }
        let _ = self
            .builder
            .build_call(drop_fn, &call_args, &format!("drop_call_{}", name))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        Ok(())
    }

    fn drop_all_owned_locals(&mut self) -> Result<(), CodegenError> {
        while let Some(name) = self.owned_locals.pop() {
            self.emit_drop_for_variable(&name)?;
            self.local_types.remove(&name);
            self.variables.remove(&name);
        }
        Ok(())
    }

    fn drop_current_value(&mut self, name: &str) -> Result<(), CodegenError> {
        if !self.owned_locals.iter().any(|n| n == name) {
            return Ok(());
        }
        self.emit_drop_for_variable(name)
    }

    fn mark_expr_moved(&mut self, expr: &crate::ast::Expression) {
        use crate::ast::Expression as AstExpr;
        match expr {
            AstExpr::Identifier(name) => self.unregister_owned(name),
            AstExpr::Tuple(elements) => {
                for element in elements {
                    self.mark_expr_moved(element);
                }
            }
            _ => {}
        }
    }

    fn bind_identifier_value(
        &mut self,
        name: &str,
        value: BasicValueEnum<'ctx>,
        annotation: Option<&crate::ast::Type>,
    ) -> Result<(), CodegenError> {
        let ast_ty_owned = if let Some(ann) = annotation {
            Some(ann.clone())
        } else {
            self.semantic.get_variable_type(name).cloned()
        };

        let target_ty = ast_ty_owned
            .as_ref()
            .and_then(|ty| self.map_ast_type(ty))
            .unwrap_or_else(|| value.get_type());

        let mut stored_value = value;
        if stored_value.get_type() != target_ty {
            stored_value = self.cast_basic_to_type(stored_value, target_ty)?;
        }

        let alloca = self.create_entry_block_alloca(name, target_ty)?;
        self.builder
            .build_store(alloca, stored_value)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.variables.insert(name.to_string(), (alloca, target_ty));

        if let Some(ast_ty) = ast_ty_owned {
            self.local_types.insert(name.to_string(), ast_ty.clone());
            self.record_unsigned_binding(name, &ast_ty);
            self.track_owned_binding(name);
        }

        Ok(())
    }

    fn materialize_struct_value(
        &self,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(StructValue<'ctx>, StructType<'ctx>), CodegenError> {
        if value.is_struct_value() {
            let struct_value = value.into_struct_value();
            let struct_ty = struct_value.get_type();
            Ok((struct_value, struct_ty))
        } else {
            Err(CodegenError::InvalidOperation(
                "tuple destructuring requires struct value".to_string(),
            ))
        }
    }

    fn bind_pattern_value(
        &mut self,
        pattern: &BindingPattern,
        value: BasicValueEnum<'ctx>,
        annotation: Option<&crate::ast::Type>,
    ) -> Result<(), CodegenError> {
        match pattern {
            BindingPattern::Identifier(name) => self.bind_identifier_value(name, value, annotation),
            BindingPattern::Discard => Ok(()),
            BindingPattern::Tuple(elements) => {
                let tuple_value = if value.is_pointer_value() {
                    let ptr_val = value.into_pointer_value();
                    let load_ty: BasicTypeEnum<'ctx> = match annotation {
                        Some(crate::ast::Type::Tuple(types)) => {
                            let element_types: Vec<BasicTypeEnum<'ctx>> = types
                                .iter()
                                .map(|ty| {
                                    self.map_ast_type(ty)
                                        .unwrap_or_else(|| self.context.i64_type().into())
                                })
                                .collect();
                            if element_types.is_empty() {
                                self.context.struct_type(&[], false).as_basic_type_enum()
                            } else {
                                self.context
                                    .struct_type(&element_types, false)
                                    .as_basic_type_enum()
                            }
                        }
                        _ => {
                            return Err(CodegenError::InvalidOperation(
                                "tuple pointer binding requires tuple annotation".to_string(),
                            ));
                        }
                    };
                    self.builder
                        .build_load(load_ty, ptr_val, "tuple_load")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                } else {
                    value
                };

                let (struct_value, struct_ty) = self.materialize_struct_value(tuple_value)?;
                if elements.len() != struct_ty.count_fields() as usize {
                    return Err(CodegenError::InvalidOperation(
                        "tuple binding arity mismatch".to_string(),
                    ));
                }

                let tuple_annotations: Option<&[crate::ast::Type]> = match annotation {
                    Some(crate::ast::Type::Tuple(types)) => Some(types.as_slice()),
                    _ => None,
                };

                for (idx, element_pattern) in elements.iter().enumerate() {
                    let field_value = self
                        .builder
                        .build_extract_value(
                            struct_value,
                            idx as u32,
                            &format!("tuple_elem{}", idx),
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    let child_annotation = tuple_annotations.and_then(|types| types.get(idx));
                    self.bind_pattern_value(element_pattern, field_value, child_annotation)?;
                }

                Ok(())
            }
        }
    }

    fn codegen_variable_decl_identifier(
        &mut self,
        name: &str,
        type_annotation: Option<crate::ast::Type>,
        value: &Expression,
    ) -> Result<(), CodegenError> {
        if let Expression::Call {
            function,
            type_args: _,
            arguments: bind_args,
        } = value
        {
            if let Expression::FieldAccess { object, field } = function.as_ref() {
                if field == "bind" {
                    if let Expression::Identifier(base_fn_name) = object.as_ref() {
                        if let Some(base_fn) = self.functions.get(base_fn_name).cloned() {
                            let base_param_metas = base_fn.get_type().get_param_types();
                            let total_params = base_param_metas.len();
                            let bound_n = bind_args.len();
                            let unbound_n = if total_params >= bound_n {
                                total_params - bound_n
                            } else {
                                0
                            };
                            let wrapper_param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> =
                                base_param_metas.iter().take(unbound_n).cloned().collect();
                            let wrapper_ty =
                                self.context.i64_type().fn_type(&wrapper_param_meta, false);
                            let wrapper_fn = self.module.add_function(name, wrapper_ty, None);
                            self.functions.insert(name.to_string(), wrapper_fn);

                            let prev_insert_block = self.builder.get_insert_block();
                            let entry = self.context.append_basic_block(wrapper_fn, "entry");
                            let prev_fn = self.current_function;
                            let prev_vars = std::mem::take(&mut self.variables);
                            let prev_owned = std::mem::take(&mut self.owned_locals);
                            self.current_function = Some(wrapper_fn);
                            self.builder.position_at_end(entry);

                            for (i, param) in wrapper_fn.get_param_iter().enumerate() {
                                let p_ty: BasicTypeEnum = match base_param_metas.get(i).cloned() {
                                    Some(inkwell::types::BasicMetadataTypeEnum::IntType(it)) => {
                                        it.into()
                                    }
                                    Some(inkwell::types::BasicMetadataTypeEnum::FloatType(ft)) => {
                                        ft.into()
                                    }
                                    Some(inkwell::types::BasicMetadataTypeEnum::PointerType(
                                        pt,
                                    )) => pt.into(),
                                    Some(inkwell::types::BasicMetadataTypeEnum::StructType(st)) => {
                                        st.as_basic_type_enum()
                                    }
                                    Some(inkwell::types::BasicMetadataTypeEnum::VectorType(vt)) => {
                                        vt.as_basic_type_enum()
                                    }
                                    Some(inkwell::types::BasicMetadataTypeEnum::ArrayType(at)) => {
                                        at.as_basic_type_enum()
                                    }
                                    _ => self.context.i64_type().into(),
                                };
                                let alloca =
                                    self.create_entry_block_alloca(&format!("arg{}", i), p_ty)?;
                                self.builder
                                    .build_store(alloca, param)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.variables.insert(format!("arg{}", i), (alloca, p_ty));
                            }

                            let mut call_args: Vec<BasicMetadataValueEnum> = Vec::new();
                            for i in 0..unbound_n {
                                let (alloca, stored_ty) = self
                                    .variables
                                    .get(&format!("arg{}", i))
                                    .copied()
                                    .ok_or_else(|| {
                                        CodegenError::CompilationError(
                                            "missing wrapper argument".to_string(),
                                        )
                                    })?;
                                let loaded = self
                                    .builder
                                    .build_load(stored_ty, alloca, &format!("arg{}", i))
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                call_args.push(loaded.into());
                            }
                            for arg in bind_args {
                                let arg_value = self.generate_expression(&arg.value)?;
                                call_args.push(arg_value.into());
                            }

                            let call_res = self
                                .builder
                                .build_call(base_fn, &call_args, "calltmp")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let ret_val: BasicValueEnum<'ctx> =
                                match call_res.try_as_basic_value().left() {
                                    Some(bv) => {
                                        let iv = self.cast_to_int(bv, self.context.i64_type())?;
                                        iv.into()
                                    }
                                    None => self.context.i64_type().const_zero().into(),
                                };
                            self.builder
                                .build_return(Some(&ret_val))
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                            self.variables = prev_vars;
                            self.owned_locals = prev_owned;
                            self.current_function = prev_fn;
                            if let Some(bb) = prev_insert_block {
                                self.builder.position_at_end(bb);
                            }
                            return Ok(());
                        }
                    }
                }
            }
        }

        let annotation_ref = type_annotation.as_ref();

        if let (
            Some(crate::ast::Type::Identifier {
                name: struct_name,
                type_args: _,
            }),
            Expression::StructLiteral {
                type_name: _,
                fields,
            },
        ) = (annotation_ref, value)
        {
            if let Some((st, order)) = self.struct_types.get(struct_name) {
                let struct_ty = *st;
                let field_order = order.clone();
                let sval =
                    self.build_struct_literal_value(struct_name, fields, struct_ty, &field_order)?;
                self.bind_identifier_value(name, sval, annotation_ref)?;
                return Ok(());
            }
        }

        if let Expression::Matrix { rows } = value {
            let rank = if rows.len() <= 1 { 1 } else { 2 };
            self.matrix_rank.insert(name.to_string(), rank);
            if rank == 1 {
                let len = rows.first().map(|r| r.len()).unwrap_or(0) as u64;
                self.vector_lengths.insert(name.to_string(), len);
            }
        }

        let value_result = self.generate_expression(value)?;
        self.bind_identifier_value(name, value_result, annotation_ref)?;

        Ok(())
    }

    fn expression_is_unsigned(&self, expr: &Expression) -> bool {
        match expr {
            Expression::Identifier(name) => {
                if self.unsigned_variables.contains(name) {
                    true
                } else {
                    self.semantic
                        .get_variable_type(name)
                        .map(|ty| self.is_unsigned_type(ty))
                        .unwrap_or(false)
                }
            }
            Expression::Literal(Literal::Integer(int_lit)) => matches!(
                int_lit.suffix,
                Some(crate::ast::IntSuffix::U8)
                    | Some(crate::ast::IntSuffix::U16)
                    | Some(crate::ast::IntSuffix::U32)
                    | Some(crate::ast::IntSuffix::U64)
            ),
            Expression::BinaryOp { left, right, .. } => {
                self.expression_is_unsigned(left) && self.expression_is_unsigned(right)
            }
            Expression::FieldAccess { object, .. } => self.expression_is_unsigned(object),
            _ => false,
        }
    }

    // Unify two integers to a common width (use i64) and return both cast plus chosen type
    fn unify_ints(
        &self,
        l: BasicValueEnum<'ctx>,
        r: BasicValueEnum<'ctx>,
    ) -> Result<
        (
            inkwell::values::IntValue<'ctx>,
            inkwell::values::IntValue<'ctx>,
            IntType<'ctx>,
        ),
        CodegenError,
    > {
        let l_bits = match l {
            BasicValueEnum::IntValue(iv) => iv.get_type().get_bit_width(),
            _ => 64,
        };
        let r_bits = match r {
            BasicValueEnum::IntValue(iv) => iv.get_type().get_bit_width(),
            _ => 64,
        };
        let target_bits = l_bits.max(r_bits).max(8);
        let ty = self.int_type_for_width(target_bits);
        let li = self.cast_to_int(l, ty)?;
        let ri = self.cast_to_int(r, ty)?;
        Ok((li, ri, ty))
    }

    #[allow(dead_code)]
    fn get_int_type_of(&self, v: BasicValueEnum<'ctx>) -> Option<IntType<'ctx>> {
        if v.is_int_value() {
            Some(v.into_int_value().get_type())
        } else {
            None
        }
    }

    // Resolve the semantic struct name for a variable (peels pointers/optionals/results)
    fn semantic_struct_name_of_var(&self, var_name: &str) -> Option<String> {
        use crate::ast::Type as AstType;
        // First prefer semantic types (covers globals and some params)
        fn peel<'a>(t: &'a AstType) -> &'a AstType {
            match t {
                AstType::Pointer { pointee, .. } => peel(pointee),
                AstType::RawPointer { pointee, .. } => peel(pointee),
                AstType::Optional { inner } => peel(inner),
                AstType::Result { inner } => peel(inner),
                other => other,
            }
        }
        if let Some(t) = self.semantic.get_variable_type(var_name) {
            if let AstType::Identifier { name, type_args: _ } = peel(t) {
                return Some(name.clone());
            }
        }
        // Fallback: inspect current codegen variable table for a struct-typed local
        if let Some((_, bty)) = self.variables.get(var_name) {
            if let BasicTypeEnum::StructType(st) = bty {
                if let Some((name, (_llvm_st, _))) = self
                    .struct_types
                    .iter()
                    .find(|(_, (llvm_st, _))| llvm_st == st)
                {
                    return Some(name.clone());
                }
            }
        }
        None
    }

    fn cast_to_int(
        &self,
        value: BasicValueEnum<'ctx>,
        int_ty: IntType<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        if value.is_int_value() {
            let iv = value.into_int_value();
            if iv.get_type() == int_ty {
                Ok(iv)
            } else {
                self.builder
                    .build_int_cast(iv, int_ty, "icast")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))
            }
        } else if value.is_float_value() {
            self.builder
                .build_float_to_signed_int(value.into_float_value(), int_ty, "ftosi")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))
        } else if value.is_pointer_value() {
            self.builder
                .build_ptr_to_int(value.into_pointer_value(), int_ty, "ptoi")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))
        } else {
            Ok(int_ty.const_zero())
        }
    }

    fn cast_to_float(
        &self,
        value: BasicValueEnum<'ctx>,
        float_ty: FloatType<'ctx>,
    ) -> Result<inkwell::values::FloatValue<'ctx>, CodegenError> {
        if value.is_float_value() {
            let fv = value.into_float_value();
            // If types differ (f32 vs f64), convert
            if fv.get_type() == float_ty {
                Ok(fv)
            } else {
                self.builder
                    .build_float_cast(fv, float_ty, "fcast")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))
            }
        } else if value.is_int_value() {
            self.builder
                .build_signed_int_to_float(value.into_int_value(), float_ty, "sitofp")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))
        } else {
            Ok(float_ty.const_zero())
        }
    }

    fn cast_to_ptr(
        &self,
        value: BasicValueEnum<'ctx>,
        ptr_ty: inkwell::types::PointerType<'ctx>,
    ) -> Result<inkwell::values::PointerValue<'ctx>, CodegenError> {
        if value.is_pointer_value() {
            let pv = value.into_pointer_value();
            if pv.get_type() == ptr_ty {
                Ok(pv)
            } else {
                Ok(self
                    .builder
                    .build_pointer_cast(pv, ptr_ty, "pcast")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?)
            }
        } else if value.is_int_value() {
            self.builder
                .build_int_to_ptr(value.into_int_value(), ptr_ty, "itop")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))
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

        let i8_ptr_ptr_type = i8_ptr_type.ptr_type(AddressSpace::default());

        if self.command_line_argc_global.is_none() {
            let argc_global = self
                .module
                .add_global(self.context.i32_type(), None, "tricti_argc");
            argc_global.set_initializer(&self.context.i32_type().const_zero());
            self.command_line_argc_global = Some(argc_global);
        }

        if self.command_line_argv_global.is_none() {
            let argv_global = self.module.add_global(i8_ptr_ptr_type, None, "tricti_argv");
            argv_global.set_initializer(&i8_ptr_ptr_type.const_null());
            self.command_line_argv_global = Some(argv_global);
        }

        // Buffered formatting helper
        let snprintf_ty = i32_type.fn_type(
            &[
                i8_ptr_type.into(),
                self.context.i64_type().into(),
                i8_ptr_type.into(),
            ],
            true,
        );
        let snprintf_fn = self.module.add_function("snprintf", snprintf_ty, None);
        self.functions.insert("snprintf".to_string(), snprintf_fn);

        // Basic allocator hooks via libc malloc/free
        let malloc_ty = i8_ptr_type.fn_type(&[self.context.i64_type().into()], false);
        let malloc_fn = self.module.add_function("malloc", malloc_ty, None);
        // Expose as `alloc` to tricti source
        self.functions.insert("alloc".to_string(), malloc_fn);

        let free_ty = self
            .context
            .void_type()
            .fn_type(&[i8_ptr_type.into()], false);
        let free_fn = self.module.add_function("free", free_ty, None);
        self.functions.insert("dealloc".to_string(), free_fn);

        // memcpy for raw byte copies (used by String intrinsics)
        let memcpy_ty = i8_ptr_type.fn_type(
            &[
                i8_ptr_type.into(),
                i8_ptr_type.into(),
                self.context.i64_type().into(),
            ],
            false,
        );
        let memcpy_fn = self.module.add_function("memcpy", memcpy_ty, None);
        self.functions.insert("memcpy".to_string(), memcpy_fn);

        // Process exit (libc)
        let exit_ty = self.context.void_type().fn_type(&[i32_type.into()], false);
        let exit_fn = self
            .module
            .add_function("exit", exit_ty, Some(Linkage::External));
        self.functions.insert("exit".to_string(), exit_fn);

        // strlen for string length (bytes); expose as `len`
        let strlen_ty = self
            .context
            .i64_type()
            .fn_type(&[i8_ptr_type.into()], false);
        let strlen_fn = self.module.add_function("strlen", strlen_ty, None);
        self.functions.insert("len".to_string(), strlen_fn);

        let access_ty = i32_type.fn_type(&[i8_ptr_type.into(), i32_type.into()], false);
        let access_fn = self.module.add_function("access", access_ty, None);
        self.functions
            .insert("__libc_access".to_string(), access_fn);

        let mkdir_ty = i32_type.fn_type(&[i8_ptr_type.into(), i32_type.into()], false);
        let mkdir_fn = self.module.add_function("mkdir", mkdir_ty, None);
        self.functions.insert("__libc_mkdir".to_string(), mkdir_fn);

        let unlink_ty = i32_type.fn_type(&[i8_ptr_type.into()], false);
        let unlink_fn = self.module.add_function("unlink", unlink_ty, None);
        self.functions
            .insert("__libc_unlink".to_string(), unlink_fn);

        let rename_ty = i32_type.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        let rename_fn = self.module.add_function("rename", rename_ty, None);
        self.functions
            .insert("__libc_rename".to_string(), rename_fn);

        // Character classification helper
        let isspace_ty = i32_type.fn_type(&[i32_type.into()], false);
        let isspace_fn = self.module.add_function("isspace", isspace_ty, None);
        self.functions.insert("isspace".to_string(), isspace_fn);

        // strcmp for string equality; wrap into streq(a: string, b: string) -> bool
        let strcmp_ty = i32_type.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        let strcmp_fn = self.module.add_function("strcmp", strcmp_ty, None);
        // Define streq wrapper: i1 streq(i8*, i8*) { %c = call i32 @strcmp(a,b); %z = icmp eq i32 %c, 0; ret i1 %z }
        let bool_ty = self.context.bool_type();
        let streq_ty = bool_ty.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        let streq_fn = self.module.add_function("streq", streq_ty, None);
        // Build body
        let prev_bb = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(streq_fn, "entry");
        self.builder.position_at_end(entry);
        let a = streq_fn.get_nth_param(0).unwrap();
        let b = streq_fn.get_nth_param(1).unwrap();
        let call = self
            .builder
            .build_call(strcmp_fn, &[a.into(), b.into()], "strcmp_call")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let rv = call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strcmp returned void?".to_string()))?
            .into_int_value();
        let eqz = self
            .builder
            .build_int_compare(IntPredicate::EQ, rv, i32_type.const_zero(), "eqz")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&eqz))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb {
            self.builder.position_at_end(bb);
        }
        self.functions.insert("streq".to_string(), streq_fn);

        // strstr for substring search; wrap into contains(haystack: string, needle: string) -> bool
        let strstr_ty = i8_ptr_type.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        let strstr_fn = self.module.add_function("strstr", strstr_ty, None);
        let contains_ty = bool_ty.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        let contains_fn = self.module.add_function("contains", contains_ty, None);
        let prev_bb2 = self.builder.get_insert_block();
        let entry2 = self.context.append_basic_block(contains_fn, "entry");
        self.builder.position_at_end(entry2);
        let ha = contains_fn.get_nth_param(0).unwrap();
        let ne = contains_fn.get_nth_param(1).unwrap();
        let call2 = self
            .builder
            .build_call(strstr_fn, &[ha.into(), ne.into()], "strstr_call")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        // strstr returns null if not found
        let rv2 = call2
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strstr returned void?".to_string()))?
            .into_pointer_value();
        let is_null = self
            .builder
            .build_is_null(rv2, "isnull")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        // contains = not is_null
        let contains_val = self
            .builder
            .build_not(is_null, "notnull")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&contains_val))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb2 {
            self.builder.position_at_end(bb);
        }
        self.functions.insert("contains".to_string(), contains_fn);

        // starts_with: strncmp(hay, pre, len(pre)) == 0
        let size_t_ty = self.context.i64_type();
        let strncmp_ty = i32_type.fn_type(
            &[i8_ptr_type.into(), i8_ptr_type.into(), size_t_ty.into()],
            false,
        );
        let strncmp_fn = self.module.add_function("strncmp", strncmp_ty, None);
        let starts_ty = bool_ty.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        let starts_fn = self.module.add_function("starts_with", starts_ty, None);
        let prev_bb3 = self.builder.get_insert_block();
        let entry3 = self.context.append_basic_block(starts_fn, "entry");
        self.builder.position_at_end(entry3);
        let hay = starts_fn.get_nth_param(0).unwrap();
        let pre = starts_fn.get_nth_param(1).unwrap();
        // n = strlen(pre)
        let call_len_pre = self
            .builder
            .build_call(*self.functions.get("len").unwrap(), &[pre.into()], "lenpre")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let n_val = call_len_pre
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strlen returned void?".to_string()))?
            .into_int_value();
        let cmp = self
            .builder
            .build_call(
                strncmp_fn,
                &[hay.into(), pre.into(), n_val.into()],
                "strncmp_call",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let rv_i32 = cmp
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strncmp returned void?".to_string()))?
            .into_int_value();
        let eqz2 = self
            .builder
            .build_int_compare(IntPredicate::EQ, rv_i32, i32_type.const_zero(), "eqz")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&eqz2))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb3 {
            self.builder.position_at_end(bb);
        }
        self.functions.insert("starts_with".to_string(), starts_fn);

        // ends_with: compare suffix of hay with suf using lengths
        let ends_ty = bool_ty.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        let ends_fn = self.module.add_function("ends_with", ends_ty, None);
        let prev_bb4 = self.builder.get_insert_block();
        let entry4 = self.context.append_basic_block(ends_fn, "entry");
        self.builder.position_at_end(entry4);
        let hay2 = ends_fn.get_nth_param(0).unwrap();
        let suf = ends_fn.get_nth_param(1).unwrap();
        // lh = strlen(hay); ls = strlen(suf)
        let lh_call = self
            .builder
            .build_call(
                *self.functions.get("len").unwrap(),
                &[hay2.into()],
                "lenhay",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let ls_call = self
            .builder
            .build_call(*self.functions.get("len").unwrap(), &[suf.into()], "lensuf")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let lh = lh_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strlen returned void?".to_string()))?
            .into_int_value();
        let ls = ls_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strlen returned void?".to_string()))?
            .into_int_value();
        // if ls > lh: return false
        let gt = self
            .builder
            .build_int_compare(IntPredicate::UGT, ls, lh, "ls_gt_lh")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        // Create blocks
        let parent = ends_fn;
        let then_bb = self.context.append_basic_block(parent, "then");
        let cont_bb = self.context.append_basic_block(parent, "cont");
        self.builder
            .build_conditional_branch(gt, then_bb, cont_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        // then: return false
        self.builder.position_at_end(then_bb);
        let f = bool_ty.const_int(0, false);
        self.builder
            .build_return(Some(&f))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        // cont: compare last ls bytes: strncmp(hay + (lh-ls), suf, ls) == 0
        self.builder.position_at_end(cont_bb);
        let diff = self
            .builder
            .build_int_sub(lh, ls, "lh_minus_ls")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let hay_ptr = hay2.into_pointer_value();
        let i8_ty = self.context.i8_type();
        let off = unsafe {
            self.builder
                .build_in_bounds_gep(i8_ty, hay_ptr, &[diff], "suffix_ptr")
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let cmp2 = self
            .builder
            .build_call(
                strncmp_fn,
                &[off.into(), suf.into(), ls.into()],
                "strncmp_suf",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let rv2 = cmp2
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strncmp returned void?".to_string()))?
            .into_int_value();
        let eqz3 = self
            .builder
            .build_int_compare(IntPredicate::EQ, rv2, i32_type.const_zero(), "eqz")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&eqz3))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb4 {
            self.builder.position_at_end(bb);
        }
        self.functions.insert("ends_with".to_string(), ends_fn);

        // find: return index of first occurrence or -1 if not found
        let find_ty = self
            .context
            .i64_type()
            .fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        let find_fn = self.module.add_function("find", find_ty, None);
        let prev_bb5 = self.builder.get_insert_block();
        let entry5 = self.context.append_basic_block(find_fn, "entry");
        self.builder.position_at_end(entry5);
        let hayf = find_fn.get_nth_param(0).unwrap();
        let nef = find_fn.get_nth_param(1).unwrap();
        let p = self
            .builder
            .build_call(strstr_fn, &[hayf.into(), nef.into()], "strstr_find")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let pv = p
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strstr returned void?".to_string()))?
            .into_pointer_value();
        // if null -> -1
        let is_null2 = self
            .builder
            .build_is_null(pv, "isnull")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let parentf = find_fn;
        let thenf = self.context.append_basic_block(parentf, "then");
        let contf = self.context.append_basic_block(parentf, "cont");
        self.builder
            .build_conditional_branch(is_null2, thenf, contf)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        // then: ret -1
        self.builder.position_at_end(thenf);
        let minus1 = self.context.i64_type().const_int(u64::MAX, true);
        self.builder
            .build_return(Some(&minus1))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        // cont: ptrdiff = (pv - hayf) as i64
        self.builder.position_at_end(contf);
        let haypv = hayf.into_pointer_value();
        // Compute difference by casting to intptr
        let intptr = self.context.i64_type();
        let pv_i = self
            .builder
            .build_ptr_to_int(pv, intptr, "pv_i")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let hay_i = self
            .builder
            .build_ptr_to_int(haypv, intptr, "hay_i")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let diff_i = self
            .builder
            .build_int_sub(pv_i, hay_i, "diff_i")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let idx64 = self
            .builder
            .build_int_cast(diff_i, self.context.i64_type(), "idx64")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&idx64))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb5 {
            self.builder.position_at_end(bb);
        }
        self.functions.insert("find".to_string(), find_fn);

        // === String helpers for SKIP_STDLIB mode ===
        // String_equals(&String, &String) -> bool (accepts pointers to pointers)
        let string_ptr_ty = i8_ptr_type.ptr_type(AddressSpace::default());
        let string_equals_ty =
            bool_ty.fn_type(&[string_ptr_ty.into(), string_ptr_ty.into()], false);
        let string_equals_fn = self
            .module
            .add_function("String_equals", string_equals_ty, None);
        self.functions
            .insert("String_equals".to_string(), string_equals_fn);
        let prev_bb6 = self.builder.get_insert_block();
        let entry6 = self.context.append_basic_block(string_equals_fn, "entry");
        self.builder.position_at_end(entry6);
        let lhs_ptr_ptr = string_equals_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let rhs_ptr_ptr = string_equals_fn
            .get_nth_param(1)
            .unwrap()
            .into_pointer_value();
        let lhs = self
            .builder
            .build_load(i8_ptr_type, lhs_ptr_ptr, "lhs")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let rhs = self
            .builder
            .build_load(i8_ptr_type, rhs_ptr_ptr, "rhs")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let lhs_is_null = self
            .builder
            .build_is_null(lhs, "lhs_is_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let rhs_is_null = self
            .builder
            .build_is_null(rhs, "rhs_is_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let both_null = self
            .builder
            .build_and(lhs_is_null, rhs_is_null, "both_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let parent_eq = string_equals_fn;
        let both_bb = self.context.append_basic_block(parent_eq, "both_null");
        let else_bb = self.context.append_basic_block(parent_eq, "not_both");
        self.builder
            .build_conditional_branch(both_null, both_bb, else_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        // both null -> true
        self.builder.position_at_end(both_bb);
        self.builder
            .build_return(Some(&bool_ty.const_int(1, false)))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        // else: if either null -> false, else compare
        self.builder.position_at_end(else_bb);
        let either_null = self
            .builder
            .build_or(lhs_is_null, rhs_is_null, "either_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let null_bb = self.context.append_basic_block(parent_eq, "null_case");
        let cmp_bb = self.context.append_basic_block(parent_eq, "cmp");
        self.builder
            .build_conditional_branch(either_null, null_bb, cmp_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        // null case -> false
        self.builder.position_at_end(null_bb);
        self.builder
            .build_return(Some(&bool_ty.const_int(0, false)))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        // compare via streq
        self.builder.position_at_end(cmp_bb);
        let call_eq = self
            .builder
            .build_call(streq_fn, &[lhs.into(), rhs.into()], "call_streq")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let cmp_res = call_eq
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("streq returned void".to_string()))?
            .into_int_value();
        self.builder
            .build_return(Some(&cmp_res))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb6 {
            self.builder.position_at_end(bb);
        }

        // String_from_cstr(*u8) -> String (internally i8*)
        let string_from_cstr_ty = i8_ptr_type.fn_type(&[i8_ptr_type.into()], false);
        let string_from_cstr_fn =
            self.module
                .add_function("String_from_cstr", string_from_cstr_ty, None);
        self.functions
            .insert("String_from_cstr".to_string(), string_from_cstr_fn);
        let prev_bb7 = self.builder.get_insert_block();
        let entry7 = self
            .context
            .append_basic_block(string_from_cstr_fn, "entry");
        self.builder.position_at_end(entry7);
        let src_param = string_from_cstr_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let null_ptr = i8_ptr_type.const_zero();
        let src_is_null = self
            .builder
            .build_is_null(src_param, "src_is_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let null_block = self
            .context
            .append_basic_block(string_from_cstr_fn, "src_null");
        let copy_block = self.context.append_basic_block(string_from_cstr_fn, "copy");
        self.builder
            .build_conditional_branch(src_is_null, null_block, copy_block)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(null_block);
        self.builder
            .build_return(Some(&null_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(copy_block);
        let len_call = self
            .builder
            .build_call(strlen_fn, &[src_param.into()], "strlen_src")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let len_val = len_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strlen returned void".to_string()))?
            .into_int_value();
        let one = self.context.i64_type().const_int(1, false);
        let total_size = self
            .builder
            .build_int_add(len_val, one, "total_size")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dest_call = self
            .builder
            .build_call(malloc_fn, &[total_size.into()], "alloc_string")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dest_ptr = dest_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("malloc returned void".to_string()))?
            .into_pointer_value();
        let dest_is_null = self
            .builder
            .build_is_null(dest_ptr, "dest_is_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dest_fail_bb = self
            .context
            .append_basic_block(string_from_cstr_fn, "alloc_fail");
        let cont_bb = self.context.append_basic_block(string_from_cstr_fn, "cont");
        self.builder
            .build_conditional_branch(dest_is_null, dest_fail_bb, cont_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(dest_fail_bb);
        self.builder
            .build_return(Some(&null_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(cont_bb);
        self.builder
            .build_call(
                memcpy_fn,
                &[dest_ptr.into(), src_param.into(), total_size.into()],
                "memcpy_copy",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&dest_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb7 {
            self.builder.position_at_end(bb);
        }

        // String_new() -> String (allocates an empty null-terminated buffer)
        let string_new_ty = i8_ptr_type.fn_type(&[], false);
        let string_new_fn = self.module.add_function("String_new", string_new_ty, None);
        self.functions
            .insert("String_new".to_string(), string_new_fn);
        let prev_bb8 = self.builder.get_insert_block();
        let entry8 = self.context.append_basic_block(string_new_fn, "entry");
        self.builder.position_at_end(entry8);
        let one = self.context.i64_type().const_int(1, false);
        let alloc_call = self
            .builder
            .build_call(malloc_fn, &[one.into()], "string_new_alloc")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let new_ptr = alloc_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("malloc returned void".to_string()))?
            .into_pointer_value();
        let alloc_is_null = self
            .builder
            .build_is_null(new_ptr, "string_new_is_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let alloc_fail_bb = self.context.append_basic_block(string_new_fn, "alloc_fail");
        let alloc_ok_bb = self.context.append_basic_block(string_new_fn, "alloc_ok");
        self.builder
            .build_conditional_branch(alloc_is_null, alloc_fail_bb, alloc_ok_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(alloc_fail_bb);
        self.builder
            .build_return(Some(&i8_ptr_type.const_zero()))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(alloc_ok_bb);
        let zero_i8 = self.context.i8_type().const_zero();
        self.builder
            .build_store(new_ptr, zero_i8)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&new_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb8 {
            self.builder.position_at_end(bb);
        }

        // String_clone(&String) -> String
        let string_clone_ty = i8_ptr_type.fn_type(&[string_ptr_ty.into()], false);
        let string_clone_fn = self
            .module
            .add_function("String_clone", string_clone_ty, None);
        self.functions
            .insert("String_clone".to_string(), string_clone_fn);
        let prev_bb9 = self.builder.get_insert_block();
        let entry9 = self.context.append_basic_block(string_clone_fn, "entry");
        self.builder.position_at_end(entry9);
        let src_ptr_ptr = string_clone_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let src_ptr = self
            .builder
            .build_load(i8_ptr_type, src_ptr_ptr, "clone_src")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let src_is_null = self
            .builder
            .build_is_null(src_ptr, "clone_src_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let clone_null_bb = self
            .context
            .append_basic_block(string_clone_fn, "clone_null");
        let clone_copy_bb = self
            .context
            .append_basic_block(string_clone_fn, "clone_copy");
        self.builder
            .build_conditional_branch(src_is_null, clone_null_bb, clone_copy_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(clone_null_bb);
        self.builder
            .build_return(Some(&i8_ptr_type.const_zero()))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(clone_copy_bb);
        let clone_len_call = self
            .builder
            .build_call(strlen_fn, &[src_ptr.into()], "clone_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let clone_len = clone_len_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strlen returned void".to_string()))?
            .into_int_value();
        let clone_size = self
            .builder
            .build_int_add(clone_len, one, "clone_size")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let clone_alloc = self
            .builder
            .build_call(malloc_fn, &[clone_size.into()], "clone_alloc")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let clone_ptr = clone_alloc
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("malloc returned void".to_string()))?
            .into_pointer_value();
        let clone_alloc_null = self
            .builder
            .build_is_null(clone_ptr, "clone_alloc_is_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let clone_fail_bb = self
            .context
            .append_basic_block(string_clone_fn, "clone_alloc_fail");
        let clone_do_copy_bb = self
            .context
            .append_basic_block(string_clone_fn, "clone_do_copy");
        self.builder
            .build_conditional_branch(clone_alloc_null, clone_fail_bb, clone_do_copy_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(clone_fail_bb);
        self.builder
            .build_return(Some(&i8_ptr_type.const_zero()))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(clone_do_copy_bb);
        self.builder
            .build_call(
                memcpy_fn,
                &[clone_ptr.into(), src_ptr.into(), clone_size.into()],
                "clone_memcpy",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&clone_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb9 {
            self.builder.position_at_end(bb);
        }

        // String_substring(&String, start: u64, end: u64) -> String
        let string_substring_ty = i8_ptr_type.fn_type(
            &[
                string_ptr_ty.into(),
                self.context.i64_type().into(),
                self.context.i64_type().into(),
            ],
            false,
        );
        let string_substring_fn =
            self.module
                .add_function("String_substring", string_substring_ty, None);
        self.functions
            .insert("String_substring".to_string(), string_substring_fn);
        let prev_bb10 = self.builder.get_insert_block();
        let entry10 = self
            .context
            .append_basic_block(string_substring_fn, "entry");
        self.builder.position_at_end(entry10);
        let substring_src_ptr_ptr = string_substring_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let substring_start = string_substring_fn
            .get_nth_param(1)
            .unwrap()
            .into_int_value();
        let substring_end = string_substring_fn
            .get_nth_param(2)
            .unwrap()
            .into_int_value();
        let substring_src = self
            .builder
            .build_load(i8_ptr_type, substring_src_ptr_ptr, "substring_src")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let substring_src_is_null = self
            .builder
            .build_is_null(substring_src, "substring_src_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let substring_null_bb = self
            .context
            .append_basic_block(string_substring_fn, "substring_null");
        let substring_len_bb = self
            .context
            .append_basic_block(string_substring_fn, "substring_len");
        self.builder
            .build_conditional_branch(substring_src_is_null, substring_null_bb, substring_len_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(substring_null_bb);
        let new_fn = *self
            .functions
            .get("String_new")
            .ok_or_else(|| CodegenError::CompilationError("String_new missing".to_string()))?;
        let empty_call = self
            .builder
            .build_call(new_fn, &[], "substring_empty")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let empty_ptr = empty_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("String_new returned void".to_string()))?
            .into_pointer_value();
        self.builder
            .build_return(Some(&empty_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(substring_len_bb);
        let substring_len_call = self
            .builder
            .build_call(strlen_fn, &[substring_src.into()], "substring_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let substring_total_len = substring_len_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strlen returned void".to_string()))?
            .into_int_value();
        let start_ge_len = self
            .builder
            .build_int_compare(
                IntPredicate::UGE,
                substring_start,
                substring_total_len,
                "substring_start_ge_len",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let substring_start_oob_bb = self
            .context
            .append_basic_block(string_substring_fn, "substring_start_oob");
        let substring_continue_bb = self
            .context
            .append_basic_block(string_substring_fn, "substring_continue");
        self.builder
            .build_conditional_branch(start_ge_len, substring_start_oob_bb, substring_continue_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(substring_start_oob_bb);
        let empty_call2 = self
            .builder
            .build_call(new_fn, &[], "substring_empty2")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let empty_ptr2 = empty_call2
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("String_new returned void".to_string()))?
            .into_pointer_value();
        self.builder
            .build_return(Some(&empty_ptr2))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(substring_continue_bb);
        let end_gt_len = self
            .builder
            .build_int_compare(
                IntPredicate::UGT,
                substring_end,
                substring_total_len,
                "substring_end_gt_len",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let end_clamped_val = self
            .builder
            .build_select(
                end_gt_len,
                substring_total_len,
                substring_end,
                "substring_end_clamped",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let start_ge_end = self
            .builder
            .build_int_compare(
                IntPredicate::UGE,
                substring_start,
                end_clamped_val,
                "substring_start_ge_end",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let substring_empty_bb = self
            .context
            .append_basic_block(string_substring_fn, "substring_empty");
        let substring_copy_bb = self
            .context
            .append_basic_block(string_substring_fn, "substring_copy");
        self.builder
            .build_conditional_branch(start_ge_end, substring_empty_bb, substring_copy_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(substring_empty_bb);
        let empty_call3 = self
            .builder
            .build_call(new_fn, &[], "substring_empty3")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let empty_ptr3 = empty_call3
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("String_new returned void".to_string()))?
            .into_pointer_value();
        self.builder
            .build_return(Some(&empty_ptr3))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(substring_copy_bb);
        let substring_len_value = self
            .builder
            .build_int_sub(end_clamped_val, substring_start, "substring_length")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let substring_size = self
            .builder
            .build_int_add(substring_len_value, one, "substring_size")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let substring_alloc = self
            .builder
            .build_call(malloc_fn, &[substring_size.into()], "substring_alloc")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let substring_result = substring_alloc
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("malloc returned void".to_string()))?
            .into_pointer_value();
        let substring_alloc_null = self
            .builder
            .build_is_null(substring_result, "substring_alloc_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let substring_alloc_fail_bb = self
            .context
            .append_basic_block(string_substring_fn, "substring_alloc_fail");
        let substring_alloc_copy_bb = self
            .context
            .append_basic_block(string_substring_fn, "substring_alloc_copy");
        self.builder
            .build_conditional_branch(
                substring_alloc_null,
                substring_alloc_fail_bb,
                substring_alloc_copy_bb,
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(substring_alloc_fail_bb);
        self.builder
            .build_return(Some(&i8_ptr_type.const_zero()))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(substring_alloc_copy_bb);
        let substring_i8 = self.context.i8_type();
        let substring_offset_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                substring_i8,
                substring_src,
                &[substring_start],
                "substring_src_offset",
            )
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_call(
                memcpy_fn,
                &[
                    substring_result.into(),
                    substring_offset_ptr.into(),
                    substring_len_value.into(),
                ],
                "substring_memcpy",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let substring_term_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                substring_i8,
                substring_result,
                &[substring_len_value],
                "substring_term",
            )
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(substring_term_ptr, zero_i8)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&substring_result))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb10 {
            self.builder.position_at_end(bb);
        }

        // String_trim(&String) -> String
        let string_trim_ty = i8_ptr_type.fn_type(&[string_ptr_ty.into()], false);
        let string_trim_fn = self
            .module
            .add_function("String_trim", string_trim_ty, None);
        self.functions
            .insert("String_trim".to_string(), string_trim_fn);
        let prev_bb11 = self.builder.get_insert_block();
        let entry11 = self.context.append_basic_block(string_trim_fn, "entry");
        self.builder.position_at_end(entry11);
        let trim_src_ptr_ptr = string_trim_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let trim_src_ptr = self
            .builder
            .build_load(i8_ptr_type, trim_src_ptr_ptr, "trim_src")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let trim_src_is_null = self
            .builder
            .build_is_null(trim_src_ptr, "trim_src_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_null_bb = self.context.append_basic_block(string_trim_fn, "trim_null");
        let trim_init_bb = self.context.append_basic_block(string_trim_fn, "trim_init");
        self.builder
            .build_conditional_branch(trim_src_is_null, trim_null_bb, trim_init_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_null_bb);
        let empty_call_trim = self
            .builder
            .build_call(new_fn, &[], "trim_empty")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let empty_trim_ptr = empty_call_trim
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("String_new returned void".to_string()))?
            .into_pointer_value();
        self.builder
            .build_return(Some(&empty_trim_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_init_bb);
        let trim_len_call = self
            .builder
            .build_call(strlen_fn, &[trim_src_ptr.into()], "trim_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_len_val = trim_len_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strlen returned void".to_string()))?
            .into_int_value();
        let is_empty_trim = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                trim_len_val,
                self.context.i64_type().const_zero(),
                "trim_len_zero",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_empty_bb = self
            .context
            .append_basic_block(string_trim_fn, "trim_len_empty");
        let trim_prepare_bb = self
            .context
            .append_basic_block(string_trim_fn, "trim_prepare");
        self.builder
            .build_conditional_branch(is_empty_trim, trim_empty_bb, trim_prepare_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_empty_bb);
        let empty_call_trim2 = self
            .builder
            .build_call(new_fn, &[], "trim_empty2")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let empty_trim_ptr2 = empty_call_trim2
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("String_new returned void".to_string()))?
            .into_pointer_value();
        self.builder
            .build_return(Some(&empty_trim_ptr2))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_prepare_bb);
        let i64_type = self.context.i64_type();
        let trim_start_alloca = self
            .builder
            .build_alloca(i64_type, "trim_start")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_end_alloca = self
            .builder
            .build_alloca(i64_type, "trim_end")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(trim_start_alloca, i64_type.const_zero())
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(trim_end_alloca, trim_len_val)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_src_ref = self
            .builder
            .build_alloca(i8_ptr_type, "trim_src_ref")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(trim_src_ref, trim_src_ptr)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        let trim_start_loop = self
            .context
            .append_basic_block(string_trim_fn, "trim_start_loop");
        let trim_start_exit = self
            .context
            .append_basic_block(string_trim_fn, "trim_start_exit");
        self.builder
            .build_unconditional_branch(trim_start_loop)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_start_loop);
        let current_start = self
            .builder
            .build_load(i64_type, trim_start_alloca, "trim_start_val")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let start_lt_len = self
            .builder
            .build_int_compare(
                IntPredicate::ULT,
                current_start,
                trim_len_val,
                "trim_start_lt_len",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_start_body = self
            .context
            .append_basic_block(string_trim_fn, "trim_start_body");
        self.builder
            .build_conditional_branch(start_lt_len, trim_start_body, trim_start_exit)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_start_body);
        let i8_type = self.context.i8_type();
        let current_char_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                i8_type,
                trim_src_ptr,
                &[current_start],
                "trim_char_ptr",
            )
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let current_char = self
            .builder
            .build_load(i8_type, current_char_ptr, "trim_char")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let current_char_i32 = self
            .builder
            .build_int_z_extend(current_char, i32_type, "trim_char_i32")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let is_space_call = self
            .builder
            .build_call(isspace_fn, &[current_char_i32.into()], "trim_isspace")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let is_space_val = is_space_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("isspace returned void".to_string()))?
            .into_int_value();
        let is_space_cmp = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                is_space_val,
                i32_type.const_zero(),
                "trim_is_space",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_start_inc = self
            .context
            .append_basic_block(string_trim_fn, "trim_start_inc");
        let trim_start_break = self
            .context
            .append_basic_block(string_trim_fn, "trim_start_break");
        self.builder
            .build_conditional_branch(is_space_cmp, trim_start_inc, trim_start_break)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_start_inc);
        let next_start = self
            .builder
            .build_int_add(current_start, one, "trim_next_start")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(trim_start_alloca, next_start)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(trim_start_loop)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_start_break);
        self.builder
            .build_unconditional_branch(trim_start_exit)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_start_exit);
        let start_after_trim = self
            .builder
            .build_load(i64_type, trim_start_alloca, "trim_start_final")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let start_eq_len_trim = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                start_after_trim,
                trim_len_val,
                "trim_start_eq_len",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_all_space_bb = self
            .context
            .append_basic_block(string_trim_fn, "trim_all_space");
        let trim_end_prep_bb = self
            .context
            .append_basic_block(string_trim_fn, "trim_end_prep");
        self.builder
            .build_conditional_branch(start_eq_len_trim, trim_all_space_bb, trim_end_prep_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_all_space_bb);
        let empty_call_trim3 = self
            .builder
            .build_call(new_fn, &[], "trim_empty3")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let empty_trim_ptr3 = empty_call_trim3
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("String_new returned void".to_string()))?
            .into_pointer_value();
        self.builder
            .build_return(Some(&empty_trim_ptr3))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_end_prep_bb);
        let trim_end_loop = self
            .context
            .append_basic_block(string_trim_fn, "trim_end_loop");
        let trim_end_done = self
            .context
            .append_basic_block(string_trim_fn, "trim_end_done");
        self.builder
            .build_unconditional_branch(trim_end_loop)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_end_loop);
        let current_end = self
            .builder
            .build_load(i64_type, trim_end_alloca, "trim_end_val")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let end_gt_start = self
            .builder
            .build_int_compare(
                IntPredicate::UGT,
                current_end,
                start_after_trim,
                "trim_end_gt_start",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let end_gt_zero = self
            .builder
            .build_int_compare(
                IntPredicate::UGT,
                current_end,
                i64_type.const_zero(),
                "trim_end_gt_zero",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let end_continue_cond = self
            .builder
            .build_and(end_gt_start, end_gt_zero, "trim_end_cond")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_end_body = self
            .context
            .append_basic_block(string_trim_fn, "trim_end_body");
        self.builder
            .build_conditional_branch(end_continue_cond, trim_end_body, trim_end_done)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_end_body);
        let end_minus_one = self
            .builder
            .build_int_sub(current_end, one, "trim_idx")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let end_char_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                i8_type,
                trim_src_ptr,
                &[end_minus_one],
                "trim_end_char_ptr",
            )
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let end_char = self
            .builder
            .build_load(i8_type, end_char_ptr, "trim_end_char")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let end_char_i32 = self
            .builder
            .build_int_z_extend(end_char, i32_type, "trim_end_char_i32")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let end_space_call = self
            .builder
            .build_call(isspace_fn, &[end_char_i32.into()], "trim_end_isspace")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let end_space_val = end_space_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("isspace returned void".to_string()))?
            .into_int_value();
        let end_is_space = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                end_space_val,
                i32_type.const_zero(),
                "trim_end_is_space",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_end_dec = self
            .context
            .append_basic_block(string_trim_fn, "trim_end_dec");
        let trim_end_break = self
            .context
            .append_basic_block(string_trim_fn, "trim_end_break");
        self.builder
            .build_conditional_branch(end_is_space, trim_end_dec, trim_end_break)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_end_dec);
        self.builder
            .build_store(trim_end_alloca, end_minus_one)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(trim_end_loop)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_end_break);
        self.builder
            .build_unconditional_branch(trim_end_done)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_end_done);
        let final_end = self
            .builder
            .build_load(i64_type, trim_end_alloca, "trim_end_final")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let end_le_start = self
            .builder
            .build_int_compare(
                IntPredicate::ULE,
                final_end,
                start_after_trim,
                "trim_end_le_start",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_return_empty = self
            .context
            .append_basic_block(string_trim_fn, "trim_return_empty");
        let trim_return_substring = self
            .context
            .append_basic_block(string_trim_fn, "trim_return_substring");
        self.builder
            .build_conditional_branch(end_le_start, trim_return_empty, trim_return_substring)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_return_empty);
        let empty_call_trim4 = self
            .builder
            .build_call(new_fn, &[], "trim_empty4")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let empty_trim_ptr4 = empty_call_trim4
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("String_new returned void".to_string()))?
            .into_pointer_value();
        self.builder
            .build_return(Some(&empty_trim_ptr4))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(trim_return_substring);
        let substring_fn = *self.functions.get("String_substring").ok_or_else(|| {
            CodegenError::CompilationError("String_substring missing".to_string())
        })?;
        let trim_sub_call = self
            .builder
            .build_call(
                substring_fn,
                &[
                    trim_src_ref.into(),
                    start_after_trim.into(),
                    final_end.into(),
                ],
                "trim_substring",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let trim_result_ptr = trim_sub_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("String_substring returned void".to_string())
            })?
            .into_pointer_value();
        self.builder
            .build_return(Some(&trim_result_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb11 {
            self.builder.position_at_end(bb);
        }

        // String_push_str(&mut String, &String)
        let string_push_str_ty = self
            .context
            .void_type()
            .fn_type(&[string_ptr_ty.into(), string_ptr_ty.into()], false);
        let string_push_str_fn =
            self.module
                .add_function("String_push_str", string_push_str_ty, None);
        self.functions
            .insert("String_push_str".to_string(), string_push_str_fn);
        let prev_bb12 = self.builder.get_insert_block();
        let entry12 = self.context.append_basic_block(string_push_str_fn, "entry");
        self.builder.position_at_end(entry12);
        let push_dst_ptr_ptr = string_push_str_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let push_src_ptr_ptr = string_push_str_fn
            .get_nth_param(1)
            .unwrap()
            .into_pointer_value();
        let push_dst_ptr = self
            .builder
            .build_load(i8_ptr_type, push_dst_ptr_ptr, "push_dst")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let push_src_ptr = self
            .builder
            .build_load(i8_ptr_type, push_src_ptr_ptr, "push_src")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let push_src_null = self
            .builder
            .build_is_null(push_src_ptr, "push_src_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_return_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_return");
        let push_continue_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_continue");
        self.builder
            .build_conditional_branch(push_src_null, push_return_bb, push_continue_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_continue_bb);
        let len_alloca = self
            .builder
            .build_alloca(i64_type, "push_dst_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let src_len_alloca = self
            .builder
            .build_alloca(i64_type, "push_src_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(len_alloca, i64_type.const_zero())
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(src_len_alloca, i64_type.const_zero())
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dst_null = self
            .builder
            .build_is_null(push_dst_ptr, "push_dst_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dst_len_compute_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_dst_len_compute");
        let dst_len_cont_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_dst_len_cont");
        self.builder
            .build_conditional_branch(dst_null, dst_len_cont_bb, dst_len_compute_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(dst_len_compute_bb);
        let dst_len_call = self
            .builder
            .build_call(strlen_fn, &[push_dst_ptr.into()], "push_dst_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dst_len_val = dst_len_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strlen returned void".to_string()))?
            .into_int_value();
        self.builder
            .build_store(len_alloca, dst_len_val)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(dst_len_cont_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(dst_len_cont_bb);
        let src_len_call = self
            .builder
            .build_call(strlen_fn, &[push_src_ptr.into()], "push_src_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let src_len_val = src_len_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strlen returned void".to_string()))?
            .into_int_value();
        self.builder
            .build_store(src_len_alloca, src_len_val)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let src_len_zero = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                src_len_val,
                i64_type.const_zero(),
                "push_src_len_zero",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_src_empty_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_src_empty");
        let push_alloc_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_alloc");
        self.builder
            .build_conditional_branch(src_len_zero, push_src_empty_bb, push_alloc_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_src_empty_bb);
        self.builder
            .build_unconditional_branch(push_return_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_alloc_bb);
        let dst_len_loaded = self
            .builder
            .build_load(i64_type, len_alloca, "push_dst_len_val")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let total_len = self
            .builder
            .build_int_add(dst_len_loaded, src_len_val, "push_total_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let total_size = self
            .builder
            .build_int_add(total_len, one, "push_total_size")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_alloc_call = self
            .builder
            .build_call(malloc_fn, &[total_size.into()], "push_alloc")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_new_ptr = push_alloc_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("malloc returned void".to_string()))?
            .into_pointer_value();
        let push_alloc_null = self
            .builder
            .build_is_null(push_new_ptr, "push_alloc_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_alloc_fail_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_alloc_fail");
        let push_copy_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_copy");
        self.builder
            .build_conditional_branch(push_alloc_null, push_alloc_fail_bb, push_copy_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_alloc_fail_bb);
        self.builder
            .build_unconditional_branch(push_return_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_copy_bb);
        let dst_len_nonzero = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                dst_len_loaded,
                i64_type.const_zero(),
                "push_dst_has_data",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_copy_dst_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_copy_dst");
        let push_copy_dst_cont_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_copy_dst_cont");
        self.builder
            .build_conditional_branch(dst_len_nonzero, push_copy_dst_bb, push_copy_dst_cont_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_copy_dst_bb);
        self.builder
            .build_call(
                memcpy_fn,
                &[
                    push_new_ptr.into(),
                    push_dst_ptr.into(),
                    dst_len_loaded.into(),
                ],
                "push_copy_dst",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(push_copy_dst_cont_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_copy_dst_cont_bb);
        let dst_offset_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                i8_type,
                push_new_ptr,
                &[dst_len_loaded],
                "push_src_offset",
            )
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_call(
                memcpy_fn,
                &[
                    dst_offset_ptr.into(),
                    push_src_ptr.into(),
                    src_len_val.into(),
                ],
                "push_copy_src",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let terminator_ptr = unsafe {
            self.builder
                .build_in_bounds_gep(i8_type, push_new_ptr, &[total_len], "push_terminator")
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(terminator_ptr, zero_i8)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dst_was_null = dst_null;
        let dealloc_fn = *self
            .functions
            .get("dealloc")
            .ok_or_else(|| CodegenError::CompilationError("dealloc missing".to_string()))?;
        let dst_free_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_free_dst");
        let dst_store_bb = self
            .context
            .append_basic_block(string_push_str_fn, "push_store");
        self.builder
            .build_conditional_branch(dst_was_null, dst_store_bb, dst_free_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(dst_free_bb);
        self.builder
            .build_call(dealloc_fn, &[push_dst_ptr.into()], "push_free")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(dst_store_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(dst_store_bb);
        self.builder
            .build_store(push_dst_ptr_ptr, push_new_ptr)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(push_return_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_return_bb);
        self.builder
            .build_return(None)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb12 {
            self.builder.position_at_end(bb);
        }

        // String_push_char(&mut String, ch: u8)
        let string_push_char_ty = self.context.void_type().fn_type(
            &[string_ptr_ty.into(), self.context.i8_type().into()],
            false,
        );
        let string_push_char_fn =
            self.module
                .add_function("String_push_char", string_push_char_ty, None);
        self.functions
            .insert("String_push_char".to_string(), string_push_char_fn);
        let prev_bb13 = self.builder.get_insert_block();
        let entry13 = self
            .context
            .append_basic_block(string_push_char_fn, "entry");
        self.builder.position_at_end(entry13);
        let push_char_dst_ptr_ptr = string_push_char_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let push_char_value = string_push_char_fn
            .get_nth_param(1)
            .unwrap()
            .into_int_value();
        let push_char_dst_ptr = self
            .builder
            .build_load(i8_ptr_type, push_char_dst_ptr_ptr, "push_char_dst")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let push_char_dst_null = self
            .builder
            .build_is_null(push_char_dst_ptr, "push_char_dst_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_char_return_bb = self
            .context
            .append_basic_block(string_push_char_fn, "push_char_return");
        let push_char_compute_bb = self
            .context
            .append_basic_block(string_push_char_fn, "push_char_compute");
        self.builder
            .build_conditional_branch(
                push_char_dst_null,
                push_char_return_bb,
                push_char_compute_bb,
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_char_compute_bb);
        let push_char_len_call = self
            .builder
            .build_call(strlen_fn, &[push_char_dst_ptr.into()], "push_char_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_char_len = push_char_len_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strlen returned void".to_string()))?
            .into_int_value();
        let push_char_total_len = self
            .builder
            .build_int_add(push_char_len, one, "push_char_total_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_char_total_size = self
            .builder
            .build_int_add(push_char_total_len, one, "push_char_total_size")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_char_alloc = self
            .builder
            .build_call(malloc_fn, &[push_char_total_size.into()], "push_char_alloc")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_char_new_ptr = push_char_alloc
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("malloc returned void".to_string()))?
            .into_pointer_value();
        let push_char_alloc_null = self
            .builder
            .build_is_null(push_char_new_ptr, "push_char_alloc_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_char_fail_bb = self
            .context
            .append_basic_block(string_push_char_fn, "push_char_fail");
        let push_char_copy_bb = self
            .context
            .append_basic_block(string_push_char_fn, "push_char_copy");
        self.builder
            .build_conditional_branch(push_char_alloc_null, push_char_fail_bb, push_char_copy_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_char_fail_bb);
        self.builder
            .build_unconditional_branch(push_char_return_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_char_copy_bb);
        let push_char_len_nonzero = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                push_char_len,
                i64_type.const_zero(),
                "push_char_len_nonzero",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let push_char_copy_existing_bb = self
            .context
            .append_basic_block(string_push_char_fn, "push_char_copy_existing");
        let push_char_copy_skip_bb = self
            .context
            .append_basic_block(string_push_char_fn, "push_char_copy_skip");
        self.builder
            .build_conditional_branch(
                push_char_len_nonzero,
                push_char_copy_existing_bb,
                push_char_copy_skip_bb,
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_char_copy_existing_bb);
        self.builder
            .build_call(
                memcpy_fn,
                &[
                    push_char_new_ptr.into(),
                    push_char_dst_ptr.into(),
                    push_char_len.into(),
                ],
                "push_char_copy_existing",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(push_char_copy_skip_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_char_copy_skip_bb);
        let char_insert_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                i8_type,
                push_char_new_ptr,
                &[push_char_len],
                "push_char_insert_ptr",
            )
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(char_insert_ptr, push_char_value)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let char_term_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                i8_type,
                push_char_new_ptr,
                &[push_char_total_len],
                "push_char_term_ptr",
            )
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(char_term_ptr, zero_i8)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_call(dealloc_fn, &[push_char_dst_ptr.into()], "push_char_free")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(push_char_dst_ptr_ptr, push_char_new_ptr)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(push_char_return_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(push_char_return_bb);
        self.builder
            .build_return(None)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb13 {
            self.builder.position_at_end(bb);
        }

        // String_from_u64(u64) -> String
        let string_from_u64_ty = i8_ptr_type.fn_type(&[self.context.i64_type().into()], false);
        let string_from_u64_fn =
            self.module
                .add_function("String_from_u64", string_from_u64_ty, None);
        self.functions
            .insert("String_from_u64".to_string(), string_from_u64_fn);
        let prev_bb14 = self.builder.get_insert_block();
        let entry14 = self.context.append_basic_block(string_from_u64_fn, "entry");
        self.builder.position_at_end(entry14);
        let value_u64 = string_from_u64_fn
            .get_nth_param(0)
            .unwrap()
            .into_int_value();
        let buffer_size = self.context.i64_type().const_int(32, false);
        let buffer_alloc = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "from_u64_alloc")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let buffer_ptr = buffer_alloc
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("malloc returned void".to_string()))?
            .into_pointer_value();
        let buffer_is_null = self
            .builder
            .build_is_null(buffer_ptr, "from_u64_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let buffer_fail_bb = self
            .context
            .append_basic_block(string_from_u64_fn, "from_u64_fail");
        let buffer_write_bb = self
            .context
            .append_basic_block(string_from_u64_fn, "from_u64_write");
        self.builder
            .build_conditional_branch(buffer_is_null, buffer_fail_bb, buffer_write_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(buffer_fail_bb);
        self.builder
            .build_return(Some(&i8_ptr_type.const_zero()))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(buffer_write_bb);
        let fmt_u64 = self
            .builder
            .build_global_string_ptr("%llu", "fmt_u64")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let snprintf_fn = *self
            .functions
            .get("snprintf")
            .ok_or_else(|| CodegenError::CompilationError("snprintf missing".to_string()))?;
        self.builder
            .build_call(
                snprintf_fn,
                &[
                    buffer_ptr.into(),
                    buffer_size.into(),
                    fmt_u64.as_pointer_value().into(),
                    value_u64.into(),
                ],
                "from_u64_snprintf",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let from_cstr_fn = *self.functions.get("String_from_cstr").ok_or_else(|| {
            CodegenError::CompilationError("String_from_cstr missing".to_string())
        })?;
        let from_u64_call = self
            .builder
            .build_call(from_cstr_fn, &[buffer_ptr.into()], "from_u64_clone")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let from_u64_ptr = from_u64_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("String_from_cstr returned void".to_string())
            })?
            .into_pointer_value();
        self.builder
            .build_call(dealloc_fn, &[buffer_ptr.into()], "from_u64_free")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&from_u64_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb14 {
            self.builder.position_at_end(bb);
        }

        // String_from_i64(i64) -> String
        let string_from_i64_ty = i8_ptr_type.fn_type(&[self.context.i64_type().into()], false);
        let string_from_i64_fn =
            self.module
                .add_function("String_from_i64", string_from_i64_ty, None);
        self.functions
            .insert("String_from_i64".to_string(), string_from_i64_fn);
        let prev_bb15 = self.builder.get_insert_block();
        let entry15 = self.context.append_basic_block(string_from_i64_fn, "entry");
        self.builder.position_at_end(entry15);
        let value_i64 = string_from_i64_fn
            .get_nth_param(0)
            .unwrap()
            .into_int_value();
        let buffer_alloc_i64 = self
            .builder
            .build_call(malloc_fn, &[buffer_size.into()], "from_i64_alloc")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let buffer_ptr_i64 = buffer_alloc_i64
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("malloc returned void".to_string()))?
            .into_pointer_value();
        let buffer_i64_null = self
            .builder
            .build_is_null(buffer_ptr_i64, "from_i64_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let buffer_i64_fail_bb = self
            .context
            .append_basic_block(string_from_i64_fn, "from_i64_fail");
        let buffer_i64_write_bb = self
            .context
            .append_basic_block(string_from_i64_fn, "from_i64_write");
        self.builder
            .build_conditional_branch(buffer_i64_null, buffer_i64_fail_bb, buffer_i64_write_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(buffer_i64_fail_bb);
        self.builder
            .build_return(Some(&i8_ptr_type.const_zero()))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(buffer_i64_write_bb);
        let fmt_i64 = self
            .builder
            .build_global_string_ptr("%lld", "fmt_i64")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_call(
                snprintf_fn,
                &[
                    buffer_ptr_i64.into(),
                    buffer_size.into(),
                    fmt_i64.as_pointer_value().into(),
                    value_i64.into(),
                ],
                "from_i64_snprintf",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let from_i64_call = self
            .builder
            .build_call(from_cstr_fn, &[buffer_ptr_i64.into()], "from_i64_clone")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let from_i64_ptr = from_i64_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("String_from_cstr returned void".to_string())
            })?
            .into_pointer_value();
        self.builder
            .build_call(dealloc_fn, &[buffer_ptr_i64.into()], "from_i64_free")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&from_i64_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb15 {
            self.builder.position_at_end(bb);
        }

        // String_ends_with(&String, &String) -> bool
        let string_ends_with_ty =
            bool_ty.fn_type(&[string_ptr_ty.into(), string_ptr_ty.into()], false);
        let string_ends_with_fn =
            self.module
                .add_function("String_ends_with", string_ends_with_ty, None);
        self.functions
            .insert("String_ends_with".to_string(), string_ends_with_fn);
        let prev_bb16 = self.builder.get_insert_block();
        let entry16 = self
            .context
            .append_basic_block(string_ends_with_fn, "entry");
        self.builder.position_at_end(entry16);
        let ends_first_ptr_ptr = string_ends_with_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let ends_second_ptr_ptr = string_ends_with_fn
            .get_nth_param(1)
            .unwrap()
            .into_pointer_value();
        let ends_first = self
            .builder
            .build_load(i8_ptr_type, ends_first_ptr_ptr, "ends_first")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let ends_second = self
            .builder
            .build_load(i8_ptr_type, ends_second_ptr_ptr, "ends_second")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let first_null = self
            .builder
            .build_is_null(ends_first, "ends_first_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let second_null = self
            .builder
            .build_is_null(ends_second, "ends_second_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let both_null = self
            .builder
            .build_and(first_null, second_null, "ends_both_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let ends_both_bb = self
            .context
            .append_basic_block(string_ends_with_fn, "ends_both");
        let ends_non_both_bb = self
            .context
            .append_basic_block(string_ends_with_fn, "ends_non_both");
        self.builder
            .build_conditional_branch(both_null, ends_both_bb, ends_non_both_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(ends_both_bb);
        self.builder
            .build_return(Some(&bool_ty.const_int(1, false)))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(ends_non_both_bb);
        let either_null = self
            .builder
            .build_or(first_null, second_null, "ends_either_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let ends_null_case_bb = self
            .context
            .append_basic_block(string_ends_with_fn, "ends_null_case");
        let ends_compare_bb = self
            .context
            .append_basic_block(string_ends_with_fn, "ends_compare");
        self.builder
            .build_conditional_branch(either_null, ends_null_case_bb, ends_compare_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(ends_null_case_bb);
        self.builder
            .build_return(Some(&bool_ty.const_int(0, false)))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(ends_compare_bb);
        let ends_fn = *self
            .functions
            .get("ends_with")
            .ok_or_else(|| CodegenError::CompilationError("ends_with missing".to_string()))?;
        let ends_call = self
            .builder
            .build_call(
                ends_fn,
                &[ends_first.into(), ends_second.into()],
                "ends_result",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let ends_res = ends_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("ends_with returned void".to_string()))?
            .into_int_value();
        self.builder
            .build_return(Some(&ends_res))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb16 {
            self.builder.position_at_end(bb);
        }

        // marshal_string_to_cstr(String) -> *raw u8
        let marshal_string_to_cstr_ty = i8_ptr_type.fn_type(&[i8_ptr_type.into()], false);
        let marshal_string_to_cstr_fn =
            self.module
                .add_function("marshal_string_to_cstr", marshal_string_to_cstr_ty, None);
        self.functions.insert(
            "marshal_string_to_cstr".to_string(),
            marshal_string_to_cstr_fn,
        );
        let prev_bb17 = self.builder.get_insert_block();
        let entry17 = self
            .context
            .append_basic_block(marshal_string_to_cstr_fn, "entry");
        self.builder.position_at_end(entry17);
        let marshal_src = marshal_string_to_cstr_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let marshal_null_ptr = i8_ptr_type.const_zero();
        let marshal_src_is_null = self
            .builder
            .build_is_null(marshal_src, "marshal_src_is_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let marshal_null_bb = self
            .context
            .append_basic_block(marshal_string_to_cstr_fn, "marshal_null");
        let marshal_copy_bb = self
            .context
            .append_basic_block(marshal_string_to_cstr_fn, "marshal_copy");
        self.builder
            .build_conditional_branch(marshal_src_is_null, marshal_null_bb, marshal_copy_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(marshal_null_bb);
        self.builder
            .build_return(Some(&marshal_null_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(marshal_copy_bb);
        let strlen_fn = *self
            .functions
            .get("len")
            .ok_or_else(|| CodegenError::CompilationError("strlen missing".to_string()))?;
        let alloc_fn = *self
            .functions
            .get("alloc")
            .ok_or_else(|| CodegenError::CompilationError("alloc missing".to_string()))?;
        let memcpy_fn = *self
            .functions
            .get("memcpy")
            .ok_or_else(|| CodegenError::CompilationError("memcpy missing".to_string()))?;
        let marshal_len_call = self
            .builder
            .build_call(strlen_fn, &[marshal_src.into()], "marshal_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let marshal_len = marshal_len_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("strlen returned void".to_string()))?
            .into_int_value();
        let marshal_total = self
            .builder
            .build_int_add(
                marshal_len,
                self.context.i64_type().const_int(1, false),
                "marshal_total",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let marshal_alloc = self
            .builder
            .build_call(alloc_fn, &[marshal_total.into()], "marshal_alloc")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let marshal_dst = marshal_alloc
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("alloc returned void".to_string()))?
            .into_pointer_value();
        let marshal_alloc_null = self
            .builder
            .build_is_null(marshal_dst, "marshal_alloc_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let marshal_alloc_fail_bb = self
            .context
            .append_basic_block(marshal_string_to_cstr_fn, "marshal_alloc_fail");
        let marshal_alloc_copy_bb = self
            .context
            .append_basic_block(marshal_string_to_cstr_fn, "marshal_alloc_copy");
        self.builder
            .build_conditional_branch(
                marshal_alloc_null,
                marshal_alloc_fail_bb,
                marshal_alloc_copy_bb,
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(marshal_alloc_fail_bb);
        self.builder
            .build_return(Some(&marshal_null_ptr))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(marshal_alloc_copy_bb);
        self.builder
            .build_call(
                memcpy_fn,
                &[marshal_dst.into(), marshal_src.into(), marshal_total.into()],
                "marshal_memcpy",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&marshal_dst))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb17 {
            self.builder.position_at_end(bb);
        }

        // marshal_free_cstr(*raw u8)
        let marshal_free_cstr_ty = self
            .context
            .void_type()
            .fn_type(&[i8_ptr_type.into()], false);
        let marshal_free_cstr_fn =
            self.module
                .add_function("marshal_free_cstr", marshal_free_cstr_ty, None);
        self.functions
            .insert("marshal_free_cstr".to_string(), marshal_free_cstr_fn);
        let prev_bb18 = self.builder.get_insert_block();
        let entry18 = self
            .context
            .append_basic_block(marshal_free_cstr_fn, "entry");
        self.builder.position_at_end(entry18);
        let free_src = marshal_free_cstr_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let dealloc_fn = *self
            .functions
            .get("dealloc")
            .ok_or_else(|| CodegenError::CompilationError("dealloc missing".to_string()))?;
        self.builder
            .build_call(dealloc_fn, &[free_src.into()], "marshal_free")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(None)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb18 {
            self.builder.position_at_end(bb);
        }

        // CommandLine_load_args() -> [String]
        let (vec_struct, data_idx, len_idx, cap_idx) = self.vector_field_indices()?;
        let command_line_load_args_ty = vec_struct.fn_type(&[], false);
        let command_line_load_args_fn =
            self.module
                .add_function("CommandLine_load_args", command_line_load_args_ty, None);
        self.functions.insert(
            "CommandLine_load_args".to_string(),
            command_line_load_args_fn,
        );
        let prev_bb19 = self.builder.get_insert_block();
        let prev_fn = self.current_function;
        self.current_function = Some(command_line_load_args_fn);
        let entry19 = self
            .context
            .append_basic_block(command_line_load_args_fn, "entry");
        self.builder.position_at_end(entry19);
        let i64_ty = self.context.i64_type();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let argv_ptr_ty = i8_ptr_type.ptr_type(AddressSpace::default());
        let i64_ptr_ty = i64_ty.ptr_type(AddressSpace::default());
        let zero_i64 = i64_ty.const_zero();
        let empty_vec_const = vec_struct.const_named_struct(&[
            ptr_ty.const_null().into(),
            zero_i64.into(),
            zero_i64.into(),
        ]);
        let argc_global = self.command_line_argc_global.ok_or_else(|| {
            CodegenError::CompilationError("Command line argc global missing".to_string())
        })?;
        let argv_global = self.command_line_argv_global.ok_or_else(|| {
            CodegenError::CompilationError("Command line argv global missing".to_string())
        })?;
        let argc_val = self
            .builder
            .build_load(
                self.context.i32_type(),
                argc_global.as_pointer_value(),
                "cli_argc",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let argc_i64 = self
            .builder
            .build_int_z_extend(argc_val, i64_ty, "cli_argc_i64")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let argc_is_zero = self
            .builder
            .build_int_compare(IntPredicate::EQ, argc_i64, zero_i64, "cli_argc_zero")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let empty_bb = self
            .context
            .append_basic_block(command_line_load_args_fn, "cli_empty");
        let cont_bb = self
            .context
            .append_basic_block(command_line_load_args_fn, "cli_continue");
        self.builder
            .build_conditional_branch(argc_is_zero, empty_bb, cont_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(empty_bb);
        self.builder
            .build_return(Some(&empty_vec_const))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(cont_bb);
        let argv_val = self
            .builder
            .build_load(argv_ptr_ty, argv_global.as_pointer_value(), "cli_argv")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let argv_is_null = self
            .builder
            .build_is_null(argv_val, "cli_argv_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let argv_null_bb = self
            .context
            .append_basic_block(command_line_load_args_fn, "cli_argv_empty");
        let argv_use_bb = self
            .context
            .append_basic_block(command_line_load_args_fn, "cli_argv_use");
        self.builder
            .build_conditional_branch(argv_is_null, argv_null_bb, argv_use_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(argv_null_bb);
        self.builder
            .build_return(Some(&empty_vec_const))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(argv_use_bb);
        let elem_size = i64_ty.const_int(8, false);
        let total_bytes = self
            .builder
            .build_int_mul(argc_i64, elem_size, "cli_bytes")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let alloc_fn = *self
            .functions
            .get("alloc")
            .ok_or_else(|| CodegenError::CompilationError("alloc missing".to_string()))?;
        let alloc_call = self
            .builder
            .build_call(alloc_fn, &[total_bytes.into()], "cli_alloc")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let alloc_ptr = alloc_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("alloc returned void".to_string()))?
            .into_pointer_value();
        let alloc_is_null = self
            .builder
            .build_is_null(alloc_ptr, "cli_alloc_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let alloc_fail_bb = self
            .context
            .append_basic_block(command_line_load_args_fn, "cli_alloc_fail");
        let alloc_ok_bb = self
            .context
            .append_basic_block(command_line_load_args_fn, "cli_alloc_ok");
        self.builder
            .build_conditional_branch(alloc_is_null, alloc_fail_bb, alloc_ok_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(alloc_fail_bb);
        self.builder
            .build_return(Some(&empty_vec_const))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(alloc_ok_bb);
        let data_ptr = self
            .builder
            .build_pointer_cast(alloc_ptr, ptr_ty, "cli_data_ptr")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let data_ptr_i64 = self
            .builder
            .build_pointer_cast(alloc_ptr, i64_ptr_ty, "cli_data_i64")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let idx_alloca = self.create_entry_block_alloca("cli_idx", i64_ty.into())?;
        self.builder
            .build_store(idx_alloca, zero_i64)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let cond_bb = self
            .context
            .append_basic_block(command_line_load_args_fn, "cli_copy_cond");
        let body_bb = self
            .context
            .append_basic_block(command_line_load_args_fn, "cli_copy_body");
        let inc_bb = self
            .context
            .append_basic_block(command_line_load_args_fn, "cli_copy_inc");
        let end_bb = self
            .context
            .append_basic_block(command_line_load_args_fn, "cli_copy_end");
        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(cond_bb);
        let idx_val = self
            .builder
            .build_load(i64_ty, idx_alloca, "cli_idx_val")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let idx_lt = self
            .builder
            .build_int_compare(IntPredicate::ULT, idx_val, argc_i64, "cli_idx_lt")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_conditional_branch(idx_lt, body_bb, end_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(body_bb);
        let arg_ptr_ptr = unsafe {
            self.builder
                .build_in_bounds_gep(i8_ptr_type, argv_val, &[idx_val], "cli_arg_ptr_ptr")
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let arg_cstr = self
            .builder
            .build_load(i8_ptr_type, arg_ptr_ptr, "cli_arg_cstr")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let string_from_cstr_fn = *self.functions.get("String_from_cstr").ok_or_else(|| {
            CodegenError::CompilationError("String_from_cstr missing".to_string())
        })?;
        let arg_str_call = self
            .builder
            .build_call(string_from_cstr_fn, &[arg_cstr.into()], "cli_arg_string")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let arg_str_ptr = arg_str_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("String_from_cstr returned void".to_string())
            })?
            .into_pointer_value();
        let arg_i64 = self
            .builder
            .build_ptr_to_int(arg_str_ptr, i64_ty, "cli_arg_i64")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let data_slot = unsafe {
            self.builder
                .build_in_bounds_gep(i64_ty, data_ptr_i64, &[idx_val], "cli_store_ptr")
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(data_slot, arg_i64)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(inc_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(inc_bb);
        let next_idx = self
            .builder
            .build_int_add(idx_val, i64_ty.const_int(1, false), "cli_next_idx")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(idx_alloca, next_idx)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(end_bb);
        let mut vec_value = vec_struct.get_undef();
        vec_value = self
            .builder
            .build_insert_value(vec_value, data_ptr, data_idx, "cli_vec_data")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        vec_value = self
            .builder
            .build_insert_value(vec_value, argc_i64, len_idx, "cli_vec_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        vec_value = self
            .builder
            .build_insert_value(vec_value, argc_i64, cap_idx, "cli_vec_cap")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        self.builder
            .build_return(Some(&vec_value))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.current_function = prev_fn;
        if let Some(bb) = prev_bb19 {
            self.builder.position_at_end(bb);
        }

        let (command_struct, args_field_index) = self.ensure_command_line_struct()?;

        // CommandLine_current() -> CommandLine
        let command_current_ty = command_struct.fn_type(&[], false);
        let command_current_fn =
            self.module
                .add_function("CommandLine_current", command_current_ty, None);
        self.functions
            .insert("CommandLine_current".to_string(), command_current_fn);
        let prev_bb20 = self.builder.get_insert_block();
        let entry20 = self.context.append_basic_block(command_current_fn, "entry");
        self.builder.position_at_end(entry20);
        let load_args_fn = *self.functions.get("CommandLine_load_args").ok_or_else(|| {
            CodegenError::CompilationError("CommandLine_load_args missing".to_string())
        })?;
        let load_call = self
            .builder
            .build_call(load_args_fn, &[], "command_load")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let load_val = load_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("CommandLine_load_args returned void".to_string())
            })?
            .into_struct_value();
        let command_result = self
            .builder
            .build_insert_value(
                command_struct.get_undef(),
                load_val,
                args_field_index,
                "command_set_args",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        self.builder
            .build_return(Some(&command_result))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb20 {
            self.builder.position_at_end(bb);
        }

        let command_ptr_ty = command_struct.ptr_type(AddressSpace::default());
        let command_len_ty = self
            .context
            .i64_type()
            .fn_type(&[command_ptr_ty.into()], false);
        let command_len_fn = self
            .module
            .add_function("CommandLine_len", command_len_ty, None);
        self.functions
            .insert("CommandLine_len".to_string(), command_len_fn);
        let prev_bb21 = self.builder.get_insert_block();
        let entry21 = self.context.append_basic_block(command_len_fn, "entry");
        self.builder.position_at_end(entry21);
        let args_param_ptr = command_len_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let args_field_ptr = self
            .builder
            .build_struct_gep(
                command_struct,
                args_param_ptr,
                args_field_index,
                "command_args_ptr",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let (vec_struct_for_cmd, _data_idx_cmd, len_idx_cmd, _cap_idx_cmd) =
            self.vector_field_indices()?;
        let args_val = self
            .builder
            .build_load(vec_struct_for_cmd, args_field_ptr, "command_args")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let args_struct = args_val.into_struct_value();
        let len_val = self
            .builder
            .build_extract_value(args_struct, len_idx_cmd, "command_args_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        self.builder
            .build_return(Some(&len_val))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb21 {
            self.builder.position_at_end(bb);
        }

        let command_is_empty_ty = self
            .context
            .bool_type()
            .fn_type(&[command_ptr_ty.into()], false);
        let command_is_empty_fn =
            self.module
                .add_function("CommandLine_is_empty", command_is_empty_ty, None);
        self.functions
            .insert("CommandLine_is_empty".to_string(), command_is_empty_fn);
        let prev_bb22 = self.builder.get_insert_block();
        let entry22 = self
            .context
            .append_basic_block(command_is_empty_fn, "entry");
        self.builder.position_at_end(entry22);
        let command_is_empty_param = command_is_empty_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let command_len_call = self
            .builder
            .build_call(
                command_len_fn,
                &[command_is_empty_param.into()],
                "command_len_call",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let command_len_value = command_len_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("CommandLine_len returned void".to_string())
            })?
            .into_int_value();
        let is_zero = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                command_len_value,
                self.context.i64_type().const_zero(),
                "command_len_zero",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&is_zero))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb22 {
            self.builder.position_at_end(bb);
        }

        let enum_ty = self.ensure_enum_struct_type();
        let command_get_ty = enum_ty.fn_type(
            &[command_ptr_ty.into(), self.context.i64_type().into()],
            false,
        );
        let command_get_fn = self
            .module
            .add_function("CommandLine_get", command_get_ty, None);
        self.functions
            .insert("CommandLine_get".to_string(), command_get_fn);
        let prev_bb23 = self.builder.get_insert_block();
        let prev_fn23 = self.current_function;
        self.current_function = Some(command_get_fn);
        let entry23 = self.context.append_basic_block(command_get_fn, "entry");
        self.builder.position_at_end(entry23);
        let get_cmd_ptr = command_get_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let get_index = command_get_fn.get_nth_param(1).unwrap().into_int_value();
        let args_gep_get = self
            .builder
            .build_struct_gep(
                command_struct,
                get_cmd_ptr,
                args_field_index,
                "command_get_args_ptr",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let (vec_struct, data_idx, len_idx, _cap_idx) = self.vector_field_indices()?;
        let args_val_get = self
            .builder
            .build_load(vec_struct, args_gep_get, "command_get_args")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        let len_val_get = self
            .builder
            .build_extract_value(args_val_get, len_idx, "command_get_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let data_ptr_raw = self
            .builder
            .build_extract_value(args_val_get, data_idx, "command_get_data")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let i64_ty = self.context.i64_type();
        let i8_ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_ptr_ty = i64_ty.ptr_type(AddressSpace::default());
        let string_ptr_ty = i8_ptr_type.ptr_type(AddressSpace::default());
        let data_ptr_i64 = self
            .builder
            .build_pointer_cast(data_ptr_raw, i64_ptr_ty, "command_get_data_i64")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let result_alloca = self.create_entry_block_alloca("command_get_result", enum_ty.into())?;
        let clone_temp =
            self.create_entry_block_alloca("command_get_clone", string_ptr_ty.into())?;
        let idx_oob = self
            .builder
            .build_int_compare(IntPredicate::UGE, get_index, len_val_get, "command_get_oob")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let current_fn_get = self.current_function.unwrap();
        let none_bb_get = self
            .context
            .append_basic_block(current_fn_get, "command.get.none");
        let some_bb_get = self
            .context
            .append_basic_block(current_fn_get, "command.get.some");
        let merge_bb_get = self
            .context
            .append_basic_block(current_fn_get, "command.get.merge");
        self.builder
            .build_conditional_branch(idx_oob, none_bb_get, some_bb_get)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(none_bb_get);
        let none_struct =
            enum_ty.const_named_struct(&[i64_ty.const_zero().into(), i64_ty.const_zero().into()]);
        self.builder
            .build_store(result_alloca, none_struct)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(merge_bb_get)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(some_bb_get);
        let elem_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                i64_ty,
                data_ptr_i64,
                &[get_index],
                "command_get_elem_ptr",
            )
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let elem_val = self
            .builder
            .build_load(i64_ty, elem_ptr, "command_get_elem")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let elem_ptr_val = self
            .builder
            .build_int_to_ptr(elem_val, i8_ptr_type, "command_get_elem_ptrval")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(clone_temp, elem_ptr_val)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let string_clone_fn = *self
            .functions
            .get("String_clone")
            .ok_or_else(|| CodegenError::CompilationError("String_clone missing".to_string()))?;
        let clone_call = self
            .builder
            .build_call(
                string_clone_fn,
                &[clone_temp.into()],
                "command_get_clone_call",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let cloned_ptr = clone_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("String_clone returned void".to_string())
            })?
            .into_pointer_value();
        let cloned_i64 = self
            .builder
            .build_ptr_to_int(cloned_ptr, i64_ty, "command_get_clone_i64")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let some_struct = self
            .builder
            .build_insert_value(
                enum_ty.get_undef(),
                i64_ty.const_int(1, false),
                0,
                "command_get_some_tag",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        let some_with_payload = self
            .builder
            .build_insert_value(some_struct, cloned_i64, 1, "command_get_some_payload")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        self.builder
            .build_store(result_alloca, some_with_payload)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(merge_bb_get)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(merge_bb_get);
        let get_result = self
            .builder
            .build_load(enum_ty, result_alloca, "command_get_result")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&get_result))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.current_function = prev_fn23;
        if let Some(bb) = prev_bb23 {
            self.builder.position_at_end(bb);
        }

        let command_slice_ty = vec_struct.fn_type(
            &[command_ptr_ty.into(), self.context.i64_type().into()],
            false,
        );
        let command_slice_fn =
            self.module
                .add_function("CommandLine_slice_from", command_slice_ty, None);
        self.functions
            .insert("CommandLine_slice_from".to_string(), command_slice_fn);
        let prev_bb24 = self.builder.get_insert_block();
        let prev_fn24 = self.current_function;
        self.current_function = Some(command_slice_fn);
        let entry24 = self.context.append_basic_block(command_slice_fn, "entry");
        self.builder.position_at_end(entry24);
        let slice_cmd_ptr = command_slice_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let slice_start = command_slice_fn.get_nth_param(1).unwrap().into_int_value();
        let slice_args_ptr = self
            .builder
            .build_struct_gep(
                command_struct,
                slice_cmd_ptr,
                args_field_index,
                "command_slice_args_ptr",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let slice_args_val = self
            .builder
            .build_load(vec_struct, slice_args_ptr, "command_slice_args")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        let slice_len = self
            .builder
            .build_extract_value(slice_args_val, len_idx, "command_slice_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let slice_data_raw = self
            .builder
            .build_extract_value(slice_args_val, data_idx, "command_slice_data")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let slice_data_i64 = self
            .builder
            .build_pointer_cast(slice_data_raw, i64_ptr_ty, "command_slice_data_i64")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let empty_vec = vec_struct.const_named_struct(&[
            self.context
                .ptr_type(AddressSpace::default())
                .const_null()
                .into(),
            i64_ty.const_zero().into(),
            i64_ty.const_zero().into(),
        ]);
        let start_ge_len = self
            .builder
            .build_int_compare(
                IntPredicate::UGE,
                slice_start,
                slice_len,
                "command_slice_oob",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let empty_bb_slice = self
            .context
            .append_basic_block(command_slice_fn, "command.slice.empty");
        let cont_bb_slice = self
            .context
            .append_basic_block(command_slice_fn, "command.slice.cont");
        self.builder
            .build_conditional_branch(start_ge_len, empty_bb_slice, cont_bb_slice)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(empty_bb_slice);
        self.builder
            .build_return(Some(&empty_vec))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(cont_bb_slice);
        let remaining_len = self
            .builder
            .build_int_sub(slice_len, slice_start, "command_slice_remaining")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let remaining_zero = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                remaining_len,
                i64_ty.const_zero(),
                "command_slice_remaining_zero",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let empty2_bb_slice = self
            .context
            .append_basic_block(command_slice_fn, "command.slice.empty2");
        let work_bb_slice = self
            .context
            .append_basic_block(command_slice_fn, "command.slice.work");
        self.builder
            .build_conditional_branch(remaining_zero, empty2_bb_slice, work_bb_slice)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(empty2_bb_slice);
        self.builder
            .build_return(Some(&empty_vec))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(work_bb_slice);
        let total_bytes_slice = self
            .builder
            .build_int_mul(
                remaining_len,
                i64_ty.const_int(8, false),
                "command_slice_bytes",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let alloc_fn = *self
            .functions
            .get("alloc")
            .ok_or_else(|| CodegenError::CompilationError("alloc missing".to_string()))?;
        let slice_alloc_call = self
            .builder
            .build_call(alloc_fn, &[total_bytes_slice.into()], "command_slice_alloc")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let slice_alloc_raw = slice_alloc_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("alloc returned void".to_string()))?
            .into_pointer_value();
        let alloc_null = self
            .builder
            .build_is_null(slice_alloc_raw, "command_slice_alloc_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let alloc_fail_slice = self
            .context
            .append_basic_block(command_slice_fn, "command.slice.alloc_fail");
        let alloc_ok_slice = self
            .context
            .append_basic_block(command_slice_fn, "command.slice.alloc_ok");
        self.builder
            .build_conditional_branch(alloc_null, alloc_fail_slice, alloc_ok_slice)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(alloc_fail_slice);
        self.builder
            .build_return(Some(&empty_vec))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(alloc_ok_slice);
        let slice_data_new = self
            .builder
            .build_pointer_cast(
                slice_alloc_raw,
                self.context.ptr_type(AddressSpace::default()),
                "command_slice_data_ptr",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let slice_data_new_i64 = self
            .builder
            .build_pointer_cast(slice_alloc_raw, i64_ptr_ty, "command_slice_data_i64_new")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let slice_idx_alloca =
            self.create_entry_block_alloca("command_slice_idx", i64_ty.into())?;
        let slice_clone_tmp =
            self.create_entry_block_alloca("command_slice_clone_tmp", string_ptr_ty.into())?;
        self.builder
            .build_store(slice_idx_alloca, i64_ty.const_zero())
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let loop_cond_bb = self
            .context
            .append_basic_block(command_slice_fn, "command.slice.loop.cond");
        let loop_body_bb = self
            .context
            .append_basic_block(command_slice_fn, "command.slice.loop.body");
        let loop_inc_bb = self
            .context
            .append_basic_block(command_slice_fn, "command.slice.loop.inc");
        let loop_end_bb = self
            .context
            .append_basic_block(command_slice_fn, "command.slice.loop.end");
        self.builder
            .build_unconditional_branch(loop_cond_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(loop_cond_bb);
        let loop_idx_val = self
            .builder
            .build_load(i64_ty, slice_idx_alloca, "command_slice_idx_val")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let loop_cond = self
            .builder
            .build_int_compare(
                IntPredicate::ULT,
                loop_idx_val,
                remaining_len,
                "command_slice_loop_cond",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_conditional_branch(loop_cond, loop_body_bb, loop_end_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(loop_body_bb);
        let source_idx = self
            .builder
            .build_int_add(loop_idx_val, slice_start, "command_slice_source_idx")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let source_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                i64_ty,
                slice_data_i64,
                &[source_idx],
                "command_slice_source_ptr",
            )
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let source_val = self
            .builder
            .build_load(i64_ty, source_ptr, "command_slice_source_val")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let source_ptr_value = self
            .builder
            .build_int_to_ptr(source_val, i8_ptr_type, "command_slice_source_ptrval")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(slice_clone_tmp, source_ptr_value)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let clone_call_slice = self
            .builder
            .build_call(
                string_clone_fn,
                &[slice_clone_tmp.into()],
                "command_slice_clone",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let cloned_ptr_slice = clone_call_slice
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("String_clone returned void".to_string())
            })?
            .into_pointer_value();
        let cloned_val_slice = self
            .builder
            .build_ptr_to_int(cloned_ptr_slice, i64_ty, "command_slice_cloned_i64")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dest_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                i64_ty,
                slice_data_new_i64,
                &[loop_idx_val],
                "command_slice_dest_ptr",
            )
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(dest_ptr, cloned_val_slice)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(loop_inc_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(loop_inc_bb);
        let next_idx_slice = self
            .builder
            .build_int_add(
                loop_idx_val,
                i64_ty.const_int(1, false),
                "command_slice_next_idx",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(slice_idx_alloca, next_idx_slice)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(loop_cond_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(loop_end_bb);
        let mut slice_vec = vec_struct.get_undef();
        slice_vec = self
            .builder
            .build_insert_value(
                slice_vec,
                slice_data_new,
                data_idx,
                "command_slice_vec_data",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        slice_vec = self
            .builder
            .build_insert_value(slice_vec, remaining_len, len_idx, "command_slice_vec_len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        slice_vec = self
            .builder
            .build_insert_value(slice_vec, remaining_len, _cap_idx, "command_slice_vec_cap")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        self.builder
            .build_return(Some(&slice_vec))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.current_function = prev_fn24;
        if let Some(bb) = prev_bb24 {
            self.builder.position_at_end(bb);
        }

        let (path_struct, path_field_index) = self.ensure_path_struct()?;
        let path_new_ty = path_struct.fn_type(&[i8_ptr_type.into()], false);
        let path_new_fn = self.module.add_function("Path_new", path_new_ty, None);
        self.functions.insert("Path_new".to_string(), path_new_fn);
        let prev_bb25 = self.builder.get_insert_block();
        let entry25 = self.context.append_basic_block(path_new_fn, "entry");
        self.builder.position_at_end(entry25);
        let path_param = path_new_fn.get_nth_param(0).unwrap().into_pointer_value();
        let path_val = self
            .builder
            .build_insert_value(
                path_struct.get_undef(),
                path_param,
                path_field_index,
                "path_new_set",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_struct_value();
        self.builder
            .build_return(Some(&path_val))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb25 {
            self.builder.position_at_end(bb);
        }

        let path_ptr_ty = path_struct.ptr_type(AddressSpace::default());
        let bool_ty = self.context.bool_type();
        let path_exists_ty = bool_ty.fn_type(&[path_ptr_ty.into()], false);
        let path_exists_fn = self
            .module
            .add_function("Path_exists", path_exists_ty, None);
        self.functions
            .insert("Path_exists".to_string(), path_exists_fn);
        let prev_bb26 = self.builder.get_insert_block();
        let entry26 = self.context.append_basic_block(path_exists_fn, "entry");
        self.builder.position_at_end(entry26);
        let exists_param = path_exists_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let path_field_ptr = self
            .builder
            .build_struct_gep(
                path_struct,
                exists_param,
                path_field_index,
                "path_exists_ptr",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let path_str = self
            .builder
            .build_load(i8_ptr_type, path_field_ptr, "path_exists_str")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_pointer_value();
        let marshal_fn = *self
            .functions
            .get("marshal_string_to_cstr")
            .ok_or_else(|| {
                CodegenError::CompilationError("marshal_string_to_cstr missing".to_string())
            })?;
        let free_fn = *self.functions.get("marshal_free_cstr").ok_or_else(|| {
            CodegenError::CompilationError("marshal_free_cstr missing".to_string())
        })?;
        let access_fn = *self
            .functions
            .get("__libc_access")
            .ok_or_else(|| CodegenError::CompilationError("access missing".to_string()))?;
        let path_c_call = self
            .builder
            .build_call(marshal_fn, &[path_str.into()], "path_exists_cstr")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let path_c = path_c_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("marshal_string_to_cstr returned void".to_string())
            })?
            .into_pointer_value();
        let path_c_null = self
            .builder
            .build_is_null(path_c, "path_exists_cstr_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let exists_fail_bb = self
            .context
            .append_basic_block(path_exists_fn, "path.exists.fail");
        let exists_work_bb = self
            .context
            .append_basic_block(path_exists_fn, "path.exists.work");
        self.builder
            .build_conditional_branch(path_c_null, exists_fail_bb, exists_work_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(exists_fail_bb);
        let fail_bool = bool_ty.const_zero();
        self.builder
            .build_return(Some(&fail_bool))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(exists_work_bb);
        let access_call = self
            .builder
            .build_call(
                access_fn,
                &[path_c.into(), i32_type.const_zero().into()],
                "path_exists_access",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let access_val = access_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("access returned void".to_string()))?
            .into_int_value();
        self.builder
            .build_call(free_fn, &[path_c.into()], "path_exists_free")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let success_cmp = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                access_val,
                i32_type.const_zero(),
                "path_exists_ok",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&success_cmp))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb26 {
            self.builder.position_at_end(bb);
        }

        let io_dir_exists_ty = bool_ty.fn_type(&[i8_ptr_type.into()], false);
        let io_dir_exists_fn = self
            .module
            .add_function("io_dir_exists", io_dir_exists_ty, None);
        self.functions
            .insert("io_dir_exists".to_string(), io_dir_exists_fn);
        let prev_bb27 = self.builder.get_insert_block();
        let entry27 = self.context.append_basic_block(io_dir_exists_fn, "entry");
        self.builder.position_at_end(entry27);
        let dir_param = io_dir_exists_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let dir_c_call = self
            .builder
            .build_call(marshal_fn, &[dir_param.into()], "io_dir_exists_c")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dir_c = dir_c_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("marshal_string_to_cstr returned void".to_string())
            })?
            .into_pointer_value();
        let dir_c_null = self
            .builder
            .build_is_null(dir_c, "io_dir_exists_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dir_exists_fail = self
            .context
            .append_basic_block(io_dir_exists_fn, "io.dir.exists.fail");
        let dir_exists_work = self
            .context
            .append_basic_block(io_dir_exists_fn, "io.dir.exists.work");
        self.builder
            .build_conditional_branch(dir_c_null, dir_exists_fail, dir_exists_work)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(dir_exists_fail);
        let dir_fail = bool_ty.const_zero();
        self.builder
            .build_return(Some(&dir_fail))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(dir_exists_work);
        let dir_access_call = self
            .builder
            .build_call(
                access_fn,
                &[dir_c.into(), i32_type.const_zero().into()],
                "io_dir_exists_access",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dir_access_val = dir_access_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("access returned void".to_string()))?
            .into_int_value();
        self.builder
            .build_call(free_fn, &[dir_c.into()], "io_dir_exists_free")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dir_success = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                dir_access_val,
                i32_type.const_zero(),
                "io_dir_exists_ok",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&dir_success))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb27 {
            self.builder.position_at_end(bb);
        }

        let io_dir_create_ty = bool_ty.fn_type(&[i8_ptr_type.into()], false);
        let io_dir_create_fn =
            self.module
                .add_function("io_dir_create_dir", io_dir_create_ty, None);
        self.functions
            .insert("io_dir_create_dir".to_string(), io_dir_create_fn);
        let prev_bb28 = self.builder.get_insert_block();
        let entry28 = self.context.append_basic_block(io_dir_create_fn, "entry");
        self.builder.position_at_end(entry28);
        let mkdir_fn = *self
            .functions
            .get("__libc_mkdir")
            .ok_or_else(|| CodegenError::CompilationError("mkdir missing".to_string()))?;
        let dir_create_param = io_dir_create_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let dir_create_c_call = self
            .builder
            .build_call(marshal_fn, &[dir_create_param.into()], "io_dir_create_c")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dir_create_c = dir_create_c_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("marshal_string_to_cstr returned void".to_string())
            })?
            .into_pointer_value();
        let dir_create_null = self
            .builder
            .build_is_null(dir_create_c, "io_dir_create_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let dir_create_fail = self
            .context
            .append_basic_block(io_dir_create_fn, "io.dir.create.fail");
        let dir_create_work = self
            .context
            .append_basic_block(io_dir_create_fn, "io.dir.create.work");
        self.builder
            .build_conditional_branch(dir_create_null, dir_create_fail, dir_create_work)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(dir_create_fail);
        let dir_create_fail_val = bool_ty.const_zero();
        self.builder
            .build_return(Some(&dir_create_fail_val))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(dir_create_work);
        let mode_val = i32_type.const_int(0o755, false);
        let mkdir_call = self
            .builder
            .build_call(
                mkdir_fn,
                &[dir_create_c.into(), mode_val.into()],
                "io_dir_mkdir",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let mkdir_ret = mkdir_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("mkdir returned void".to_string()))?
            .into_int_value();
        self.builder
            .build_call(free_fn, &[dir_create_c.into()], "io_dir_create_free")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let mkdir_ok = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                mkdir_ret,
                i32_type.const_zero(),
                "io_dir_mkdir_ok",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&mkdir_ok))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb28 {
            self.builder.position_at_end(bb);
        }

        let io_dir_remove_ty = bool_ty.fn_type(&[i8_ptr_type.into()], false);
        let io_dir_remove_fn =
            self.module
                .add_function("io_dir_remove_file", io_dir_remove_ty, None);
        self.functions
            .insert("io_dir_remove_file".to_string(), io_dir_remove_fn);
        let prev_bb29 = self.builder.get_insert_block();
        let entry29 = self.context.append_basic_block(io_dir_remove_fn, "entry");
        self.builder.position_at_end(entry29);
        let unlink_fn = *self
            .functions
            .get("__libc_unlink")
            .ok_or_else(|| CodegenError::CompilationError("unlink missing".to_string()))?;
        let remove_param = io_dir_remove_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let remove_c_call = self
            .builder
            .build_call(marshal_fn, &[remove_param.into()], "io_dir_remove_c")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let remove_c = remove_c_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("marshal_string_to_cstr returned void".to_string())
            })?
            .into_pointer_value();
        let remove_null = self
            .builder
            .build_is_null(remove_c, "io_dir_remove_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let remove_fail = self
            .context
            .append_basic_block(io_dir_remove_fn, "io.dir.remove.fail");
        let remove_work = self
            .context
            .append_basic_block(io_dir_remove_fn, "io.dir.remove.work");
        self.builder
            .build_conditional_branch(remove_null, remove_fail, remove_work)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(remove_fail);
        let dir_remove_fail_val = bool_ty.const_zero();
        self.builder
            .build_return(Some(&dir_remove_fail_val))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(remove_work);
        let unlink_call = self
            .builder
            .build_call(unlink_fn, &[remove_c.into()], "io_dir_unlink")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let unlink_ret = unlink_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("unlink returned void".to_string()))?
            .into_int_value();
        self.builder
            .build_call(free_fn, &[remove_c.into()], "io_dir_remove_free")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let unlink_ok = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                unlink_ret,
                i32_type.const_zero(),
                "io_dir_unlink_ok",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&unlink_ok))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb29 {
            self.builder.position_at_end(bb);
        }

        let io_dir_rename_ty = bool_ty.fn_type(&[i8_ptr_type.into(), i8_ptr_type.into()], false);
        let io_dir_rename_fn = self
            .module
            .add_function("io_dir_rename", io_dir_rename_ty, None);
        self.functions
            .insert("io_dir_rename".to_string(), io_dir_rename_fn);
        let prev_bb30 = self.builder.get_insert_block();
        let entry30 = self.context.append_basic_block(io_dir_rename_fn, "entry");
        self.builder.position_at_end(entry30);
        let rename_fn = *self
            .functions
            .get("__libc_rename")
            .ok_or_else(|| CodegenError::CompilationError("rename missing".to_string()))?;
        let rename_from = io_dir_rename_fn
            .get_nth_param(0)
            .unwrap()
            .into_pointer_value();
        let rename_to = io_dir_rename_fn
            .get_nth_param(1)
            .unwrap()
            .into_pointer_value();
        let rename_from_c_call = self
            .builder
            .build_call(marshal_fn, &[rename_from.into()], "io_dir_rename_from_c")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let rename_from_c = rename_from_c_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("marshal_string_to_cstr returned void".to_string())
            })?
            .into_pointer_value();
        let rename_to_c_call = self
            .builder
            .build_call(marshal_fn, &[rename_to.into()], "io_dir_rename_to_c")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let rename_to_c = rename_to_c_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| {
                CodegenError::CompilationError("marshal_string_to_cstr returned void".to_string())
            })?
            .into_pointer_value();
        let rename_from_null = self
            .builder
            .build_is_null(rename_from_c, "io_dir_rename_from_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let rename_to_null = self
            .builder
            .build_is_null(rename_to_c, "io_dir_rename_to_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let rename_fail = self
            .context
            .append_basic_block(io_dir_rename_fn, "io.dir.rename.fail");
        let rename_work = self
            .context
            .append_basic_block(io_dir_rename_fn, "io.dir.rename.work");
        let rename_cond = self
            .builder
            .build_or(rename_from_null, rename_to_null, "io_dir_rename_null")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_conditional_branch(rename_cond, rename_fail, rename_work)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(rename_fail);
        let dir_rename_fail_val = bool_ty.const_zero();
        self.builder
            .build_return(Some(&dir_rename_fail_val))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder.position_at_end(rename_work);
        let rename_call = self
            .builder
            .build_call(
                rename_fn,
                &[rename_from_c.into(), rename_to_c.into()],
                "io_dir_rename_call",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let rename_ret = rename_call
            .try_as_basic_value()
            .left()
            .ok_or_else(|| CodegenError::CompilationError("rename returned void".to_string()))?
            .into_int_value();
        self.builder
            .build_call(free_fn, &[rename_from_c.into()], "io_dir_rename_from_free")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_call(free_fn, &[rename_to_c.into()], "io_dir_rename_to_free")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let rename_ok = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                rename_ret,
                i32_type.const_zero(),
                "io_dir_rename_ok",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_return(Some(&rename_ok))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        if let Some(bb) = prev_bb30 {
            self.builder.position_at_end(bb);
        }

        Ok(())
    }

    pub fn generate_program(&mut self, program: &Program) -> Result<(), CodegenError> {
        // First declare and define user functions from const decls
        self.declare_and_define_functions(program)?;

        if self.runtime_mode {
            // Freestanding: emit _start that calls tricti_main (if present) then exit
            let start_ty = self.context.void_type().fn_type(&[], false);
            let start_fn = self.module.add_function("_start", start_ty, None);
            let entry = self.context.append_basic_block(start_fn, "entry");
            self.builder.position_at_end(entry);
            self.current_function = Some(start_fn);

            // Call tricti_main if it exists
            let exit_fn = *self
                .functions
                .get("exit")
                .ok_or_else(|| CodegenError::CompilationError("exit not declared".to_string()))?;

            let code_i32 = if let Some(main_fn) = self.functions.get("tricti_main").cloned() {
                let call = self
                    .builder
                    .build_call(main_fn, &[], "call_tricti_main")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                // If main returns a value, cast to i32; else 0
                if let Some(bv) = call.try_as_basic_value().left() {
                    let iv = if bv.is_int_value() {
                        bv.into_int_value()
                    } else if bv.is_float_value() {
                        self.builder
                            .build_float_to_signed_int(
                                bv.into_float_value(),
                                self.context.i32_type(),
                                "retf2i",
                            )
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    } else {
                        self.context.i32_type().const_zero()
                    };
                    // If not i32, cast
                    if iv.get_type() == self.context.i32_type() {
                        iv
                    } else {
                        self.builder
                            .build_int_cast(iv, self.context.i32_type(), "ret2i32")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    }
                } else {
                    self.context.i32_type().const_zero()
                }
            } else {
                // No tricti_main: run top-level in a minimal block like legacy main()
                let code_alloca =
                    self.create_entry_block_alloca("retcode", self.context.i32_type().into())?;
                self.builder
                    .build_store(code_alloca, self.context.i32_type().const_zero())
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                for statement in &program.statements {
                    let _ = self.generate_statement(statement);
                }
                self.builder
                    .build_load(self.context.i32_type(), code_alloca, "retcode")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_int_value()
            };

            // Call exit(code)
            let args: Vec<BasicMetadataValueEnum> = vec![code_i32.into()];
            self.builder
                .build_call(exit_fn, &args, "exit_call")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            // Ret void to keep verifier happy (exit never returns)
            self.builder
                .build_return(None)
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            return Ok(());
        }

        // Hosted mode: synthesize a C main() that drives either user `main` or top-level statements.
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

        if let Some(main_fn) = self.functions.get("tricti_main").cloned() {
            let call = self
                .builder
                .build_call(main_fn, &[], "call_tricti_main")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            let code_i32 = if let Some(bv) = call.try_as_basic_value().left() {
                let iv = if bv.is_int_value() {
                    bv.into_int_value()
                } else if bv.is_float_value() {
                    self.builder
                        .build_float_to_signed_int(
                            bv.into_float_value(),
                            self.context.i32_type(),
                            "retf2i",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                } else {
                    self.context.i32_type().const_zero()
                };
                if iv.get_type() == self.context.i32_type() {
                    iv
                } else {
                    self.builder
                        .build_int_cast(iv, self.context.i32_type(), "ret2i32")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                }
            } else {
                self.context.i32_type().const_zero()
            };
            self.drop_all_owned_locals()?;
            self.builder
                .build_return(Some(&code_i32))
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        } else {
            let code_alloca =
                self.create_entry_block_alloca("retcode", self.context.i32_type().into())?;
            self.builder
                .build_store(code_alloca, self.context.i32_type().const_zero())
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

            if let Some(stmts) = main_body {
                for s in stmts {
                    let _ = self.generate_statement(s);
                }
            } else {
                for statement in &program.statements {
                    let _ = self.generate_statement(statement);
                }
            }

            let code_i32 = self
                .builder
                .build_load(self.context.i32_type(), code_alloca, "retcode")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                .into_int_value();
            self.drop_all_owned_locals()?;
            self.builder
                .build_return(Some(&code_i32))
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        }

        for (name, func) in &self.functions {
            if name.contains("get") {
                eprintln!(
                    "function {} type {}",
                    name,
                    func.get_type().print_to_string().to_string()
                );
            }
        }

        Ok(())
    }

    fn generate_statement(&mut self, statement: &Statement) -> Result<(), CodegenError> {
        match statement {
            Statement::ConstDecl {
                name,
                type_annotation,
                value,
                ..
            } => {
                // Handle only non-function const expressions here; functions are handled in declare/define passes
                if let ConstValue::Expression(expr) = value {
                    if let Expression::Function { .. } = expr {
                        return Ok(());
                    }
                    // Struct literal special-case when annotated with a known struct type
                    if let (
                        Some(Type::Identifier {
                            name: struct_name,
                            type_args: _,
                        }),
                        Expression::StructLiteral {
                            type_name: _,
                            fields,
                        },
                    ) = (type_annotation, expr)
                    {
                        if let Some((struct_ty, order_clone)) = self
                            .struct_types
                            .get(struct_name)
                            .map(|(st, order)| (*st, order.clone()))
                        {
                            let sval = self.build_struct_literal_value(
                                struct_name,
                                fields,
                                struct_ty,
                                &order_clone,
                            )?;
                            let alloca = self.create_entry_block_alloca(name, struct_ty.into())?;
                            self.builder
                                .build_store(alloca, sval)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.variables
                                .insert(name.clone(), (alloca, struct_ty.into()));
                            return Ok(());
                        }
                    }
                    let mut value_result = self.generate_expression(expr)?;
                    let target_ty = if let Some(ty) = type_annotation {
                        self.map_ast_type(ty).unwrap_or(value_result.get_type())
                    } else if let Some(t) = self.semantic.get_variable_type(name) {
                        self.map_ast_type(t).unwrap_or(value_result.get_type())
                    } else {
                        value_result.get_type()
                    };
                    if value_result.get_type() != target_ty {
                        value_result = self.cast_basic_to_type(value_result, target_ty)?;
                    }
                    let alloca = self.create_entry_block_alloca(name, target_ty)?;
                    self.builder
                        .build_store(alloca, value_result)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.variables.insert(name.clone(), (alloca, target_ty));
                    return Ok(());
                }
                return Ok(());
            }
            Statement::VariableDecl {
                pattern,
                type_annotation,
                value,
            } => {
                if let BindingPattern::Identifier(name) = &pattern {
                    return self.codegen_variable_decl_identifier(
                        name,
                        type_annotation.clone(),
                        value,
                    );
                }

                let value_result = self.generate_expression(value)?;
                self.bind_pattern_value(&pattern, value_result, type_annotation.as_ref())?;
                return Ok(());
            }

            Statement::Assignment { target, value, .. } => {
                if let Expression::Identifier(name) = target {
                    if name == "output" {
                        eprintln!("codegen assignment target identifier output");
                    }
                }
                if matches!(target, Expression::Index { .. }) {
                    eprintln!(
                        "generate assignment target kind: INDEX, value kind: {:?}",
                        value
                    );
                }
                let value_result = self.generate_expression(value)?;

                match target {
                    Expression::Identifier(name) => {
                        let (variable_ptr, var_ty) = match self.variables.get(name) {
                            Some((ptr, ty)) => (*ptr, *ty),
                            None => return Err(CodegenError::UndefinedVariable(name.clone())),
                        };
                        self.drop_current_value(name)?;
                        // Track rank/length if assigning a matrix literal
                        if let Expression::Matrix { rows } = value {
                            let rank = if rows.len() <= 1 { 1 } else { 2 };
                            self.matrix_rank.insert(name.clone(), rank);
                            if rank == 1 {
                                let len = rows.first().map(|r| r.len()).unwrap_or(0) as u64;
                                self.vector_lengths.insert(name.clone(), len);
                            }
                        }
                        // Cast value to variable's type if needed
                        let casted = if value_result.get_type() != var_ty {
                            self.cast_basic_to_type(value_result, var_ty)?
                        } else {
                            value_result
                        };
                        self.builder
                            .build_store(variable_ptr, casted)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.track_owned_binding(name);
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
                                                .ok_or_else(|| {
                                                    CodegenError::InvalidOperation(
                                                        "field index out of range".to_string(),
                                                    )
                                                })?;
                                            // Cast assigned value to field type
                                            let casted = match fty {
                                                BasicTypeEnum::IntType(it) => {
                                                    self.cast_to_int(value_result, it)?.into()
                                                }
                                                BasicTypeEnum::FloatType(ft) => {
                                                    self.cast_to_float(value_result, ft)?.into()
                                                }
                                                BasicTypeEnum::PointerType(pt) => {
                                                    self.cast_to_ptr(value_result, pt)?.into()
                                                }
                                                _ => value_result,
                                            };
                                            let fld_ptr = self
                                                .builder
                                                .build_struct_gep(
                                                    *st, *base_ptr, pos as u32, "fldw",
                                                )
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
                                            self.builder.build_store(fld_ptr, casted).map_err(
                                                |e| CodegenError::CompilationError(e.to_string()),
                                            )?;
                                        }
                                    }
                                } else if base_ty.is_pointer_type() {
                                    // Pointer to struct: use semantic type to locate struct layout and store via loaded pointer
                                    if let Some(struct_name) =
                                        self.semantic_struct_name_of_var(base_name)
                                    {
                                        if let Some((st, order)) =
                                            self.struct_types.get(&struct_name)
                                        {
                                            if let Some(pos) = order.iter().position(|n| n == field)
                                            {
                                                let fty = st
                                                    .get_field_type_at_index(pos as u32)
                                                    .ok_or_else(|| {
                                                        CodegenError::InvalidOperation(
                                                            "field index out of range".to_string(),
                                                        )
                                                    })?;
                                                let casted = match fty {
                                                    BasicTypeEnum::IntType(it) => {
                                                        self.cast_to_int(value_result, it)?.into()
                                                    }
                                                    BasicTypeEnum::FloatType(ft) => {
                                                        self.cast_to_float(value_result, ft)?.into()
                                                    }
                                                    BasicTypeEnum::PointerType(pt) => {
                                                        self.cast_to_ptr(value_result, pt)?.into()
                                                    }
                                                    _ => value_result,
                                                };
                                                let loaded_ptr = *base_ptr;
                                                let fld_ptr = self
                                                    .builder
                                                    .build_struct_gep(
                                                        *st, loaded_ptr, pos as u32, "fldw",
                                                    )
                                                    .map_err(|e| {
                                                        CodegenError::CompilationError(
                                                            e.to_string(),
                                                        )
                                                    })?;
                                                self.builder.build_store(fld_ptr, casted).map_err(
                                                    |e| {
                                                        CodegenError::CompilationError(
                                                            e.to_string(),
                                                        )
                                                    },
                                                )?;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Expression::Index { object, indices } => {
                        let base = self.generate_expression(object)?;
                        if base.is_pointer_value() {
                            let idx_val = if let Some(ix) = indices.get(0) {
                                self.generate_expression(ix)?
                            } else {
                                self.context.i64_type().const_zero().into()
                            };
                            let idx_i64 = self.cast_to_int(idx_val, self.context.i64_type())?;
                            let elem_ptr = unsafe {
                                self.builder.build_in_bounds_gep(
                                    self.context.i64_type(),
                                    base.into_pointer_value(),
                                    &[idx_i64],
                                    "idx",
                                )
                            }
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let val_ty_tmp = value_result.get_type().print_to_string();
                            let val_ty = val_ty_tmp.to_string_lossy();
                            eprintln!("assign index storing value type: {}", val_ty);
                            self.builder
                                .build_store(elem_ptr, value_result)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        }
                    }
                    _ => {}
                }
            }

            Statement::ForLoop {
                variable,
                iterable,
                body,
                ..
            } => {
                // Support: for i in N and for i in start:end[:step]; also iterate elements for 1D matrices (vectors)
                // Try vector element iteration first
                if let Expression::Matrix { rows } = iterable {
                    // Inline literal vector: get pointer and length from literal
                    let base = self.generate_expression(iterable)?;
                    if base.is_pointer_value() {
                        let len = if rows.len() <= 1 {
                            rows.first().map(|r| r.len()).unwrap_or(0)
                        } else {
                            rows.len()
                        } as u64;
                        // idx and loop var allocas
                        let idx_allo =
                            self.create_entry_block_alloca("idx", self.context.i64_type().into())?;
                        self.builder
                            .build_store(idx_allo, self.context.i64_type().const_zero())
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let elem_allo = self
                            .create_entry_block_alloca(variable, self.context.i64_type().into())?;
                        let prev = self.variables.insert(
                            variable.clone(),
                            (elem_allo, self.context.i64_type().into()),
                        );

                        let current_fn = self.current_function.ok_or_else(|| {
                            CodegenError::CompilationError("No current function".to_string())
                        })?;
                        let cond_bb = self.context.append_basic_block(current_fn, "for.cond");
                        let body_bb = self.context.append_basic_block(current_fn, "for.body");
                        let inc_bb = self.context.append_basic_block(current_fn, "for.inc");
                        let end_bb = self.context.append_basic_block(current_fn, "for.end");

                        self.builder
                            .build_unconditional_branch(cond_bb)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        // cond
                        self.builder.position_at_end(cond_bb);
                        let i64_bte: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                        let idx_cur = self
                            .builder
                            .build_load(i64_bte, idx_allo, "idx")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();
                        let endc = self.context.i64_type().const_int(len, false);
                        let cmp = self
                            .builder
                            .build_int_compare(IntPredicate::SLT, idx_cur, endc, "forcmp")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder
                            .build_conditional_branch(cmp, body_bb, end_bb)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        // body
                        self.builder.position_at_end(body_bb);
                        let elem_ptr = unsafe {
                            self.builder.build_in_bounds_gep(
                                self.context.i64_type(),
                                base.into_pointer_value(),
                                &[idx_cur],
                                "idx",
                            )
                        }
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let loaded = self
                            .builder
                            .build_load(self.context.i64_type(), elem_ptr, "elem")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder
                            .build_store(elem_allo, loaded)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        {
                            let _loop_scope = LoopScope::new(&mut self.loop_stack, inc_bb, end_bb);
                            for s in body {
                                let _ = self.generate_statement(s);
                            }
                            self.branch_to(inc_bb)?;
                        }
                        // inc
                        self.builder.position_at_end(inc_bb);
                        let i64_bte2: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                        let idx_cur2 = self
                            .builder
                            .build_load(i64_bte2, idx_allo, "idx")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();
                        let next = self
                            .builder
                            .build_int_add(
                                idx_cur2,
                                self.context.i64_type().const_int(1, false),
                                "inc",
                            )
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder
                            .build_store(idx_allo, next)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder
                            .build_unconditional_branch(cond_bb)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        // end
                        self.builder.position_at_end(end_bb);
                        if let Some(prev_binding) = prev {
                            self.variables.insert(variable.clone(), prev_binding);
                        } else {
                            self.variables.remove(variable);
                        }
                        return Ok(());
                    }
                } else if let Expression::Identifier(name) = iterable {
                    // Variable that may be a vector: use semantic type to get length and load base pointer
                    if let Some(Type::Matrix {
                        element_type: _,
                        dimensions,
                    }) = self.semantic.get_variable_type(name)
                    {
                        let len = if dimensions.is_empty() {
                            0
                        } else {
                            dimensions.iter().product::<usize>()
                        } as u64;
                        if len > 0 {
                            let base_val = self.generate_expression(iterable)?;
                            if base_val.is_pointer_value() {
                                let base = base_val.into_pointer_value();
                                let idx_allo = self.create_entry_block_alloca(
                                    "idx",
                                    self.context.i64_type().into(),
                                )?;
                                self.builder
                                    .build_store(idx_allo, self.context.i64_type().const_zero())
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let elem_allo = self.create_entry_block_alloca(
                                    variable,
                                    self.context.i64_type().into(),
                                )?;
                                let prev = self.variables.insert(
                                    variable.clone(),
                                    (elem_allo, self.context.i64_type().into()),
                                );

                                let current_fn = self.current_function.ok_or_else(|| {
                                    CodegenError::CompilationError(
                                        "No current function".to_string(),
                                    )
                                })?;
                                let cond_bb =
                                    self.context.append_basic_block(current_fn, "for.cond");
                                let body_bb =
                                    self.context.append_basic_block(current_fn, "for.body");
                                let inc_bb = self.context.append_basic_block(current_fn, "for.inc");
                                let end_bb = self.context.append_basic_block(current_fn, "for.end");

                                self.builder
                                    .build_unconditional_branch(cond_bb)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // cond
                                self.builder.position_at_end(cond_bb);
                                let i64_bte3: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                let idx_cur = self
                                    .builder
                                    .build_load(i64_bte3, idx_allo, "idx")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                    .into_int_value();
                                let endc = self.context.i64_type().const_int(len, false);
                                let cmp = self
                                    .builder
                                    .build_int_compare(IntPredicate::SLT, idx_cur, endc, "forcmp")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_conditional_branch(cmp, body_bb, end_bb)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // body
                                self.builder.position_at_end(body_bb);
                                let elem_ptr = unsafe {
                                    self.builder.build_in_bounds_gep(
                                        self.context.i64_type(),
                                        base,
                                        &[idx_cur],
                                        "idx",
                                    )
                                }
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let loaded = self
                                    .builder
                                    .build_load(self.context.i64_type(), elem_ptr, "elem")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_store(elem_allo, loaded)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                {
                                    let _loop_scope =
                                        LoopScope::new(&mut self.loop_stack, inc_bb, end_bb);
                                    for s in body {
                                        let _ = self.generate_statement(s);
                                    }
                                    self.branch_to(inc_bb)?;
                                }
                                // inc
                                self.builder.position_at_end(inc_bb);
                                let i64_bte4: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                let idx_cur2 = self
                                    .builder
                                    .build_load(i64_bte4, idx_allo, "idx")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                    .into_int_value();
                                let next = self
                                    .builder
                                    .build_int_add(
                                        idx_cur2,
                                        self.context.i64_type().const_int(1, false),
                                        "inc",
                                    )
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_store(idx_allo, next)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_unconditional_branch(cond_bb)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // end
                                self.builder.position_at_end(end_bb);
                                if let Some(prev_binding) = prev {
                                    self.variables.insert(variable.clone(), prev_binding);
                                } else {
                                    self.variables.remove(variable);
                                }
                                return Ok(());
                            }
                        }
                    } else if let Some((alloca, var_bte)) = self.variables.get(name) {
                        if let BasicTypeEnum::StructType(st_local) = var_bte {
                            let candidate_slices: [(&str, BasicTypeEnum<'ctx>); 2] = [
                                ("slice_i64", self.context.i64_type().into()),
                                ("slice_bool", self.context.bool_type().into()),
                            ];
                            for (struct_name, elem_ty) in candidate_slices {
                                if let Some((st_known, order)) = self.struct_types.get(struct_name)
                                {
                                    if st_local == st_known {
                                        let order_vec = order.clone();
                                        self.generate_for_loop_over_slice(
                                            variable,
                                            body,
                                            *alloca,
                                            *st_local,
                                            order_vec,
                                            struct_name,
                                            elem_ty,
                                        )?;
                                        return Ok(());
                                    }
                                }
                            }
                        }
                        // else fall through to semantic typing below
                    } else if let Some(Type::Identifier {
                        name: tn,
                        type_args: _,
                    }) = self.semantic.get_variable_type(name)
                    {
                        let candidate_slices: [(&str, BasicTypeEnum<'ctx>); 2] = [
                            ("slice_i64", self.context.i64_type().into()),
                            ("slice_bool", self.context.bool_type().into()),
                        ];
                        for (struct_name, elem_ty) in candidate_slices {
                            if tn == struct_name {
                                if let Some((alloca, var_bte)) = self.variables.get(name) {
                                    if let BasicTypeEnum::StructType(st) = var_bte {
                                        if let Some((_st_known, order)) =
                                            self.struct_types.get(struct_name)
                                        {
                                            let order_vec = order.clone();
                                            self.generate_for_loop_over_slice(
                                                variable,
                                                body,
                                                *alloca,
                                                *st,
                                                order_vec,
                                                struct_name,
                                                elem_ty,
                                            )?;
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                        }
                    } else if let Some(len) = self.vector_lengths.get(name).cloned() {
                        if len > 0 {
                            let base_val = self.generate_expression(iterable)?;
                            if base_val.is_pointer_value() {
                                let base = base_val.into_pointer_value();
                                let idx_allo = self.create_entry_block_alloca(
                                    "idx",
                                    self.context.i64_type().into(),
                                )?;
                                self.builder
                                    .build_store(idx_allo, self.context.i64_type().const_zero())
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let elem_allo = self.create_entry_block_alloca(
                                    variable,
                                    self.context.i64_type().into(),
                                )?;
                                let prev = self.variables.insert(
                                    variable.clone(),
                                    (elem_allo, self.context.i64_type().into()),
                                );

                                let current_fn = self.current_function.ok_or_else(|| {
                                    CodegenError::CompilationError(
                                        "No current function".to_string(),
                                    )
                                })?;
                                let cond_bb =
                                    self.context.append_basic_block(current_fn, "for.cond");
                                let body_bb =
                                    self.context.append_basic_block(current_fn, "for.body");
                                let inc_bb = self.context.append_basic_block(current_fn, "for.inc");
                                let end_bb = self.context.append_basic_block(current_fn, "for.end");

                                self.builder
                                    .build_unconditional_branch(cond_bb)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // cond
                                self.builder.position_at_end(cond_bb);
                                let i64_bte3: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                let idx_cur = self
                                    .builder
                                    .build_load(i64_bte3, idx_allo, "idx")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                    .into_int_value();
                                let endc = self.context.i64_type().const_int(len, false);
                                let cmp = self
                                    .builder
                                    .build_int_compare(IntPredicate::SLT, idx_cur, endc, "forcmp")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_conditional_branch(cmp, body_bb, end_bb)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // body
                                self.builder.position_at_end(body_bb);
                                let elem_ptr = unsafe {
                                    self.builder.build_in_bounds_gep(
                                        self.context.i64_type(),
                                        base,
                                        &[idx_cur],
                                        "idx",
                                    )
                                }
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let loaded = self
                                    .builder
                                    .build_load(self.context.i64_type(), elem_ptr, "elem")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_store(elem_allo, loaded)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                {
                                    let _loop_scope =
                                        LoopScope::new(&mut self.loop_stack, inc_bb, end_bb);
                                    for s in body {
                                        let _ = self.generate_statement(s);
                                    }
                                    self.branch_to(inc_bb)?;
                                }
                                // inc
                                self.builder.position_at_end(inc_bb);
                                let i64_bte4: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                let idx_cur2 = self
                                    .builder
                                    .build_load(i64_bte4, idx_allo, "idx")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                    .into_int_value();
                                let next = self
                                    .builder
                                    .build_int_add(
                                        idx_cur2,
                                        self.context.i64_type().const_int(1, false),
                                        "inc",
                                    )
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_store(idx_allo, next)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_unconditional_branch(cond_bb)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // end
                                self.builder.position_at_end(end_bb);
                                if let Some(prev_binding) = prev {
                                    self.variables.insert(variable.clone(), prev_binding);
                                } else {
                                    self.variables.remove(variable);
                                }
                                return Ok(());
                            }
                        }
                    }
                }

                // Fallback: index-based numeric or range loop
                // Support: for i in N and for i in start:end[:step]
                let (init_val_opt, end_val_opt, step_val_opt) = match iterable {
                    Expression::Literal(Literal::Integer(n)) => {
                        let end_val = self.const_int_from_literal(n)?;
                        (
                            Some(self.context.i64_type().const_zero()),
                            Some(end_val),
                            Some(self.context.i64_type().const_int(1, false)),
                        )
                    }
                    Expression::Identifier(name) => {
                        if let Some((ptr, ty)) = self.variables.get(name) {
                            if let BasicTypeEnum::IntType(i_ty) = ty {
                                let v = self
                                    .builder
                                    .build_load(*ty, *ptr, &format!("{}_end", name))
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let iv = self.cast_to_int(v, *i_ty)?;
                                (
                                    Some(self.context.i64_type().const_zero()),
                                    Some(iv),
                                    Some(self.context.i64_type().const_int(1, false)),
                                )
                            } else {
                                (None, None, None)
                            }
                        } else {
                            (None, None, None)
                        }
                    }
                    Expression::Matrix { rows } => {
                        // iterate over a vector: treat as 0..len
                        let len = if rows.len() <= 1 {
                            rows.first().map(|r| r.len()).unwrap_or(0)
                        } else {
                            rows.len()
                        } as u64;
                        (
                            Some(self.context.i64_type().const_zero()),
                            Some(self.context.i64_type().const_int(len, false)),
                            Some(self.context.i64_type().const_int(1, false)),
                        )
                    }
                    Expression::Range { start, end, step } => {
                        // Try to detect constant negative step from AST (UnaryOp::Negate of integer)
                        let _step_ast_is_neg = matches!(
                            step.as_deref(),
                            Some(Expression::UnaryOp {
                                operator: UnaryOperator::Negate,
                                ..
                            })
                        );
                        let s = self.generate_expression(start)?;
                        let e = self.generate_expression(end)?;
                        let sv = step
                            .as_ref()
                            .map(|x| self.generate_expression(x))
                            .transpose()?;
                        let step_val_any =
                            sv.unwrap_or(self.context.i64_type().const_int(1, false).into());
                        let step_i64 = self.cast_to_int(step_val_any, self.context.i64_type())?;
                        // Stash a marker in the high bit of step when AST says it's negative? Not needed; we'll recompute sign below and also carry the AST hint via a side channel
                        // We can't return extra flag here, so compute sign later using both is_const and AST hint.
                        (
                            Some(self.cast_to_int(s, self.context.i64_type())?),
                            Some(self.cast_to_int(e, self.context.i64_type())?),
                            Some(step_i64),
                        )
                    }
                    _ => (None, None, None),
                };

                if let (Some(init_val), Some(end_val), Some(step_val)) =
                    (init_val_opt, end_val_opt, step_val_opt)
                {
                    // Determine if step is a constant negative value (for comparator selection)
                    let step_is_const_neg = step_val.is_const()
                        && step_val
                            .get_zero_extended_constant()
                            .map(|v| (v as i64) < 0)
                            .unwrap_or(false);
                    // allocate loop var
                    let i_allo =
                        self.create_entry_block_alloca(variable, self.context.i64_type().into())?;
                    self.builder
                        .build_store(i_allo, init_val)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let prev = self
                        .variables
                        .insert(variable.clone(), (i_allo, self.context.i64_type().into()));

                    let current_fn = self.current_function.ok_or_else(|| {
                        CodegenError::CompilationError("No current function".to_string())
                    })?;
                    let cond_bb = self.context.append_basic_block(current_fn, "for.cond");
                    let body_bb = self.context.append_basic_block(current_fn, "for.body");
                    let inc_bb = self.context.append_basic_block(current_fn, "for.inc");
                    let end_bb = self.context.append_basic_block(current_fn, "for.end");

                    // jump to cond
                    self.builder
                        .build_unconditional_branch(cond_bb)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    // cond
                    self.builder.position_at_end(cond_bb);
                    let i64_bte: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                    let cur_i = self
                        .builder
                        .build_load(i64_bte, i_allo, "i")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                        .into_int_value();
                    // Condition depends on step sign. If step is a known negative constant, use cur_i > end; if unknown at compile-time, branch on step<0 to choose comparator.
                    // Prefer constant sign if available; else if non-const but AST hinted negative via unary, treat as negative
                    let step_ast_neg_hint = match iterable {
                        Expression::Range { step, .. } => matches!(
                            step.as_deref(),
                            Some(Expression::UnaryOp {
                                operator: UnaryOperator::Negate,
                                ..
                            })
                        ),
                        _ => false,
                    };
                    let cmp = if step_is_const_neg || step_ast_neg_hint {
                        // exclusive: i > end
                        self.builder
                            .build_int_compare(IntPredicate::SGT, cur_i, end_val, "forcmp")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    } else if step_val.is_const() {
                        // exclusive: i < end
                        self.builder
                            .build_int_compare(IntPredicate::SLT, cur_i, end_val, "forcmp")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    } else {
                        // dynamic step: choose comparator blocks
                        let current_fn = self.current_function.ok_or_else(|| {
                            CodegenError::CompilationError("No current function".to_string())
                        })?;
                        let neg_bb = self.context.append_basic_block(current_fn, "for.cond.neg");
                        let pos_bb = self.context.append_basic_block(current_fn, "for.cond.pos");
                        let join_bb = self.context.append_basic_block(current_fn, "for.cond.join");
                        let is_neg = self
                            .builder
                            .build_int_compare(
                                IntPredicate::SLT,
                                step_val,
                                self.context.i64_type().const_zero(),
                                "isneg",
                            )
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder
                            .build_conditional_branch(is_neg, neg_bb, pos_bb)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        // neg path
                        self.builder.position_at_end(neg_bb);
                        let cmp_neg = self
                            .builder
                            .build_int_compare(IntPredicate::SGT, cur_i, end_val, "cmpneg")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder
                            .build_unconditional_branch(join_bb)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let neg_end = self.builder.get_insert_block().unwrap();
                        // pos path
                        self.builder.position_at_end(pos_bb);
                        let cmp_pos = self
                            .builder
                            .build_int_compare(IntPredicate::SLT, cur_i, end_val, "cmppos")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder
                            .build_unconditional_branch(join_bb)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let pos_end = self.builder.get_insert_block().unwrap();
                        // join
                        self.builder.position_at_end(join_bb);
                        let phi = self
                            .builder
                            .build_phi(self.context.bool_type(), "forcmpphi")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let neg_bv: BasicValueEnum<'ctx> = cmp_neg.into();
                        let pos_bv: BasicValueEnum<'ctx> = cmp_pos.into();
                        phi.add_incoming(&[(&neg_bv, neg_end), (&pos_bv, pos_end)]);
                        phi.as_basic_value().into_int_value()
                    };
                    self.builder
                        .build_conditional_branch(cmp, body_bb, end_bb)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    // body
                    self.builder.position_at_end(body_bb);
                    {
                        let _loop_scope = LoopScope::new(&mut self.loop_stack, inc_bb, end_bb);
                        for s in body {
                            let _ = self.generate_statement(s);
                        }
                        self.branch_to(inc_bb)?;
                    }

                    // inc
                    self.builder.position_at_end(inc_bb);
                    let cur_i2 = self
                        .builder
                        .build_load(i64_bte, i_allo, "i")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                        .into_int_value();
                    let next = self
                        .builder
                        .build_int_add(cur_i2, step_val, "inc")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_store(i_allo, next)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_unconditional_branch(cond_bb)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    // end
                    self.builder.position_at_end(end_bb);
                    if let Some(prev_binding) = prev {
                        self.variables.insert(variable.clone(), prev_binding);
                    } else {
                        self.variables.remove(variable);
                    }
                } else {
                    // unsupported iterable; skip
                }
            }

            Statement::Expression(expr) => {
                self.generate_expression(expr)?;
            }
            Statement::ModuleDecl {
                name: _,
                items,
                is_public: _,
            } => {
                if let Some(stmts) = items {
                    for s in stmts {
                        let _ = self.generate_statement(s);
                    }
                }
            }

            Statement::Return(expr) => {
                let trace_return = std::env::var("TRACE_RETURN").is_ok();
                if trace_return {
                    eprintln!("codegen return stmt: {:?}", expr);
                }
                if let Some(expr) = expr {
                    if trace_return {
                        eprintln!("TRACE_RETURN evaluating return expression");
                    }
                    let value = match self.generate_expression(expr) {
                        Ok(v) => v,
                        Err(err) => {
                            if trace_return {
                                eprintln!("TRACE_RETURN return expression codegen failed: {}", err);
                            }
                            return Err(err);
                        }
                    };
                    if trace_return {
                        eprintln!("TRACE_RETURN expression evaluated, building return");
                        eprintln!(
                            "return expr evaluated to type {}",
                            value.get_type().print_to_string().to_string()
                        );
                    }
                    self.mark_expr_moved(expr);
                    if trace_return {
                        eprintln!("TRACE_RETURN dropping owned locals before return");
                    }
                    self.drop_all_owned_locals()?;
                    if trace_return {
                        eprintln!("TRACE_RETURN emitting LLVM return");
                    }
                    self.builder
                        .build_return(Some(&value))
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    if trace_return {
                        if let Some(block) = self.builder.get_insert_block() {
                            eprintln!(
                                "post-return block name: {}",
                                block.get_name().to_string_lossy()
                            );
                            eprintln!(
                                "after build_return, terminator present? {}",
                                block.get_terminator().is_some()
                            );
                        }
                    }
                } else {
                    self.drop_all_owned_locals()?;
                    let ret_ast = self
                        .current_function_return_ast
                        .clone()
                        .unwrap_or(crate::ast::Type::None);
                    if let Some(ret_ty) = self.map_ast_type(&ret_ast) {
                        let default_value = self.default_value_for_type(ret_ty);
                        self.builder
                            .build_return(Some(&default_value))
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    } else {
                        self.builder
                            .build_return(None)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    }
                }
            }

            Statement::Break(expr) => {
                if let Some(expr) = expr {
                    let _ = self.generate_expression(expr)?;
                }
                if let Some(ctx) = self.loop_stack.last() {
                    self.branch_to(ctx.break_bb)?;
                }
            }

            Statement::Continue => {
                if let Some(ctx) = self.loop_stack.last() {
                    self.branch_to(ctx.continue_bb)?;
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
            let fty = st.get_field_type_at_index(i as u32).ok_or_else(|| {
                CodegenError::InvalidOperation("field index out of range".to_string())
            })?;
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
                        let zero_alloca =
                            self.create_entry_block_alloca("zero_struct", st2.into())?;
                        let bte: BasicTypeEnum<'ctx> = st2.into();
                        self.builder
                            .build_load(bte, zero_alloca, "zst")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
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
            let gep = self
                .builder
                .build_struct_gep(st, tmp, i as u32, "fldw")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            self.builder
                .build_store(gep, casted)
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        }
        // Load as value
        let bte: BasicTypeEnum<'ctx> = st.into();
        let loaded = self
            .builder
            .build_load(bte, tmp, "tmp_load")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        Ok(loaded)
    }

    fn try_construct_enum_struct_literal(
        &mut self,
        struct_name: &str,
        fields: &HashMap<String, Expression>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let Some((enum_name, variant_name)) = struct_name.rsplit_once('_') else {
            return Ok(None);
        };
        let Some(Type::Enum { variants, order }) = self.semantic.types.get(enum_name) else {
            return Ok(None);
        };
        let Some(idx) = order.iter().position(|candidate| candidate == variant_name) else {
            return Ok(None);
        };
        let Some(Some(payload_ty)) = variants.get(variant_name) else {
            return Ok(None);
        };
        let resolved_payload = self.semantic.resolve_type(payload_ty);
        match resolved_payload {
            Type::Struct { .. } => {
                let enum_ty = self.enum_struct.ok_or_else(|| {
                    CodegenError::CompilationError(format!(
                        "enum representation type unavailable for {}",
                        enum_name
                    ))
                })?;
                let tag_val = self.context.i64_type().const_int(idx as u64, false);
                let (struct_ty, field_order) = self.ensure_struct_type_by_name(struct_name)?;
                let struct_val =
                    self.build_struct_literal_value(struct_name, fields, struct_ty, &field_order)?;
                let payload_alloca = self.create_entry_block_alloca(
                    &format!("{}_payload", struct_name),
                    struct_ty.into(),
                )?;
                self.builder
                    .build_store(payload_alloca, struct_val)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let payload_ptr_int = self
                    .builder
                    .build_ptr_to_int(
                        payload_alloca,
                        self.context.i64_type(),
                        "enum_struct_payload_ptr",
                    )
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let with_tag = self
                    .builder
                    .build_insert_value(enum_ty.get_undef(), tag_val, 0, "enum_tag")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();
                let with_payload = self
                    .builder
                    .build_insert_value(with_tag, payload_ptr_int, 1, "enum_payload")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();
                Ok(Some(with_payload.as_basic_value_enum()))
            }
            _ => Ok(None),
        }
    }

    #[allow(dead_code)]
    fn load_tuple_from_value(&mut self, value: BasicValueEnum<'ctx>) -> Option<StructValue<'ctx>> {
        if value.is_struct_value() {
            Some(value.into_struct_value())
        } else {
            None
        }
    }

    #[allow(dead_code)]
    fn evaluate_tuple_pattern(
        &mut self,
        tuple_val: StructValue<'ctx>,
        tuple_ty: StructType<'ctx>,
        items: &[Expression],
    ) -> Result<
        (
            Vec<IntValue<'ctx>>,
            Vec<(String, Vec<u32>, BasicTypeEnum<'ctx>)>,
        ),
        CodegenError,
    > {
        if tuple_ty.count_fields() != items.len() as u32 {
            return Err(CodegenError::InvalidOperation(
                "tuple pattern arity mismatch".to_string(),
            ));
        }

        let mut conds = Vec::new();
        let mut bindings = Vec::new();
        self.evaluate_tuple_pattern_inner(
            tuple_val,
            tuple_ty,
            items,
            &[],
            &mut conds,
            &mut bindings,
        )?;
        Ok((conds, bindings))
    }

    #[allow(dead_code)]
    fn evaluate_tuple_pattern_inner(
        &mut self,
        tuple_val: StructValue<'ctx>,
        tuple_ty: StructType<'ctx>,
        items: &[Expression],
        prefix: &[u32],
        conds: &mut Vec<IntValue<'ctx>>,
        bindings: &mut Vec<(String, Vec<u32>, BasicTypeEnum<'ctx>)>,
    ) -> Result<(), CodegenError> {
        for (idx, item) in items.iter().enumerate() {
            let field_ty = tuple_ty
                .get_field_type_at_index(idx as u32)
                .ok_or_else(|| {
                    CodegenError::InvalidOperation("tuple field index out of range".to_string())
                })?;
            let field_val = self
                .builder
                .build_extract_value(tuple_val, idx as u32, &format!("tuple_elem{}", idx))
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

            let mut path: Vec<u32> = prefix.to_vec();
            path.push(idx as u32);

            match item {
                Expression::Identifier(name) if name == "_" => {}
                Expression::Identifier(name) => {
                    bindings.push((name.clone(), path, field_ty));
                }
                Expression::Literal(Literal::Integer(lit)) => {
                    if let BasicTypeEnum::IntType(_it) = field_ty {
                        let tuple_iv = field_val.into_int_value();
                        let literal_i64 = self.const_int_from_literal(lit)?;
                        let lit_cast = literal_i64;
                        let cmp = self
                            .builder
                            .build_int_compare(
                                IntPredicate::EQ,
                                tuple_iv,
                                lit_cast,
                                &format!("tuple_cmp{}_int", idx),
                            )
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        conds.push(cmp);
                    } else {
                        return Err(CodegenError::InvalidOperation(
                            "integer literal in tuple pattern requires integer field".to_string(),
                        ));
                    }
                }
                Expression::Literal(Literal::Boolean(value)) => {
                    if let BasicTypeEnum::IntType(it) = field_ty {
                        if it.get_bit_width() != 1 {
                            return Err(CodegenError::InvalidOperation(
                                "boolean literal in tuple pattern requires bool field".to_string(),
                            ));
                        }
                        let tuple_iv = field_val.into_int_value();
                        let literal_bool = self.context.bool_type().const_int(*value as u64, false);
                        let cmp = self
                            .builder
                            .build_int_compare(
                                IntPredicate::EQ,
                                tuple_iv,
                                literal_bool,
                                &format!("tuple_cmp{}_bool", idx),
                            )
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        conds.push(cmp);
                    } else {
                        return Err(CodegenError::InvalidOperation(
                            "boolean literal in tuple pattern requires bool field".to_string(),
                        ));
                    }
                }
                Expression::Literal(Literal::Char(ch)) => {
                    if let BasicTypeEnum::IntType(it) = field_ty {
                        let tuple_iv = field_val.into_int_value();
                        let literal_char = it.const_int(*ch as u64, false);
                        let cmp = self
                            .builder
                            .build_int_compare(
                                IntPredicate::EQ,
                                tuple_iv,
                                literal_char,
                                &format!("tuple_cmp{}_char", idx),
                            )
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        conds.push(cmp);
                    } else {
                        return Err(CodegenError::InvalidOperation(
                            "char literal in tuple pattern requires integer field".to_string(),
                        ));
                    }
                }
                Expression::Tuple(sub_items) => {
                    let sub_ty = match field_ty {
                        BasicTypeEnum::StructType(st) => st,
                        _ => {
                            return Err(CodegenError::InvalidOperation(
                                "nested tuple pattern requires tuple value".to_string(),
                            ))
                        }
                    };
                    let sub_val = field_val.into_struct_value();
                    self.evaluate_tuple_pattern_inner(
                        sub_val, sub_ty, sub_items, &path, conds, bindings,
                    )?;
                }
                _ => {
                    return Err(CodegenError::InvalidOperation(
                        "unsupported pattern in tuple".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn generate_tuple_match(
        &mut self,
        scrutinee: BasicValueEnum<'ctx>,
        arms: &[MatchArm],
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if !scrutinee.is_struct_value() {
            return Err(CodegenError::InvalidOperation(
                "tuple match requires tuple value".to_string(),
            ));
        }

        let tuple_val = scrutinee.into_struct_value();
        let tuple_ty = tuple_val.get_type();
        let tuple_alloca =
            self.create_entry_block_alloca("match_tuple", tuple_ty.as_basic_type_enum())?;
        self.builder
            .build_store(tuple_alloca, tuple_val.as_basic_value_enum())
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        let current_fn = self
            .current_function
            .ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;

        let mut arm_blocks: Vec<BasicBlock<'ctx>> = Vec::with_capacity(arms.len());
        let mut binding_records: Vec<Vec<(String, Vec<u32>, BasicTypeEnum<'ctx>)>> =
            vec![Vec::new(); arms.len()];
        for i in 0..arms.len() {
            let bb = self
                .context
                .append_basic_block(current_fn, &format!("match.arm{}", i));
            arm_blocks.push(bb);
        }
        let default_bb = self.context.append_basic_block(current_fn, "match.default");
        let cont_bb = self.context.append_basic_block(current_fn, "match.cont");

        let mut next_cmp_block: Option<BasicBlock<'ctx>> = None;
        for (i, arm) in arms.iter().enumerate() {
            let cmp_block = match next_cmp_block {
                Some(bb) => bb,
                None => self.builder.get_insert_block().ok_or_else(|| {
                    CodegenError::CompilationError(
                        "missing insertion block for tuple match".to_string(),
                    )
                })?,
            };
            self.builder.position_at_end(cmp_block);

            let next_cmp_bb = if i + 1 < arms.len() {
                self.context
                    .append_basic_block(current_fn, &format!("match.cmp{}", i + 1))
            } else {
                default_bb
            };

            let cond_value = match &arm.pattern {
                Expression::Identifier(name) if name == "_" => {
                    self.context.bool_type().const_int(1, false)
                }
                Expression::Identifier(name) => {
                    binding_records[i].push((name.clone(), Vec::new(), tuple_ty.into()));
                    self.context.bool_type().const_int(1, false)
                }
                Expression::Tuple(items) => {
                    let loaded = self
                        .builder
                        .build_load(
                            tuple_ty,
                            tuple_alloca,
                            &format!("match_tuple_cmp{}_load", i),
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                        .into_struct_value();
                    let mut conds: Vec<IntValue<'ctx>> = Vec::new();
                    let mut bindings: Vec<(String, Vec<u32>, BasicTypeEnum<'ctx>)> = Vec::new();
                    self.evaluate_tuple_pattern_inner(
                        loaded,
                        tuple_ty,
                        items,
                        &[],
                        &mut conds,
                        &mut bindings,
                    )?;
                    binding_records[i] = bindings;
                    if conds.is_empty() {
                        self.context.bool_type().const_int(1, false)
                    } else {
                        let mut iter = conds.into_iter();
                        let mut current = iter.next().unwrap();
                        for (and_idx, cond) in iter.enumerate() {
                            current = self
                                .builder
                                .build_and(
                                    current,
                                    cond,
                                    &format!("tuple_match_and{}_{}", i, and_idx),
                                )
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        }
                        current
                    }
                }
                _ => {
                    return Err(CodegenError::InvalidOperation(
                        "unsupported pattern in tuple match".to_string(),
                    ));
                }
            };

            self.builder
                .build_conditional_branch(cond_value, arm_blocks[i], next_cmp_bb)
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            next_cmp_block = Some(next_cmp_bb);
        }

        let mut incoming: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        for (i, arm) in arms.iter().enumerate() {
            self.builder.position_at_end(arm_blocks[i]);
            let mut saved_bindings: Vec<(
                String,
                Option<(PointerValue<'ctx>, BasicTypeEnum<'ctx>)>,
            )> = Vec::new();

            if !binding_records[i].is_empty() {
                let loaded = self
                    .builder
                    .build_load(
                        tuple_ty,
                        tuple_alloca,
                        &format!("match_tuple_arm{}_load", i),
                    )
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();
                for (binding_idx, (name, path, field_ty)) in binding_records[i].iter().enumerate() {
                    let value = self.extract_tuple_path_value(
                        loaded,
                        path,
                        &format!("tuple_bind{}_{}", i, binding_idx),
                    )?;
                    let alloca = self.create_entry_block_alloca(name, *field_ty)?;
                    self.builder
                        .build_store(alloca, value)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let previous = self.variables.insert(name.clone(), (alloca, *field_ty));
                    saved_bindings.push((name.clone(), previous));
                }
            }

            let body_val_raw = self.generate_expression(&arm.body)?;
            let body_val = if body_val_raw.is_int_value() {
                let int_val = body_val_raw.into_int_value();
                if int_val.get_type() == self.context.i64_type() {
                    int_val.into()
                } else {
                    self.builder
                        .build_int_cast(
                            int_val,
                            self.context.i64_type(),
                            &format!("match_arm_cast{}", i),
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                        .into()
                }
            } else {
                self.cast_to_int(body_val_raw, self.context.i64_type())?
                    .into()
            };

            let arm_block = self.builder.get_insert_block().ok_or_else(|| {
                CodegenError::CompilationError(
                    "missing arm block after tuple match arm".to_string(),
                )
            })?;
            let mut flows_to_cont = false;
            if arm_block.get_terminator().is_none() {
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                flows_to_cont = true;
            }
            if flows_to_cont {
                incoming.push((body_val, arm_block));
            }

            for (name, previous) in saved_bindings.into_iter().rev() {
                if let Some(binding) = previous {
                    self.variables.insert(name, binding);
                } else {
                    self.variables.remove(&name);
                }
            }
        }

        self.builder.position_at_end(default_bb);
        let default_val: BasicValueEnum<'ctx> = self.context.i64_type().const_zero().into();
        let default_block = self.builder.get_insert_block().unwrap();
        let mut default_flows = false;
        if default_block.get_terminator().is_none() {
            self.builder
                .build_unconditional_branch(cont_bb)
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            default_flows = true;
        }
        if default_flows {
            incoming.push((default_val, default_block));
        }

        self.builder.position_at_end(cont_bb);
        if incoming.is_empty() {
            Ok(self.context.i64_type().const_zero().into())
        } else {
            let phi = self
                .builder
                .build_phi(self.context.i64_type(), "matchtmp")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            let incoming_dyn: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = incoming
                .iter()
                .map(|(v, bb)| (v as &dyn BasicValue<'ctx>, *bb))
                .collect();
            phi.add_incoming(&incoming_dyn);
            Ok(phi.as_basic_value())
        }
    }

    fn try_generate_question_else_match(
        &mut self,
        raw: BasicValueEnum<'ctx>,
        arms: &[MatchArm],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        if arms.len() != 2 {
            return Ok(None);
        }

        let (some_arm, none_arm) = (&arms[0], &arms[1]);

        let capture_name = match &some_arm.pattern {
            Expression::Call {
                function,
                type_args: _,
                arguments,
            } => {
                if !matches!(function.as_ref(), Expression::Identifier(name) if name == "some") {
                    return Ok(None);
                }
                if arguments.len() != 1 {
                    return Ok(None);
                }
                if let Expression::Identifier(ident) = &arguments[0].value {
                    ident.clone()
                } else {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        };

        if !matches!(&none_arm.pattern, Expression::Identifier(name) if name == "none") {
            return Ok(None);
        }

        if !matches!(&some_arm.body, Expression::Identifier(name) if name == "__tri_question_value")
        {
            return Ok(None);
        }

        let enum_ty = match self.enum_struct {
            Some(st) => st,
            None => return Ok(None),
        };

        let current_fn = match self.current_function {
            Some(func) => func,
            None => {
                return Err(CodegenError::CompilationError(
                    "question-else match outside function".to_string(),
                ))
            }
        };

        let opt_struct = if raw.is_struct_value() {
            raw.into_struct_value()
        } else if raw.is_pointer_value() {
            self.builder
                .build_load(enum_ty, raw.into_pointer_value(), "question_else_load")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                .into_struct_value()
        } else {
            return Ok(None);
        };

        let opt_alloca = self.create_entry_block_alloca("question_else_opt", enum_ty.into())?;
        self.builder
            .build_store(opt_alloca, opt_struct)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        let tag_ptr = self
            .builder
            .build_struct_gep(enum_ty, opt_alloca, 0, "question_else_tag_ptr")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let tag_val = self
            .builder
            .build_load(self.context.i64_type(), tag_ptr, "question_else_tag")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let payload_ptr = self
            .builder
            .build_struct_gep(enum_ty, opt_alloca, 1, "question_else_payload_ptr")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let payload_raw = self
            .builder
            .build_load(
                self.context.i64_type(),
                payload_ptr,
                "question_else_payload",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let none_tag = self.get_pattern_tag(&none_arm.pattern)?;

        let some_bb = self
            .context
            .append_basic_block(current_fn, "question_else.some");
        let none_bb = self
            .context
            .append_basic_block(current_fn, "question_else.none");
        let cont_bb = self
            .context
            .append_basic_block(current_fn, "question_else.cont");

        let is_none = self
            .builder
            .build_int_compare(IntPredicate::EQ, tag_val, none_tag, "question_else_is_none")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_conditional_branch(is_none, none_bb, some_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        let mut incoming: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        let mut phi_result_ty: Option<BasicTypeEnum<'ctx>> = None;

        // Some branch
        self.builder.position_at_end(some_bb);
        let payload_ty = payload_raw.get_type();
        let payload_alloca = self
            .create_entry_block_alloca(&capture_name, payload_ty)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(payload_alloca, payload_raw)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let previous_binding = self
            .variables
            .insert(capture_name.clone(), (payload_alloca, payload_ty));
        let some_val_raw = self.generate_expression(&some_arm.body)?;
        if let Some(prev) = previous_binding {
            self.variables.insert(capture_name.clone(), prev);
        } else {
            self.variables.remove(&capture_name);
        }
        let some_block = self.builder.get_insert_block().unwrap_or(some_bb);
        if some_block.get_terminator().is_none() {
            let mut casted = some_val_raw;
            if let Some(target_ty) = phi_result_ty {
                if casted.get_type() != target_ty {
                    casted = self.cast_basic_to_type(casted, target_ty)?;
                }
            } else {
                phi_result_ty = Some(casted.get_type());
            }
            self.builder
                .build_unconditional_branch(cont_bb)
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            incoming.push((casted, some_block));
        }

        // None branch
        self.builder.position_at_end(none_bb);
        let none_val_raw = self.generate_expression(&none_arm.body)?;
        let none_block = self.builder.get_insert_block().unwrap_or(none_bb);
        if none_block.get_terminator().is_none() {
            let mut casted = none_val_raw;
            if let Some(target_ty) = phi_result_ty {
                if casted.get_type() != target_ty {
                    casted = self.cast_basic_to_type(casted, target_ty)?;
                }
            } else {
                phi_result_ty = Some(casted.get_type());
            }
            self.builder
                .build_unconditional_branch(cont_bb)
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            incoming.push((casted, none_block));
        }

        if incoming.is_empty() {
            self.builder.position_at_end(cont_bb);
            let zero = match phi_result_ty {
                Some(BasicTypeEnum::IntType(it)) => it.const_zero().into(),
                Some(BasicTypeEnum::FloatType(ft)) => ft.const_zero().into(),
                Some(BasicTypeEnum::PointerType(pt)) => pt.const_null().into(),
                Some(BasicTypeEnum::StructType(st)) => st.get_undef().into(),
                Some(BasicTypeEnum::VectorType(vt)) => vt.const_zero().into(),
                Some(BasicTypeEnum::ArrayType(at)) => at.const_zero().into(),
                Some(_) | None => self.context.i64_type().const_zero().into(),
            };
            return Ok(Some(zero));
        }

        self.builder.position_at_end(cont_bb);
        let phi = self
            .builder
            .build_phi(
                phi_result_ty.unwrap_or(self.context.i64_type().into()),
                "question_else_result",
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let incoming_dyn: Vec<(&dyn inkwell::values::BasicValue<'ctx>, BasicBlock<'ctx>)> =
            incoming
                .iter()
                .map(|(v, bb)| (v as &dyn inkwell::values::BasicValue<'ctx>, *bb))
                .collect();
        phi.add_incoming(&incoming_dyn);
        Ok(Some(phi.as_basic_value()))
    }

    fn extract_tuple_path_value(
        &mut self,
        tuple_val: StructValue<'ctx>,
        path: &[u32],
        base_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if path.is_empty() {
            return Ok(tuple_val.as_basic_value_enum());
        }

        let mut current = tuple_val;
        for (depth, index) in path.iter().enumerate() {
            let extracted = self
                .builder
                .build_extract_value(
                    current,
                    *index,
                    &format!("{}_{}_{}", base_name, depth, index),
                )
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            if depth + 1 == path.len() {
                return Ok(extracted);
            }
            if extracted.is_struct_value() {
                current = extracted.into_struct_value();
            } else {
                return Err(CodegenError::InvalidOperation(
                    "tuple binding path does not resolve to tuple".to_string(),
                ));
            }
        }

        Err(CodegenError::InvalidOperation(
            "invalid tuple binding path".to_string(),
        ))
    }

    fn generate_expression(
        &mut self,
        expr: &Expression,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match expr {
            Expression::Literal(literal) => self.generate_literal(literal),
            Expression::Tuple(items) => {
                let mut element_values: Vec<BasicValueEnum<'ctx>> = Vec::with_capacity(items.len());
                let mut element_types: Vec<BasicTypeEnum<'ctx>> = Vec::with_capacity(items.len());
                for (idx, item) in items.iter().enumerate() {
                    let value = self.generate_expression(item)?;
                    let value_ty = value.get_type();
                    element_values.push(value);
                    element_types.push(value_ty);
                    if element_values.len() != element_types.len() {
                        return Err(CodegenError::InvalidOperation(format!(
                            "failed to gather element {} for tuple literal",
                            idx
                        )));
                    }
                }

                let struct_ty = if element_types.is_empty() {
                    self.context.struct_type(&[], false)
                } else {
                    self.context.struct_type(&element_types, false)
                };

                let mut aggregate: StructValue<'ctx> = struct_ty.get_undef();
                for (idx, value) in element_values.into_iter().enumerate() {
                    aggregate = self
                        .builder
                        .build_insert_value(
                            aggregate,
                            value,
                            idx as u32,
                            &format!("tuple_elem{}", idx),
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                        .into_struct_value();
                }

                Ok(aggregate.as_basic_value_enum())
            }
            Expression::StructLiteral { type_name, fields } => {
                if let Some(struct_name) = type_name {
                    if let Some(enum_value) =
                        self.try_construct_enum_struct_literal(struct_name, fields)?
                    {
                        return Ok(enum_value);
                    }

                    let (struct_ty, field_order) = match self.struct_types.get(struct_name) {
                        Some((struct_ty, order)) => (*struct_ty, order.clone()),
                        None => self.ensure_struct_type_by_name(struct_name)?,
                    };

                    let struct_val = self.build_struct_literal_value(
                        struct_name,
                        fields,
                        struct_ty,
                        &field_order,
                    )?;
                    Ok(struct_val)
                } else {
                    let mut candidate: Option<(String, StructType<'ctx>, Vec<String>)> = None;
                    let mut ambiguous = false;
                    for (struct_name, (struct_ty, order)) in &self.struct_types {
                        if order.len() == fields.len()
                            && order
                                .iter()
                                .all(|field_name| fields.contains_key(field_name))
                        {
                            let entry = (struct_name.clone(), *struct_ty, order.clone());
                            if candidate.is_some() {
                                ambiguous = true;
                                break;
                            }
                            candidate = Some(entry);
                        }
                    }
                    if !ambiguous {
                        if let Some((struct_name, struct_ty, field_order)) = candidate {
                            let struct_val = self.build_struct_literal_value(
                                &struct_name,
                                fields,
                                struct_ty,
                                &field_order,
                            )?;
                            return Ok(struct_val);
                        }
                    }
                    Ok(self.context.i64_type().const_zero().into())
                }
            }
            Expression::VecNew {
                length,
                fill,
                additional_dimensions,
                ..
            } => {
                let (vec_struct, data_idx, len_idx, cap_idx) = self.vector_field_indices()?;
                let i64_ty = self.context.i64_type();
                let ptr_ty = self.context.ptr_type(AddressSpace::default());

                let mut total_len: Option<IntValue<'ctx>> = None;
                if let Some(len_expr) = length.as_ref() {
                    let dim_val = self.generate_expression(len_expr.as_ref())?;
                    let dim_i64 = self.cast_to_int(dim_val, i64_ty)?;
                    total_len = Some(dim_i64);
                }
                for (idx, dim_expr) in additional_dimensions.iter().enumerate() {
                    let dim_val = self.generate_expression(dim_expr)?;
                    let dim_i64 = self.cast_to_int(dim_val, i64_ty)?;
                    total_len = Some(match total_len {
                        Some(acc) => self
                            .builder
                            .build_int_mul(acc, dim_i64, &format!("dimprod{}", idx))
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?,
                        None => dim_i64,
                    });
                }

                let len_value = total_len.unwrap_or_else(|| i64_ty.const_zero());
                let capacity_extra = i64_ty.const_int(4, false);
                let capacity_value = self
                    .builder
                    .build_int_add(len_value, capacity_extra, "vec_capacity_init")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                let elem_size = i64_ty.const_int(8, false);
                let size_bytes = self
                    .builder
                    .build_int_mul(capacity_value, elem_size, "vec_size_bytes")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                let alloc_fn = self
                    .functions
                    .get("alloc")
                    .cloned()
                    .ok_or_else(|| CodegenError::UndefinedFunction("alloc".to_string()))?;
                let call = self
                    .builder
                    .build_call(alloc_fn, &[size_bytes.into()], "vec_alloc")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let raw_ptr = call
                    .try_as_basic_value()
                    .left()
                    .ok_or_else(|| {
                        CodegenError::InvalidOperation(
                            "alloc returned void when pointer expected".to_string(),
                        )
                    })?
                    .into_pointer_value();
                let data_ptr = self
                    .builder
                    .build_pointer_cast(raw_ptr, ptr_ty, "vec_data_ptr")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                let mut vec_value = vec_struct.get_undef();
                vec_value = self
                    .builder
                    .build_insert_value(vec_value, data_ptr, data_idx, "vec_set_data")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();
                vec_value = self
                    .builder
                    .build_insert_value(vec_value, len_value, len_idx, "vec_set_len")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();
                vec_value = self
                    .builder
                    .build_insert_value(vec_value, capacity_value, cap_idx, "vec_set_cap")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();

                if let Some(fill_expr) = fill.as_ref() {
                    let fill_val = self.generate_expression(fill_expr.as_ref())?;
                    let fill_i64 = self.cast_to_int(fill_val, i64_ty)?;

                    let current_fn = self.current_function.ok_or_else(|| {
                        CodegenError::CompilationError(
                            "Vec::new fill used outside of function".to_string(),
                        )
                    })?;
                    let idx_alloca =
                        self.create_entry_block_alloca("vec_fill_idx", i64_ty.into())?;
                    self.builder
                        .build_store(idx_alloca, i64_ty.const_zero())
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    let cond_bb = self.context.append_basic_block(current_fn, "vec_fill.cond");
                    let body_bb = self.context.append_basic_block(current_fn, "vec_fill.body");
                    let inc_bb = self.context.append_basic_block(current_fn, "vec_fill.inc");
                    let end_bb = self.context.append_basic_block(current_fn, "vec_fill.end");

                    self.builder
                        .build_unconditional_branch(cond_bb)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    self.builder.position_at_end(cond_bb);
                    let idx_cur = self
                        .builder
                        .build_load(i64_ty, idx_alloca, "vec_fill_idx")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                        .into_int_value();
                    let cmp = self
                        .builder
                        .build_int_compare(IntPredicate::ULT, idx_cur, len_value, "vec_fill_cmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_conditional_branch(cmp, body_bb, end_bb)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    self.builder.position_at_end(body_bb);
                    let elem_ptr = unsafe {
                        self.builder.build_in_bounds_gep(
                            i64_ty,
                            data_ptr,
                            &[idx_cur],
                            "vec_fill_ptr",
                        )
                    }
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_store(elem_ptr, fill_i64)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_unconditional_branch(inc_bb)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    self.builder.position_at_end(inc_bb);
                    let next = self
                        .builder
                        .build_int_add(idx_cur, i64_ty.const_int(1, false), "vec_fill_inc")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_store(idx_alloca, next)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_unconditional_branch(cond_bb)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                    self.builder.position_at_end(end_bb);
                }

                Ok(vec_value.into())
            }
            Expression::VecLiteral { elements } => {
                let (vec_struct, data_idx, len_idx, cap_idx) = self.vector_field_indices()?;
                let i64_ty = self.context.i64_type();
                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                let n = elements.len();
                let len_val = i64_ty.const_int(n as u64, false);
                let capacity_val = if n == 0 {
                    i64_ty.const_int(4, false)
                } else {
                    len_val
                };
                let elem_size = i64_ty.const_int(8, false);
                let size_bytes = self
                    .builder
                    .build_int_mul(capacity_val, elem_size, "vec_literal_bytes")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                let alloc_fn = self
                    .functions
                    .get("alloc")
                    .cloned()
                    .ok_or_else(|| CodegenError::UndefinedFunction("alloc".to_string()))?;
                let call = self
                    .builder
                    .build_call(alloc_fn, &[size_bytes.into()], "vec_literal_alloc")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let raw_ptr = call
                    .try_as_basic_value()
                    .left()
                    .ok_or_else(|| {
                        CodegenError::InvalidOperation(
                            "alloc returned void when pointer expected".to_string(),
                        )
                    })?
                    .into_pointer_value();
                let data_ptr = self
                    .builder
                    .build_pointer_cast(raw_ptr, ptr_ty, "vec_literal_ptr")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                for (i, expr) in elements.iter().enumerate() {
                    let v = self.generate_expression(expr)?;
                    let iv = self.cast_to_int(v, i64_ty)?;
                    let offset = i64_ty.const_int(i as u64, false);
                    let elem_ptr = unsafe {
                        self.builder.build_in_bounds_gep(
                            i64_ty,
                            data_ptr,
                            &[offset],
                            "vec_lit_elem",
                        )
                    }
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_store(elem_ptr, iv)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                }

                let mut vec_value = vec_struct.get_undef();
                vec_value = self
                    .builder
                    .build_insert_value(vec_value, data_ptr, data_idx, "vec_lit_data")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();
                vec_value = self
                    .builder
                    .build_insert_value(vec_value, len_val, len_idx, "vec_lit_len")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();
                vec_value = self
                    .builder
                    .build_insert_value(vec_value, capacity_val, cap_idx, "vec_lit_cap")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();

                Ok(vec_value.into())
            }
            Expression::Matrix { rows } => {
                let (vec_struct, data_idx, len_idx, cap_idx) = self.vector_field_indices()?;
                let i64_ty = self.context.i64_type();
                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                if rows.len() > 1 {
                    let null_vec = {
                        let null_ptr = ptr_ty.const_null();
                        let zero = i64_ty.const_zero();
                        let mut vec_value = vec_struct.get_undef();
                        vec_value = self
                            .builder
                            .build_insert_value(vec_value, null_ptr, data_idx, "mat_null_data")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_struct_value();
                        vec_value = self
                            .builder
                            .build_insert_value(vec_value, zero, len_idx, "mat_null_len")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_struct_value();
                        self.builder
                            .build_insert_value(vec_value, zero, cap_idx, "mat_null_cap")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_struct_value()
                    };
                    return Ok(null_vec.into());
                }
                let empty: [Expression; 0] = [];
                let row0: &[Expression] = rows.get(0).map(|v| v.as_slice()).unwrap_or(&empty);
                let n = row0.len();
                let len_val = i64_ty.const_int(n as u64, false);
                let capacity_val = if n == 0 {
                    i64_ty.const_int(4, false)
                } else {
                    len_val
                };
                let elem_size = i64_ty.const_int(8, false);
                let size_bytes = self
                    .builder
                    .build_int_mul(capacity_val, elem_size, "vec_matrix_bytes")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                let alloc_fn = self
                    .functions
                    .get("alloc")
                    .cloned()
                    .ok_or_else(|| CodegenError::UndefinedFunction("alloc".to_string()))?;
                let call = self
                    .builder
                    .build_call(alloc_fn, &[size_bytes.into()], "vec_matrix_alloc")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let raw_ptr = call
                    .try_as_basic_value()
                    .left()
                    .ok_or_else(|| {
                        CodegenError::InvalidOperation(
                            "alloc returned void when pointer expected".to_string(),
                        )
                    })?
                    .into_pointer_value();
                let data_ptr = self
                    .builder
                    .build_pointer_cast(raw_ptr, ptr_ty, "vec_matrix_ptr")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                for (i, expr) in row0.iter().enumerate() {
                    let v = self.generate_expression(expr)?;
                    let iv = self.cast_to_int(v, i64_ty)?;
                    let offset = i64_ty.const_int(i as u64, false);
                    let elem_ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(i64_ty, data_ptr, &[offset], "mat_elem")
                    }
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_store(elem_ptr, iv)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                }

                let mut vec_value = vec_struct.get_undef();
                vec_value = self
                    .builder
                    .build_insert_value(vec_value, data_ptr, data_idx, "mat_data")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();
                vec_value = self
                    .builder
                    .build_insert_value(vec_value, len_val, len_idx, "mat_len")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();
                vec_value = self
                    .builder
                    .build_insert_value(vec_value, capacity_val, cap_idx, "mat_cap")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_struct_value();

                Ok(vec_value.into())
            }
            Expression::Index { object, indices } => {
                let idx_val = if let Some(ix) = indices.get(0) {
                    self.generate_expression(ix)?
                } else {
                    self.context.i64_type().const_zero().into()
                };
                let idx_i64 = self.cast_to_int(idx_val, self.context.i64_type())?;

                let (vec_struct, data_idx, _, _) = self.vector_field_indices()?;
                let data_ptr_opt = if let Expression::Identifier(var_name) = object.as_ref() {
                    if let Some((alloca, stored_ty)) = self.variables.get(var_name) {
                        if let BasicTypeEnum::StructType(st) = stored_ty {
                            if *st == vec_struct {
                                let vec_val = self
                                    .builder
                                    .build_load(*stored_ty, *alloca, "vec_index_load")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                    .into_struct_value();
                                let data_field = self
                                    .builder
                                    .build_extract_value(vec_val, data_idx, "vec_index_data")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                Some(data_field.into_pointer_value())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                let data_ptr = if let Some(ptr) = data_ptr_opt {
                    ptr
                } else {
                    let base = self.generate_expression(object)?;
                    if base.is_pointer_value() {
                        base.into_pointer_value()
                    } else {
                        return Ok(self.context.i64_type().const_zero().into());
                    }
                };

                let elem_ptr = unsafe {
                    self.builder.build_in_bounds_gep(
                        self.context.i64_type(),
                        data_ptr,
                        &[idx_i64],
                        "idx",
                    )
                }
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let loaded = self
                    .builder
                    .build_load(self.context.i64_type(), elem_ptr, "loadidx")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                Ok(loaded)
            }

            Expression::Identifier(name) => {
                // If this is an enum variant reference like Type_Variant, generate its struct {tag, payload}.
                if let Some((tname, vname)) = name.split_once('_') {
                    if let Some(Type::Enum { variants, order }) = self.semantic.types.get(tname) {
                        if let Some(variant_ty) = variants.get(vname) {
                            let idx = order.iter().position(|s| s == vname).unwrap_or(0) as u64;
                            if *variant_ty == None {
                                // No payload, return tag as i64
                                return Ok(self.context.i64_type().const_int(idx, false).into());
                            } else {
                                // With payload, return struct { tag, payload: 0 }
                                let tag = self.context.i64_type().const_int(idx, false);
                                let payload = self.context.i64_type().const_zero();
                                let struct_val = self
                                    .context
                                    .const_struct(&[tag.into(), payload.into()], false);
                                return Ok(struct_val.into());
                            }
                        }
                    }
                }
                if name == "none" {
                    if let Some(enum_ty) = self.enum_struct {
                        let tag = self.context.i64_type().const_zero();
                        let payload = self.context.i64_type().const_zero();
                        let struct_val = enum_ty.const_named_struct(&[tag.into(), payload.into()]);
                        return Ok(struct_val.into());
                    } else {
                        return Ok(self.context.i64_type().const_zero().into());
                    }
                }
                let (ptr, ty) = self
                    .variables
                    .get(name)
                    .ok_or_else(|| CodegenError::UndefinedVariable(name.clone()))?;
                let loaded = self
                    .builder
                    .build_load(*ty, *ptr, name)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                Ok(loaded)
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
                                        let loaded_ptr = self
                                            .builder
                                            .build_load(*var_ty, *ptr, "selfloadptr")
                                            .map_err(|e| {
                                                CodegenError::CompilationError(e.to_string())
                                            })?;
                                        if let Some(pos) = order.iter().position(|n| n == field) {
                                            let fty = st
                                                .get_field_type_at_index(pos as u32)
                                                .ok_or_else(|| {
                                                    CodegenError::InvalidOperation(
                                                        "field index out of range".to_string(),
                                                    )
                                                })?;
                                            let field_ptr = self
                                                .builder
                                                .build_struct_gep(
                                                    *st,
                                                    loaded_ptr.into_pointer_value(),
                                                    pos as u32,
                                                    "fld",
                                                )
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
                                            let val = self
                                                .builder
                                                .build_load(fty, field_ptr, "fldv")
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
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
                            let (idx, fty) = if let Some((_, (_llvm_st, order))) = self
                                .struct_types
                                .iter()
                                .find(|(_, (llvm_st, _))| llvm_st == st)
                            {
                                if let Some(pos) = order.iter().position(|n| n == field) {
                                    let fty = st.get_field_type_at_index(pos as u32).ok_or_else(
                                        || {
                                            CodegenError::InvalidOperation(
                                                "field index out of range".to_string(),
                                            )
                                        },
                                    )?;
                                    (pos as u32, fty)
                                } else {
                                    return Ok(self.context.i64_type().const_zero().into());
                                }
                            } else {
                                return Ok(self.context.i64_type().const_zero().into());
                            };
                            let field_ptr = self
                                .builder
                                .build_struct_gep(*st, *ptr, idx, "fld")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let val = self
                                .builder
                                .build_load(fty, field_ptr, "fldv")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            Ok(val)
                        } else if var_ty.is_pointer_type() {
                            // Use semantic struct type to compute GEP on loaded pointer
                            if let Some(struct_name) = self.semantic_struct_name_of_var(var_name) {
                                if let Some((st, order)) = self.struct_types.get(&struct_name) {
                                    if let Some(pos) = order.iter().position(|n| n == field) {
                                        let loaded_ptr = *ptr;
                                        let fty = st
                                            .get_field_type_at_index(pos as u32)
                                            .ok_or_else(|| {
                                                CodegenError::InvalidOperation(
                                                    "field index out of range".to_string(),
                                                )
                                            })?;
                                        let field_ptr = self
                                            .builder
                                            .build_struct_gep(*st, loaded_ptr, pos as u32, "fld")
                                            .map_err(|e| {
                                                CodegenError::CompilationError(e.to_string())
                                            })?;
                                        let val = self
                                            .builder
                                            .build_load(fty, field_ptr, "fldv")
                                            .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
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
                    for s in prefix {
                        let _ = self.generate_statement(s);
                    }
                    if let Statement::Expression(expr) = last {
                        return self.generate_expression(expr);
                    } else {
                        let _ = self.generate_statement(last);
                    }
                }
                Ok(self.context.i64_type().const_zero().into())
            }

            Expression::IfExpr {
                condition,
                then_expr,
                else_expr,
            } => {
                let mut then_branch = Vec::new();
                then_branch.push(Statement::Expression(*then_expr.clone()));
                let else_branch = else_expr
                    .as_ref()
                    .map(|expr| vec![Statement::Expression(*expr.clone())]);
                let synthetic_if = Expression::If {
                    condition: condition.clone(),
                    then_branch,
                    else_branch,
                };
                self.generate_expression(&synthetic_if)
            }

            Expression::If {
                condition,
                then_branch,
                else_branch,
            } => {
                if std::env::var("TRACE_RETURN").is_ok() {
                    eprintln!(
                        "codegen if: then {} stmts, else present? {}",
                        then_branch.len(),
                        else_branch.as_ref().map(|b| b.len()).unwrap_or(0)
                    );
                }
                // Evaluate condition to i1
                let cond_val = self.generate_expression(condition)?;
                let cond_bool = if cond_val.is_int_value() {
                    let zero = cond_val.get_type().into_int_type().const_zero();
                    self.builder
                        .build_int_compare(
                            IntPredicate::NE,
                            cond_val.into_int_value(),
                            zero,
                            "ifcond",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                } else if cond_val.is_float_value() {
                    let zero = cond_val.get_type().into_float_type().const_zero();
                    self.builder
                        .build_float_compare(
                            FloatPredicate::ONE,
                            cond_val.into_float_value(),
                            zero,
                            "ifcond",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                } else if cond_val.is_pointer_value() {
                    self.builder
                        .build_is_not_null(cond_val.into_pointer_value(), "ifcond")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                } else {
                    self.context.bool_type().const_zero()
                };

                let current_fn = self.current_function.ok_or_else(|| {
                    CodegenError::CompilationError("No current function".to_string())
                })?;
                let then_bb = self.context.append_basic_block(current_fn, "then");
                let else_bb = self.context.append_basic_block(current_fn, "else");
                let mut cont_bb: Option<inkwell::basic_block::BasicBlock> = None;

                // Always branch to an explicit else block for SSA merging
                self.builder
                    .build_conditional_branch(cond_bool, then_bb, else_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                // then branch
                self.builder.position_at_end(then_bb);
                // Generate THEN branch statements; stop if a terminator is emitted
                let mut then_val: BasicValueEnum<'ctx> =
                    self.context.i64_type().const_zero().into();
                if !then_branch.is_empty() {
                    for (idx, stmt) in then_branch.iter().enumerate() {
                        let is_last = idx + 1 == then_branch.len();
                        match (is_last, stmt) {
                            (true, Statement::Expression(expr)) => {
                                // Only compute a value if the block hasn't terminated
                                if self
                                    .builder
                                    .get_insert_block()
                                    .unwrap()
                                    .get_terminator()
                                    .is_none()
                                {
                                    let v = self.generate_expression(expr)?;
                                    then_val = self.cast_to_int(v, self.context.i64_type())?.into();
                                }
                            }
                            _ => {
                                let _ = self.generate_statement(stmt);
                            }
                        }
                        if self
                            .builder
                            .get_insert_block()
                            .unwrap()
                            .get_terminator()
                            .is_some()
                        {
                            break;
                        }
                    }
                }
                let then_block_after = self.builder.get_insert_block().unwrap();
                let mut then_flows = false;
                if then_block_after.get_terminator().is_none() {
                    // Create continuation block on demand
                    let cont = match cont_bb {
                        Some(bb) => bb,
                        None => {
                            let bb = self.context.append_basic_block(current_fn, "ifcont");
                            cont_bb = Some(bb);
                            bb
                        }
                    };
                    self.builder
                        .build_unconditional_branch(cont)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    then_flows = true;
                }
                let then_end_bb = self.builder.get_insert_block().unwrap();

                // else branch
                self.builder.position_at_end(else_bb);
                // Generate ELSE branch
                let mut else_val: BasicValueEnum<'ctx> =
                    self.context.i64_type().const_zero().into();
                if let Some(else_stmts) = else_branch {
                    if !else_stmts.is_empty() {
                        for (idx, stmt) in else_stmts.iter().enumerate() {
                            let is_last = idx + 1 == else_stmts.len();
                            match (is_last, stmt) {
                                (true, Statement::Expression(expr)) => {
                                    if self
                                        .builder
                                        .get_insert_block()
                                        .unwrap()
                                        .get_terminator()
                                        .is_none()
                                    {
                                        let v = self.generate_expression(expr)?;
                                        else_val =
                                            self.cast_to_int(v, self.context.i64_type())?.into();
                                    }
                                }
                                _ => {
                                    let _ = self.generate_statement(stmt);
                                }
                            }
                            if self
                                .builder
                                .get_insert_block()
                                .unwrap()
                                .get_terminator()
                                .is_some()
                            {
                                break;
                            }
                        }
                    }
                }
                let else_block_after = self.builder.get_insert_block().unwrap();
                let mut else_flows = false;
                if else_block_after.get_terminator().is_none() {
                    // Create continuation block on demand
                    let cont = match cont_bb {
                        Some(bb) => bb,
                        None => {
                            let bb = self.context.append_basic_block(current_fn, "ifcont");
                            cont_bb = Some(bb);
                            bb
                        }
                    };
                    self.builder
                        .build_unconditional_branch(cont)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    else_flows = true;
                }
                let else_end_bb = self.builder.get_insert_block().unwrap();

                // continuation with phi merge
                if let Some(cont) = cont_bb {
                    // At least one branch flows to cont
                    self.builder.position_at_end(cont);
                    let phi = self
                        .builder
                        .build_phi(self.context.i64_type(), "iftmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let mut incomings: Vec<(
                        &dyn inkwell::values::BasicValue<'ctx>,
                        inkwell::basic_block::BasicBlock,
                    )> = Vec::new();
                    if then_flows {
                        incomings.push((
                            &then_val as &dyn inkwell::values::BasicValue<'ctx>,
                            then_end_bb,
                        ));
                    }
                    if else_flows {
                        incomings.push((
                            &else_val as &dyn inkwell::values::BasicValue<'ctx>,
                            else_end_bb,
                        ));
                    }
                    phi.add_incoming(&incomings);
                    Ok(phi.as_basic_value())
                } else {
                    // Both branches terminated (e.g., return); no continuation block created.
                    Ok(self.context.i64_type().const_zero().into())
                }
            }

            Expression::Match { value, arms } => {
                // Evaluate scrutinee once
                let raw = self.generate_expression(value)?;

                if arms
                    .iter()
                    .any(|arm| matches!(arm.pattern, Expression::Tuple(_)))
                {
                    return self.generate_tuple_match(raw, arms);
                }

                let mut requires_enum_unpack = false;
                for arm in arms {
                    if self.pattern_requires_payload(&arm.pattern) {
                        requires_enum_unpack = true;
                        break;
                    }
                }

                // Evaluate scrutinee and extract tag as i64 for comparisons (enum/default path)
                let enum_ty_opt = self.enum_struct;
                let mut temp_alloca: Option<PointerValue<'ctx>> = None;
                let scrut = if raw.is_struct_value() {
                    let enum_ty = enum_ty_opt.ok_or_else(|| {
                        CodegenError::CompilationError(
                            "enum struct type unavailable for match".to_string(),
                        )
                    })?;
                    let temp = self.create_entry_block_alloca("match_scrut", enum_ty.into())?;
                    self.builder
                        .build_store(temp, raw)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    temp_alloca = Some(temp);
                    let tag_ptr = self
                        .builder
                        .build_struct_gep(enum_ty, temp, 0, "match_tag_ptr")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_load(self.context.i64_type(), tag_ptr, "match_tag")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                        .into_int_value()
                } else if requires_enum_unpack {
                    let enum_ty = enum_ty_opt.ok_or_else(|| {
                        CodegenError::CompilationError(
                            "enum struct type unavailable for match".to_string(),
                        )
                    })?;
                    let enum_ptr_ty = enum_ty.ptr_type(AddressSpace::default());
                    let enum_ptr = if raw.is_pointer_value() {
                        raw.into_pointer_value()
                    } else {
                        let raw_int = if raw.is_int_value() {
                            raw.into_int_value()
                        } else {
                            self.cast_to_int(raw, self.context.i64_type())?
                        };
                        self.builder
                            .build_int_to_ptr(raw_int, enum_ptr_ty, "match_enum_ptr")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    };
                    let loaded = self
                        .builder
                        .build_load(enum_ty, enum_ptr, "match_enum_load")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let temp = self.create_entry_block_alloca("match_scrut", enum_ty.into())?;
                    self.builder
                        .build_store(temp, loaded)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    temp_alloca = Some(temp);
                    let tag_ptr = self
                        .builder
                        .build_struct_gep(enum_ty, temp, 0, "match_tag_ptr")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_load(self.context.i64_type(), tag_ptr, "match_tag")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                        .into_int_value()
                } else if raw.is_int_value() {
                    raw.into_int_value()
                } else {
                    self.cast_to_int(raw, self.context.i64_type())?
                };

                if let Some(question_else_value) =
                    self.try_generate_question_else_match(raw, arms)?
                {
                    return Ok(question_else_value);
                }

                let current_fn = self.current_function.ok_or_else(|| {
                    CodegenError::CompilationError("No current function".to_string())
                })?;

                // Create blocks: one per arm, plus default and merge
                let mut arm_blocks: Vec<(
                    inkwell::basic_block::BasicBlock,
                    Option<inkwell::values::BasicValueEnum>,
                )> = Vec::new();
                for i in 0..arms.len() {
                    let bb = self
                        .context
                        .append_basic_block(current_fn, &format!("match.arm{}", i));
                    arm_blocks.push((bb, None));
                }
                let default_bb = self.context.append_basic_block(current_fn, "match.default");
                let cont_bb = self.context.append_basic_block(current_fn, "match.cont");

                // Build comparison chain
                let mut next_block = None;
                for (i, arm) in arms.iter().enumerate() {
                    let cmp_bb = match next_block {
                        Some(bb) => bb,
                        None => self.builder.get_insert_block().unwrap(),
                    };
                    // Position at comparison end to emit branch
                    self.builder.position_at_end(cmp_bb);
                    // Wildcard '_' matches unconditionally
                    if let Expression::Identifier(ref s) = arm.pattern {
                        if s == "_" {
                            self.builder
                                .build_unconditional_branch(arm_blocks[i].0)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // No further comparisons; set next to default but it won't be reached
                            next_block = Some(default_bb);
                            continue;
                        }
                    }
                    // Get the tag for this arm
                    let arm_tag = self.get_pattern_tag(&arm.pattern)?;
                    let is_eq = self
                        .builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            scrut,
                            arm_tag,
                            &format!("matchcmp{}", i),
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let next_cmp_bb = if i + 1 < arms.len() {
                        self.context
                            .append_basic_block(current_fn, &format!("match.cmp{}", i + 1))
                    } else {
                        default_bb
                    };
                    self.builder
                        .build_conditional_branch(is_eq, arm_blocks[i].0, next_cmp_bb)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    next_block = Some(next_cmp_bb);
                }

                // Emit each arm body -> branch to cont
                let mut incoming: Vec<(
                    inkwell::values::BasicValueEnum,
                    inkwell::basic_block::BasicBlock,
                )> = Vec::new();
                for (i, arm) in arms.iter().enumerate() {
                    self.builder.position_at_end(arm_blocks[i].0);
                    let arm_returns = Self::expression_contains_return(&arm.body);
                    let mut saved_bindings: Vec<(
                        String,
                        Option<(PointerValue<'ctx>, BasicTypeEnum<'ctx>)>,
                    )> = Vec::new();
                    // Bind variables from pattern
                    if let Expression::Call { arguments, .. } = &arm.pattern {
                        if self.pattern_requires_payload(&arm.pattern) {
                            for arg in arguments.iter() {
                                if let Expression::Identifier(var_name) = &arg.value {
                                    if var_name != "_" && var_name != "none" {
                                        let payload_val: BasicValueEnum<'ctx> =
                                            if let Some(enum_alloca) = temp_alloca {
                                                let payload_ptr = self
                                                    .builder
                                                    .build_struct_gep(
                                                        self.enum_struct.unwrap(),
                                                        enum_alloca,
                                                        1,
                                                        "payload_ptr",
                                                    )
                                                    .map_err(|e| {
                                                        CodegenError::CompilationError(
                                                            e.to_string(),
                                                        )
                                                    })?;
                                                self.builder
                                                    .build_load(
                                                        self.context.i64_type(),
                                                        payload_ptr,
                                                        "payload",
                                                    )
                                                    .map_err(|e| {
                                                        CodegenError::CompilationError(
                                                            e.to_string(),
                                                        )
                                                    })?
                                            } else {
                                                self.context.i64_type().const_zero().into()
                                            };
                                        let payload_ty: BasicTypeEnum<'ctx> =
                                            self.context.i64_type().into();
                                        let var_alloca =
                                            self.create_entry_block_alloca(var_name, payload_ty)?;
                                        self.builder.build_store(var_alloca, payload_val).map_err(
                                            |e| CodegenError::CompilationError(e.to_string()),
                                        )?;
                                        let previous = self
                                            .variables
                                            .insert(var_name.clone(), (var_alloca, payload_ty));
                                        saved_bindings.push((var_name.clone(), previous));
                                    }
                                }
                            }
                        }
                    }
                    if let Expression::StructLiteral { type_name, fields } = &arm.pattern {
                        if !fields.is_empty() {
                            let enum_ty = self.enum_struct.ok_or_else(|| {
                                CodegenError::CompilationError(
                                    "enum struct missing for struct pattern".to_string(),
                                )
                            })?;
                            let enum_alloca = temp_alloca.ok_or_else(|| {
                                CodegenError::CompilationError(
                                    "enum payload unavailable for struct pattern".to_string(),
                                )
                            })?;
                            let payload_ptr = self
                                .builder
                                .build_struct_gep(enum_ty, enum_alloca, 1, "struct_payload_ptr")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let payload_raw = self
                                .builder
                                .build_load(self.context.i64_type(), payload_ptr, "struct_payload")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into_int_value();

                            let resolved = if let Some(name) = type_name {
                                match self.ensure_struct_type_by_name(name) {
                                    Ok((st, order)) => Some((name.clone(), st, order)),
                                    Err(_) => None,
                                }
                            } else {
                                let mut candidate: Option<(String, StructType<'ctx>, Vec<String>)> =
                                    None;
                                for (struct_name, (llvm_ty, order)) in &self.struct_types {
                                    if order.len() == fields.len()
                                        && order.iter().all(|fname| fields.contains_key(fname))
                                    {
                                        candidate =
                                            Some((struct_name.clone(), *llvm_ty, order.clone()));
                                        break;
                                    }
                                }
                                candidate
                            };

                            if let Some((_struct_name, struct_ty, field_order)) = resolved {
                                let struct_ptr_ty = struct_ty.ptr_type(AddressSpace::default());
                                let payload_ptr_cast = self
                                    .builder
                                    .build_int_to_ptr(
                                        payload_raw,
                                        struct_ptr_ty,
                                        "struct_payload_cast",
                                    )
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let payload_struct_val = self
                                    .builder
                                    .build_load(struct_ty, payload_ptr_cast, "struct_payload_val")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                    .into_struct_value();

                                for (field_name, pattern_expr) in fields {
                                    let Some(position) = field_order
                                        .iter()
                                        .position(|candidate| candidate == field_name)
                                    else {
                                        return Err(CodegenError::CompilationError(format!(
                                            "unknown field '{}' in struct pattern",
                                            field_name
                                        )));
                                    };

                                    let field_val = self
                                        .builder
                                        .build_extract_value(
                                            payload_struct_val,
                                            position as u32,
                                            &format!("match_field_{}", field_name),
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;

                                    if let Expression::Identifier(var_name) = pattern_expr {
                                        if var_name == "_" {
                                            continue;
                                        }
                                        let field_ty = struct_ty
                                            .get_field_type_at_index(position as u32)
                                            .ok_or_else(|| {
                                                CodegenError::CompilationError(
                                                    "struct field index out of range".to_string(),
                                                )
                                            })?;
                                        let var_alloca =
                                            self.create_entry_block_alloca(var_name, field_ty)?;
                                        self.builder.build_store(var_alloca, field_val).map_err(
                                            |e| CodegenError::CompilationError(e.to_string()),
                                        )?;
                                        let previous = self
                                            .variables
                                            .insert(var_name.clone(), (var_alloca, field_ty));
                                        saved_bindings.push((var_name.clone(), previous));
                                    } else {
                                        return Err(CodegenError::CompilationError(format!(
                                            "unsupported pattern {:?} for struct field '{}'",
                                            pattern_expr, field_name
                                        )));
                                    }
                                }
                            } else {
                                return Err(CodegenError::CompilationError(format!(
                                    "unable to resolve struct type for pattern {:?}",
                                    type_name
                                )));
                            }
                        }
                    }
                    let body_val_raw = self.generate_expression(&arm.body)?;
                    let body_val: BasicValueEnum<'ctx> = if body_val_raw.is_int_value() {
                        let int_val = body_val_raw.into_int_value();
                        if int_val.get_type() == self.context.i64_type() {
                            int_val.into()
                        } else {
                            self.builder
                                .build_int_cast(int_val, self.context.i64_type(), "match_arm_cast")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into()
                        }
                    } else {
                        self.cast_to_int(body_val_raw, self.context.i64_type())?
                            .into()
                    };
                    let arm_block = self.builder.get_insert_block().unwrap_or(arm_blocks[i].0);
                    let mut flows_to_cont = false;
                    if !arm_returns && arm_block.get_terminator().is_none() {
                        self.builder
                            .build_unconditional_branch(cont_bb)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        flows_to_cont = true;
                    }
                    if !arm_returns && flows_to_cont {
                        incoming.push((body_val, arm_block));
                    }

                    for (name, previous) in saved_bindings.into_iter().rev() {
                        match previous {
                            Some(binding) => {
                                self.variables.insert(name, binding);
                            }
                            None => {
                                self.variables.remove(&name);
                            }
                        }
                    }
                }

                // Default yields 0
                self.builder.position_at_end(default_bb);
                let def_val: BasicValueEnum<'ctx> = self.context.i64_type().const_zero().into();
                let default_block = self.builder.get_insert_block().unwrap();
                let mut default_flows = false;
                if default_block.get_terminator().is_none() {
                    self.builder
                        .build_unconditional_branch(cont_bb)
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    default_flows = true;
                }
                if default_flows {
                    incoming.push((def_val, default_block));
                }

                // Merge with phi
                self.builder.position_at_end(cont_bb);
                if incoming.is_empty() {
                    Ok(self.context.i64_type().const_zero().into())
                } else {
                    let phi = self
                        .builder
                        .build_phi(self.context.i64_type(), "matchtmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    // Convert to expected slice of (&dyn BasicValue, BasicBlock)
                    let incoming_dyn: Vec<(
                        &dyn inkwell::values::BasicValue<'ctx>,
                        inkwell::basic_block::BasicBlock,
                    )> = incoming
                        .iter()
                        .map(|(v, bb)| (v as &dyn inkwell::values::BasicValue<'ctx>, *bb))
                        .collect();
                    phi.add_incoming(&incoming_dyn);
                    Ok(phi.as_basic_value())
                }
            }

            Expression::BinaryOp {
                left,
                operator,
                right,
            } => {
                let prev_ctx = self.current_binary_context.take();
                if matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual) {
                    self.current_binary_context =
                        Some(format!("{:?} {:?} {:?}", left, operator, right));
                }
                if std::env::var("TRACE_BINARY").is_ok() {
                    eprintln!("generate_expression BinaryOp operator: {:?}", operator);
                }
                let result = if matches!(operator, BinaryOperator::And | BinaryOperator::Or) {
                    let left_val = self.generate_expression(left)?;
                    self.generate_short_circuit_binary(operator, left_val, right)?
                } else {
                    let left_val = self.generate_expression(left)?;
                    let right_val = self.generate_expression(right)?;
                    if matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual) {
                        eprintln!(
                            "binary operands types: {} vs {}",
                            left_val.get_type().print_to_string().to_string(),
                            right_val.get_type().print_to_string().to_string()
                        );
                    }
                    let operands_unsigned =
                        self.expression_is_unsigned(left) && self.expression_is_unsigned(right);
                    self.generate_binary_op(left_val, operator, right_val, operands_unsigned)?
                };
                self.current_binary_context = prev_ctx;
                Ok(result)
            }

            Expression::UnaryOp { operator, operand } => {
                match operator {
                    UnaryOperator::AddressOf => {
                        // Special case: get address of the operand
                        match operand.as_ref() {
                            Expression::Identifier(name) => {
                                if let Some((ptr, _)) = self.variables.get(name) {
                                    Ok((*ptr).into())
                                } else {
                                    Err(CodegenError::UndefinedVariable(name.clone()))
                                }
                            }
                            _ => {
                                let value = self.generate_expression(operand)?;
                                let value_ty = value.get_type();
                                let temp_alloca =
                                    self.create_entry_block_alloca("addr_tmp", value_ty)?;
                                self.builder
                                    .build_store(temp_alloca, value)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                Ok(temp_alloca.into())
                            }
                        }
                    }
                    _ => {
                        let operand_val = self.generate_expression(operand)?;
                        self.generate_unary_op(operator, operand_val)
                    }
                }
            }

            Expression::Cast { value, to_type } => {
                let val = self.generate_expression(value)?;
                self.cast_value(val, to_type)
            }

            Expression::Question(inner) => {
                let source_val = self.generate_expression(inner)?;
                let enum_ty = self.enum_struct.ok_or_else(|| {
                    CodegenError::CompilationError(
                        "enum representation not available for question operator".to_string(),
                    )
                })?;
                let enum_ptr_ty = enum_ty.ptr_type(AddressSpace::default());
                let current_fn = self.current_function.ok_or_else(|| {
                    CodegenError::CompilationError(
                        "question operator used outside of a function".to_string(),
                    )
                })?;

                let result_struct = if source_val.is_struct_value() {
                    let sv = source_val.into_struct_value();
                    if sv.get_type() == enum_ty {
                        sv
                    } else {
                        let src_ty: BasicTypeEnum<'ctx> = sv.get_type().into();
                        let tmp_src = self
                            .create_entry_block_alloca("question_src", src_ty)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder
                            .build_store(tmp_src, sv)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let cast_ptr = self
                            .builder
                            .build_pointer_cast(tmp_src, enum_ptr_ty, "question_src_cast")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder
                            .build_load(enum_ty, cast_ptr, "question_load_from_struct")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_struct_value()
                    }
                } else if source_val.is_pointer_value() {
                    let ptr_val = source_val.into_pointer_value();
                    let cast_ptr = self
                        .builder
                        .build_pointer_cast(ptr_val, enum_ptr_ty, "question_ptr_cast")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder
                        .build_load(enum_ty, cast_ptr, "question_load")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                        .into_struct_value()
                } else {
                    return Err(CodegenError::InvalidOperation(
                        "question operator requires optional or result value".to_string(),
                    ));
                };

                let temp_alloca = self.create_entry_block_alloca("question_tmp", enum_ty.into())?;
                self.builder
                    .build_store(temp_alloca, result_struct)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                let tag_ptr = self
                    .builder
                    .build_struct_gep(enum_ty, temp_alloca, 0, "question_tag_ptr")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let tag_val = self
                    .builder
                    .build_load(self.context.i64_type(), tag_ptr, "question_tag")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    .into_int_value();

                let ret_ast = self.current_function_return_ast.clone().ok_or_else(|| {
                    CodegenError::CompilationError(
                        "question operator requires function return type".to_string(),
                    )
                })?;
                let ret_llvm_ty = self.map_ast_type(&ret_ast).ok_or_else(|| {
                    CodegenError::CompilationError(
                        "unable to determine LLVM type for function return".to_string(),
                    )
                })?;
                let (
                    is_result,
                    payload_target_ast,
                    success_predicate,
                    success_label,
                    failure_label,
                ) = match &ret_ast {
                    Type::Optional { inner } => (
                        false,
                        inner.as_ref().clone(),
                        IntPredicate::NE,
                        "question.some",
                        "question.none",
                    ),
                    Type::Result { inner } => (
                        true,
                        inner.as_ref().clone(),
                        IntPredicate::EQ,
                        "question.ok",
                        "question.err",
                    ),
                    _ => {
                        return Err(CodegenError::InvalidOperation(
                            "question operator requires optional or result return type".to_string(),
                        ))
                    }
                };
                let payload_target_ty = self.map_ast_type(&payload_target_ast);

                let zero = self.context.i64_type().const_zero();
                let is_success = self
                    .builder
                    .build_int_compare(success_predicate, tag_val, zero, "question_is_success")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                let success_bb = self.context.append_basic_block(current_fn, success_label);
                let failure_bb = self.context.append_basic_block(current_fn, failure_label);
                let cont_bb = self.context.append_basic_block(current_fn, "question.cont");

                self.builder
                    .build_conditional_branch(is_success, success_bb, failure_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                self.builder.position_at_end(success_bb);
                let payload_ptr = self
                    .builder
                    .build_struct_gep(enum_ty, temp_alloca, 1, "question_payload_ptr")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let payload_raw = self
                    .builder
                    .build_load(self.context.i64_type(), payload_ptr, "question_payload_raw")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let payload_basic = if let Some(target_ty) = payload_target_ty {
                    self.cast_basic_to_type(payload_raw, target_ty)?
                } else {
                    payload_raw
                };
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let success_block = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(failure_bb);
                if is_result {
                    let err_value = self
                        .builder
                        .build_load(enum_ty, temp_alloca, "question_err_value")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.try_build_return(Some(&err_value))?;
                } else if let BasicTypeEnum::StructType(optional_struct_ty) = ret_llvm_ty {
                    let none_struct = optional_struct_ty.const_named_struct(&[
                        self.context.i64_type().const_zero().into(),
                        self.context.i64_type().const_zero().into(),
                    ]);
                    let none_value: BasicValueEnum<'ctx> = none_struct.into();
                    self.try_build_return(Some(&none_value))?;
                } else {
                    return Err(CodegenError::InvalidOperation(
                        "question operator requires optional return type".to_string(),
                    ));
                }

                self.builder.position_at_end(cont_bb);
                let payload_ty = payload_basic.get_type();
                let phi = self
                    .builder
                    .build_phi(payload_ty, "question_result")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let incoming = vec![(payload_basic, success_block)];
                let incoming_refs: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = incoming
                    .iter()
                    .map(|(val, bb)| (val as &dyn BasicValue<'ctx>, *bb))
                    .collect();
                phi.add_incoming(&incoming_refs);
                Ok(phi.as_basic_value())
            }

            Expression::Call {
                function,
                type_args,
                arguments,
            } => {
                // Enum variant constructor call: Identifier("Type_Variant")(payload?) -> enum struct
                if let Expression::Identifier(name) = function.as_ref() {
                    if let Some((tname, vname)) = name.split_once('_') {
                        if let Some(Type::Enum { variants, order }) = self.semantic.types.get(tname)
                        {
                            if variants.contains_key(vname) {
                                if let Some(idx) = order.iter().position(|s| s == vname) {
                                    let tag_val =
                                        self.context.i64_type().const_int(idx as u64, false);
                                    let payload_val = if variants.get(vname).unwrap().is_some() {
                                        if arguments.is_empty() {
                                            return Err(CodegenError::InvalidOperation(
                                                "enum constructor missing payload".to_string(),
                                            ));
                                        }
                                        let payload =
                                            self.generate_expression(&arguments[0].value)?;
                                        eprintln!(
                                            "enum ctor {}::{} payload type {}",
                                            tname,
                                            vname,
                                            payload.get_type().print_to_string().to_string()
                                        );
                                        payload
                                    } else {
                                        self.context.i64_type().const_zero().into()
                                    };
                                    // Use builder.insert_value for non-const aggregates
                                    let enum_ty = self.enum_struct.unwrap();
                                    let undef_struct = enum_ty.get_undef();
                                    let with_tag = self
                                        .builder
                                        .build_insert_value(undef_struct, tag_val, 0, "enum_tag")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let with_payload = self
                                        .builder
                                        .build_insert_value(
                                            with_tag,
                                            payload_val,
                                            1,
                                            "enum_payload",
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    return Ok(with_payload.as_basic_value_enum());
                                }
                            }
                        }
                    }
                    if name == "some" {
                        if arguments.len() != 1 {
                            return Err(CodegenError::InvalidOperation(
                                "some expects exactly one argument".to_string(),
                            ));
                        }
                        let arg_val = self.generate_expression(&arguments[0].value)?;
                        eprintln!(
                            "constructing some with arg type {}",
                            arg_val.get_type().print_to_string().to_string()
                        );
                        let payload = self.cast_to_int(arg_val, self.context.i64_type())?;
                        eprintln!(
                            "converted payload type {}",
                            payload.get_type().print_to_string().to_string()
                        );
                        if let Some(enum_ty) = self.enum_struct {
                            let undef = enum_ty.get_undef();
                            let tagged = self
                                .builder
                                .build_insert_value(
                                    undef,
                                    self.context.i64_type().const_int(1, false),
                                    0,
                                    "some_tag",
                                )
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into_struct_value();
                            let with_payload = self
                                .builder
                                .build_insert_value(tagged, payload, 1, "some_payload")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into_struct_value();
                            eprintln!(
                                "some payload set with struct type {}",
                                with_payload.get_type().print_to_string().to_string()
                            );
                            return Ok(with_payload.as_basic_value_enum());
                        } else {
                            return Ok(self.context.i64_type().const_int(1, false).into());
                        }
                    }
                    if name == "ok" {
                        if arguments.len() != 1 {
                            return Err(CodegenError::InvalidOperation(
                                "ok expects exactly one argument".to_string(),
                            ));
                        }
                        let arg_val = self.generate_expression(&arguments[0].value)?;
                        let payload = self.cast_to_int(arg_val, self.context.i64_type())?;
                        if let Some(enum_ty) = self.enum_struct {
                            let undef = enum_ty.get_undef();
                            let tagged = self
                                .builder
                                .build_insert_value(
                                    undef,
                                    self.context.i64_type().const_zero(),
                                    0,
                                    "ok_tag",
                                )
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into_struct_value();
                            let with_payload = self
                                .builder
                                .build_insert_value(tagged, payload, 1, "ok_payload")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into_struct_value();
                            return Ok(with_payload.as_basic_value_enum());
                        } else {
                            return Ok(self.context.i64_type().const_zero().into());
                        }
                    }
                    if name == "err" {
                        if arguments.len() != 1 {
                            return Err(CodegenError::InvalidOperation(
                                "err expects exactly one argument".to_string(),
                            ));
                        }
                        let arg_val = self.generate_expression(&arguments[0].value)?;
                        let payload = self.cast_to_int(arg_val, self.context.i64_type())?;
                        if let Some(enum_ty) = self.enum_struct {
                            let undef = enum_ty.get_undef();
                            let tagged = self
                                .builder
                                .build_insert_value(
                                    undef,
                                    self.context.i64_type().const_int(1, false),
                                    0,
                                    "err_tag",
                                )
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into_struct_value();
                            let with_payload = self
                                .builder
                                .build_insert_value(tagged, payload, 1, "err_payload")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into_struct_value();
                            return Ok(with_payload.as_basic_value_enum());
                        } else {
                            return Ok(self.context.i64_type().const_int(1, false).into());
                        }
                    }
                }
                // Static path enum variant constructor: StaticPath([Type, Variant])(payload?)
                if let Expression::StaticPath { segments, .. } = function.as_ref() {
                    if segments.len() >= 2 {
                        let type_name = &segments[0];
                        let variant_name = &segments[1];
                        if let Some(Type::Enum { variants, order }) =
                            self.semantic.types.get(type_name)
                        {
                            if variants.contains_key(variant_name) {
                                if let Some(idx) = order.iter().position(|s| s == variant_name) {
                                    let tag_val =
                                        self.context.i64_type().const_int(idx as u64, false);
                                    let payload_val =
                                        if variants.get(variant_name).unwrap().is_some() {
                                            if arguments.is_empty() {
                                                return Err(CodegenError::InvalidOperation(
                                                    "enum constructor missing payload".to_string(),
                                                ));
                                            }
                                            self.generate_expression(&arguments[0].value)?
                                        } else {
                                            self.context.i64_type().const_zero().into()
                                        };
                                    // Use builder.insert_value for non-const aggregates
                                    let enum_ty = self.enum_struct.unwrap();
                                    let undef_struct = enum_ty.get_undef();
                                    let with_tag = self
                                        .builder
                                        .build_insert_value(undef_struct, tag_val, 0, "enum_tag")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let with_payload = self
                                        .builder
                                        .build_insert_value(
                                            with_tag,
                                            payload_val,
                                            1,
                                            "enum_payload",
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    return Ok(with_payload.as_basic_value_enum());
                                }
                            }
                        }
                    }

                    if segments.len() == 2 && !arguments.is_empty() {
                        let trait_name = &segments[0];
                        let method_name = &segments[1];
                        if let Some(first_arg) = arguments.get(0) {
                            if let Expression::Identifier(var_name) = &first_arg.value {
                                if let Some((base_ptr, base_ty)) = self.variables.get(var_name) {
                                    if let BasicTypeEnum::StructType(st) = base_ty {
                                        if let Some((struct_name, (_llvm_ty, _))) = self
                                            .struct_types
                                            .iter()
                                            .find(|(_, (llvm_st, _))| *llvm_st == *st)
                                        {
                                            let mangled_trait = format!(
                                                "{}_{}_{}",
                                                trait_name, struct_name, method_name
                                            );
                                            if let Some(function_val) =
                                                self.functions.get(&mangled_trait).cloned()
                                            {
                                                let param_metas =
                                                    function_val.get_type().get_param_types();
                                                let mut arg_values: Vec<
                                                    BasicMetadataValueEnum<'ctx>,
                                                > = Vec::new();

                                                if let Some(first_meta) = param_metas.get(0) {
                                                    let recv_meta: BasicMetadataValueEnum<'ctx> =
                                                        match first_meta {
                                                            inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => {
                                                                let casted = self
                                                                    .builder
                                                                    .build_pointer_cast(
                                                                        *base_ptr,
                                                                        *pt,
                                                                        "trait_path_self_ptr",
                                                                    )
                                                                    .map_err(|e| {
                                                                        CodegenError::CompilationError(
                                                                            e.to_string(),
                                                                        )
                                                                    })?;
                                                                casted.into()
                                                            }
                                                            inkwell::types::BasicMetadataTypeEnum::StructType(_st_meta) => {
                                                                let bte: BasicTypeEnum<'ctx> = (*st).into();
                                                                let loaded = self
                                                                    .builder
                                                                    .build_load(
                                                                        bte,
                                                                        *base_ptr,
                                                                        "trait_path_self_load",
                                                                    )
                                                                    .map_err(|e| {
                                                                        CodegenError::CompilationError(
                                                                            e.to_string(),
                                                                        )
                                                                    })?;
                                                                loaded.into()
                                                            }
                                                            inkwell::types::BasicMetadataTypeEnum::IntType(it) => {
                                                                let bte: BasicTypeEnum<'ctx> = (*st).into();
                                                                let loaded = self
                                                                    .builder
                                                                    .build_load(
                                                                        bte,
                                                                        *base_ptr,
                                                                        "trait_path_self_load",
                                                                    )
                                                                    .map_err(|e| {
                                                                        CodegenError::CompilationError(
                                                                            e.to_string(),
                                                                        )
                                                                    })?;
                                                                self.cast_to_int(loaded, *it)?.into()
                                                            }
                                                            inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => {
                                                                let bte: BasicTypeEnum<'ctx> = (*st).into();
                                                                let loaded = self
                                                                    .builder
                                                                    .build_load(
                                                                        bte,
                                                                        *base_ptr,
                                                                        "trait_path_self_load",
                                                                    )
                                                                    .map_err(|e| {
                                                                        CodegenError::CompilationError(
                                                                            e.to_string(),
                                                                        )
                                                                    })?;
                                                                self.cast_to_float(loaded, *ft)?.into()
                                                            }
                                                            _ => (*base_ptr).into(),
                                                        };
                                                    arg_values.push(recv_meta);
                                                }

                                                for (arg_index, arg) in
                                                    arguments.iter().enumerate().skip(1)
                                                {
                                                    let value =
                                                        self.generate_expression(&arg.value)?;
                                                    let param_meta_index = arg_index;
                                                    let casted_meta: BasicMetadataValueEnum<'ctx> =
                                                        if let Some(meta_ty) =
                                                            param_metas.get(param_meta_index)
                                                        {
                                                            match meta_ty {
                                                                inkwell::types::BasicMetadataTypeEnum::IntType(it) => {
                                                                    self.cast_to_int(value, *it)?.into()
                                                                }
                                                                inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => {
                                                                    self.cast_to_float(value, *ft)?.into()
                                                                }
                                                                inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => {
                                                                    self.cast_to_ptr(value, *pt)?.into()
                                                                }
                                                                inkwell::types::BasicMetadataTypeEnum::StructType(st_meta) => {
                                                                    if value.get_type() == (*st_meta).into() {
                                                                        value.into()
                                                                    } else {
                                                                        let tmp_alloc = self
                                                                            .create_entry_block_alloca(
                                                                                "trait_path_struct_tmp",
                                                                                (*st_meta).into(),
                                                                            )?;
                                                                        self.builder
                                                                            .build_store(tmp_alloc, value)
                                                                            .map_err(|e| {
                                                                                CodegenError::CompilationError(
                                                                                    e.to_string(),
                                                                                )
                                                                            })?;
                                                                        self.builder
                                                                            .build_load(
                                                                                *st_meta,
                                                                                tmp_alloc,
                                                                                "trait_path_struct_load",
                                                                            )
                                                                            .map_err(|e| {
                                                                                CodegenError::CompilationError(
                                                                                    e.to_string(),
                                                                                )
                                                                            })?
                                                                            .into()
                                                                    }
                                                                }
                                                                _ => self
                                                                    .cast_to_int(value, self.context.i64_type())?
                                                                    .into(),
                                                            }
                                                        } else {
                                                            self.cast_to_int(
                                                                value,
                                                                self.context.i64_type(),
                                                            )?
                                                            .into()
                                                        };
                                                    arg_values.push(casted_meta);
                                                }

                                                for pad_index in arguments.len()..param_metas.len()
                                                {
                                                    let pad: BasicMetadataValueEnum<'ctx> = match param_metas[pad_index] {
                                                        inkwell::types::BasicMetadataTypeEnum::IntType(it) => it.const_zero().into(),
                                                        inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => ft.const_zero().into(),
                                                        inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => pt.const_zero().into(),
                                                        _ => self.context.i64_type().const_zero().into(),
                                                    };
                                                    arg_values.push(pad);
                                                }

                                                let result = self
                                                    .builder
                                                    .build_call(
                                                        function_val,
                                                        &arg_values,
                                                        "trait_path_call",
                                                    )
                                                    .map_err(|e| {
                                                        CodegenError::CompilationError(
                                                            e.to_string(),
                                                        )
                                                    })?;

                                                return if let Some(value) =
                                                    result.try_as_basic_value().left()
                                                {
                                                    Ok(value)
                                                } else {
                                                    Ok(self.context.i64_type().const_zero().into())
                                                };
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Static method call (NOT enum constructor): mangle and call directly
                    // This handles cases like Vec<u64>::new()
                    let mangled_name = segments.join("_");
                    if let Some(callee_fn) = self.functions.get(&mangled_name).cloned() {
                        // Generate arguments
                        let mut arg_values: Vec<BasicMetadataValueEnum> = Vec::new();
                        for arg in arguments {
                            let val = self.generate_expression(&arg.value)?;
                            arg_values.push(val.into());
                        }

                        // Call the function directly
                        let call_result = self
                            .builder
                            .build_call(callee_fn, &arg_values, "static_method_call")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                        return if let Some(ret_val) = call_result.try_as_basic_value().left() {
                            Ok(ret_val)
                        } else {
                            // Void return
                            Ok(self.context.i64_type().const_zero().into())
                        };
                    } else {
                        return Err(CodegenError::UndefinedFunction(format!(
                            "Static method not found: {}",
                            mangled_name
                        )));
                    }
                }
                self.generate_call(function, type_args, arguments)
            }

            Expression::StaticPath { segments, .. } => {
                // Static path like Vec::new or Option::Some
                // Mangle to identifier and look up
                // Check if it's an enum variant (no call, just the tag)
                if segments.len() >= 2 {
                    let type_name = &segments[0];
                    let variant_name = &segments[1];
                    if let Some(Type::Enum { order, .. }) = self.semantic.types.get(type_name) {
                        if let Some(idx) = order.iter().position(|s| s == variant_name) {
                            let tag_val = self.context.i64_type().const_int(idx as u64, false);
                            return Ok(tag_val.into());
                        }
                    }
                }

                // Otherwise treat as function reference (return function pointer)
                // For now, just return zero as placeholder
                Ok(self.context.i64_type().const_zero().into())
            }

            _ => {
                if std::env::var("TRACE_BINARY").is_ok() {
                    eprintln!("generate_binary_op fallback reached");
                }
                // Other expressions not implemented yet
                Ok(self.context.i64_type().const_zero().into())
            }
        }
    }

    fn generate_literal(&self, literal: &Literal) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match literal {
            Literal::Integer(value) => self.const_int_from_literal(value).map(Into::into),

            Literal::Float(value) => Ok(self.context.f64_type().const_float(*value).into()),

            Literal::Boolean(value) => Ok(self
                .context
                .bool_type()
                .const_int(*value as u64, false)
                .into()),

            Literal::Char(value) => Ok(self
                .context
                .i32_type()
                .const_int(*value as u32 as u64, false)
                .into()),

            Literal::String(value) => {
                let string_val = self
                    .builder
                    .build_global_string_ptr(value, "str")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                Ok(string_val.as_pointer_value().into())
            }
        }
    }

    fn generate_short_circuit_binary(
        &mut self,
        operator: &BinaryOperator,
        left_val: BasicValueEnum<'ctx>,
        right: &Expression,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let bool_ty = self.context.bool_type();
        let left_bool = self.ensure_bool_value(left_val)?;
        let current_fn = self.current_function.ok_or_else(|| {
            CodegenError::CompilationError("logical operator requires function context".to_string())
        })?;

        match operator {
            BinaryOperator::And => {
                let rhs_bb = self.context.append_basic_block(current_fn, "and.rhs");
                let short_bb = self.context.append_basic_block(current_fn, "and.short");
                let merge_bb = self.context.append_basic_block(current_fn, "and.cont");

                self.builder
                    .build_conditional_branch(left_bool, rhs_bb, short_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                self.builder.position_at_end(short_bb);
                let false_basic: BasicValueEnum<'ctx> = bool_ty.const_zero().into();
                self.builder
                    .build_unconditional_branch(merge_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let short_block = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(rhs_bb);
                let right_val = self.generate_expression(right)?;
                let right_bool = self.ensure_bool_value(right_val)?;
                let right_basic: BasicValueEnum<'ctx> = right_bool.into();
                self.builder
                    .build_unconditional_branch(merge_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let rhs_block = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(merge_bb);
                let phi = self
                    .builder
                    .build_phi(bool_ty, "and.result")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let incoming = vec![(right_basic, rhs_block), (false_basic, short_block)];
                let incoming_refs: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = incoming
                    .iter()
                    .map(|(val, bb)| (val as &dyn BasicValue<'ctx>, *bb))
                    .collect();
                phi.add_incoming(&incoming_refs);
                Ok(phi.as_basic_value())
            }
            BinaryOperator::Or => {
                let rhs_bb = self.context.append_basic_block(current_fn, "or.rhs");
                let short_bb = self.context.append_basic_block(current_fn, "or.short");
                let merge_bb = self.context.append_basic_block(current_fn, "or.cont");

                self.builder
                    .build_conditional_branch(left_bool, short_bb, rhs_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                self.builder.position_at_end(short_bb);
                let true_basic: BasicValueEnum<'ctx> = bool_ty.const_int(1, false).into();
                self.builder
                    .build_unconditional_branch(merge_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let short_block = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(rhs_bb);
                let right_val = self.generate_expression(right)?;
                let right_bool = self.ensure_bool_value(right_val)?;
                let right_basic: BasicValueEnum<'ctx> = right_bool.into();
                self.builder
                    .build_unconditional_branch(merge_bb)
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let rhs_block = self.builder.get_insert_block().unwrap();

                self.builder.position_at_end(merge_bb);
                let phi = self
                    .builder
                    .build_phi(bool_ty, "or.result")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                let incoming = vec![(true_basic, short_block), (right_basic, rhs_block)];
                let incoming_refs: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = incoming
                    .iter()
                    .map(|(val, bb)| (val as &dyn BasicValue<'ctx>, *bb))
                    .collect();
                phi.add_incoming(&incoming_refs);
                Ok(phi.as_basic_value())
            }
            _ => Err(CodegenError::InvalidOperation(
                "Short-circuit generator only supports logical operators".to_string(),
            )),
        }
    }

    fn generate_binary_op(
        &mut self,
        left: BasicValueEnum<'ctx>,
        operator: &BinaryOperator,
        right: BasicValueEnum<'ctx>,
        operands_unsigned: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match operator {
            BinaryOperator::Add => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self
                        .builder
                        .build_int_add(l, r, "addtmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self
                        .builder
                        .build_float_add(
                            left.into_float_value(),
                            right.into_float_value(),
                            "addtmp",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_pointer_value() && right.is_int_value() {
                    // Pointer arithmetic: ptr + offset
                    let ptr = left.into_pointer_value();
                    let offset = right.into_int_value();
                    let result = unsafe {
                        self.builder.build_in_bounds_gep(
                            self.context.i8_type(), // Use i8* for byte-level pointer arithmetic
                            ptr,
                            &[offset],
                            "ptradd",
                        )
                    }
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_int_value() && right.is_pointer_value() {
                    // Pointer arithmetic: offset + ptr
                    let offset = left.into_int_value();
                    let ptr = right.into_pointer_value();
                    let result = unsafe {
                        self.builder.build_in_bounds_gep(
                            self.context.i8_type(), // Use i8* for byte-level pointer arithmetic
                            ptr,
                            &[offset],
                            "ptradd",
                        )
                    }
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid types for addition".to_string(),
                    ))
                }
            }

            BinaryOperator::Sub => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self
                        .builder
                        .build_int_sub(l, r, "subtmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self
                        .builder
                        .build_float_sub(
                            left.into_float_value(),
                            right.into_float_value(),
                            "subtmp",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid types for subtraction".to_string(),
                    ))
                }
            }

            BinaryOperator::Mul => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self
                        .builder
                        .build_int_mul(l, r, "multmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self
                        .builder
                        .build_float_mul(
                            left.into_float_value(),
                            right.into_float_value(),
                            "multmp",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid types for multiplication".to_string(),
                    ))
                }
            }

            BinaryOperator::Div => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = if operands_unsigned {
                        self.builder
                            .build_int_unsigned_div(l, r, "divtmp")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    } else {
                        self.builder
                            .build_int_signed_div(l, r, "divtmp")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    };
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self
                        .builder
                        .build_float_div(
                            left.into_float_value(),
                            right.into_float_value(),
                            "divtmp",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid types for division".to_string(),
                    ))
                }
            }

            BinaryOperator::Mod => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = if operands_unsigned {
                        self.builder
                            .build_int_unsigned_rem(l, r, "modtmp")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    } else {
                        self.builder
                            .build_int_signed_rem(l, r, "modtmp")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    };
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid types for modulo".to_string(),
                    ))
                }
            }

            BinaryOperator::Equal => {
                if let Some(enum_ty) = self.enum_struct {
                    let coerce = |val: BasicValueEnum<'ctx>, label: &str|
                        -> Result<Option<StructValue<'ctx>>, CodegenError> {
                        if val.is_struct_value() {
                            let sv = val.into_struct_value();
                            if sv.get_type() == enum_ty {
                                return Ok(Some(sv));
                            }
                        } else if val.is_pointer_value() {
                            let pv = val.into_pointer_value();
                            let enum_ptr = self.context.ptr_type(AddressSpace::default());
                            let casted = self
                                .builder
                                .build_pointer_cast(pv, enum_ptr, &format!("{}_enum_cast", label))
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let loaded = self
                                .builder
                                .build_load(enum_ty, casted, &format!("{}_enum_load", label))
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into_struct_value();
                            return Ok(Some(loaded));
                        }
                        Ok(None)
                    };

                    let left_sv = coerce(left, "lhs")?;
                    let right_sv = coerce(right, "rhs")?;
                    if let (Some(left_sv), Some(right_sv)) = (left_sv, right_sv) {
                        let l_tag = self
                            .builder
                            .build_extract_value(left_sv, 0, "enum_ltag")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();
                        let r_tag = self
                            .builder
                            .build_extract_value(right_sv, 0, "enum_rtag")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();
                        let tag_eq = self
                            .builder
                            .build_int_compare(IntPredicate::EQ, l_tag, r_tag, "enum_tag_eq")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                        let l_payload = self
                            .builder
                            .build_extract_value(left_sv, 1, "enum_lpayload")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();
                        let r_payload = self
                            .builder
                            .build_extract_value(right_sv, 1, "enum_rpayload")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();
                        let payload_eq = self
                            .builder
                            .build_int_compare(
                                IntPredicate::EQ,
                                l_payload,
                                r_payload,
                                "enum_payload_eq",
                            )
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                        let both = self
                            .builder
                            .build_and(tag_eq, payload_eq, "enum_eq")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        return Ok(both.into());
                    }
                }
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self
                        .builder
                        .build_int_compare(IntPredicate::EQ, l, r, "eqtmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::OEQ,
                            left.into_float_value(),
                            right.into_float_value(),
                            "eqtmp",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_pointer_value() && right.is_pointer_value() {
                    let l_ptr = left.into_pointer_value();
                    let r_ptr = right.into_pointer_value();
                    let l_int = self
                        .builder
                        .build_ptr_to_int(l_ptr, self.context.i64_type(), "lhs_ptrint")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let r_int = self
                        .builder
                        .build_ptr_to_int(r_ptr, self.context.i64_type(), "rhs_ptrint")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let result = self
                        .builder
                        .build_int_compare(IntPredicate::EQ, l_int, r_int, "ptr_eq")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    let l_ty = left.get_type().print_to_string().to_string();
                    let r_ty = right.get_type().print_to_string().to_string();
                    let ctx = self
                        .current_binary_context
                        .as_deref()
                        .unwrap_or("<unknown expression>");
                    Err(CodegenError::InvalidOperation(format!(
                        "Invalid types for equality comparison: {} vs {} in {}",
                        l_ty, r_ty, ctx
                    )))
                }
            }
            BinaryOperator::NotEqual => {
                if let Some(enum_ty) = self.enum_struct {
                    let coerce = |val: BasicValueEnum<'ctx>, label: &str|
                        -> Result<Option<StructValue<'ctx>>, CodegenError> {
                        if val.is_struct_value() {
                            let sv = val.into_struct_value();
                            if sv.get_type() == enum_ty {
                                return Ok(Some(sv));
                            }
                        } else if val.is_pointer_value() {
                            let pv = val.into_pointer_value();
                            let enum_ptr = self.context.ptr_type(AddressSpace::default());
                            let casted = self
                                .builder
                                .build_pointer_cast(pv, enum_ptr, &format!("{}_enum_cast", label))
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let loaded = self
                                .builder
                                .build_load(enum_ty, casted, &format!("{}_enum_load", label))
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into_struct_value();
                            return Ok(Some(loaded));
                        }
                        Ok(None)
                    };

                    let left_sv = coerce(left, "lhs")?;
                    let right_sv = coerce(right, "rhs")?;
                    if let (Some(left_sv), Some(right_sv)) = (left_sv, right_sv) {
                        let l_tag = self
                            .builder
                            .build_extract_value(left_sv, 0, "enum_ltag")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();
                        let r_tag = self
                            .builder
                            .build_extract_value(right_sv, 0, "enum_rtag")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();
                        let tag_ne = self
                            .builder
                            .build_int_compare(IntPredicate::NE, l_tag, r_tag, "enum_tag_ne")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                        let l_payload = self
                            .builder
                            .build_extract_value(left_sv, 1, "enum_lpayload")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();
                        let r_payload = self
                            .builder
                            .build_extract_value(right_sv, 1, "enum_rpayload")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();
                        let payload_ne = self
                            .builder
                            .build_int_compare(
                                IntPredicate::NE,
                                l_payload,
                                r_payload,
                                "enum_payload_ne",
                            )
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                        let combined = self
                            .builder
                            .build_or(tag_ne, payload_ne, "enum_ne")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        return Ok(combined.into());
                    }
                }
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self
                        .builder
                        .build_int_compare(IntPredicate::NE, l, r, "netmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::ONE,
                            left.into_float_value(),
                            right.into_float_value(),
                            "netmp",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_pointer_value() && right.is_pointer_value() {
                    let l_ptr = left.into_pointer_value();
                    let r_ptr = right.into_pointer_value();
                    let l_int = self
                        .builder
                        .build_ptr_to_int(l_ptr, self.context.i64_type(), "lhs_ptrint")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let r_int = self
                        .builder
                        .build_ptr_to_int(r_ptr, self.context.i64_type(), "rhs_ptrint")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let result = self
                        .builder
                        .build_int_compare(IntPredicate::NE, l_int, r_int, "ptr_ne")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    let l_ty = left.get_type().print_to_string().to_string();
                    let r_ty = right.get_type().print_to_string().to_string();
                    let ctx = self
                        .current_binary_context
                        .as_deref()
                        .unwrap_or("<unknown expression>");
                    Err(CodegenError::InvalidOperation(format!(
                        "Invalid types for inequality comparison: {} vs {} in {}",
                        l_ty, r_ty, ctx
                    )))
                }
            }

            BinaryOperator::Less => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let predicate = if operands_unsigned {
                        IntPredicate::ULT
                    } else {
                        IntPredicate::SLT
                    };
                    let result = self
                        .builder
                        .build_int_compare(predicate, l, r, "lttmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::OLT,
                            left.into_float_value(),
                            right.into_float_value(),
                            "lttmp",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid types for less than comparison".to_string(),
                    ))
                }
            }
            BinaryOperator::Greater => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let predicate = if operands_unsigned {
                        IntPredicate::UGT
                    } else {
                        IntPredicate::SGT
                    };
                    let result = self
                        .builder
                        .build_int_compare(predicate, l, r, "gttmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::OGT,
                            left.into_float_value(),
                            right.into_float_value(),
                            "gttmp",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid types for greater than comparison".to_string(),
                    ))
                }
            }
            BinaryOperator::LessEqual => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let predicate = if operands_unsigned {
                        IntPredicate::ULE
                    } else {
                        IntPredicate::SLE
                    };
                    let result = self
                        .builder
                        .build_int_compare(predicate, l, r, "letmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::OLE,
                            left.into_float_value(),
                            right.into_float_value(),
                            "letmp",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid types for <= comparison".to_string(),
                    ))
                }
            }
            BinaryOperator::GreaterEqual => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let predicate = if operands_unsigned {
                        IntPredicate::UGE
                    } else {
                        IntPredicate::SGE
                    };
                    let result = self
                        .builder
                        .build_int_compare(predicate, l, r, "getmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if left.is_float_value() && right.is_float_value() {
                    let result = self
                        .builder
                        .build_float_compare(
                            FloatPredicate::OGE,
                            left.into_float_value(),
                            right.into_float_value(),
                            "getmp",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid types for >= comparison".to_string(),
                    ))
                }
            }

            BinaryOperator::ShiftLeft => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self
                        .builder
                        .build_left_shift(l, r, "shltmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid types for left shift".to_string(),
                    ))
                }
            }

            BinaryOperator::ShiftRight => {
                if left.is_int_value() && right.is_int_value() {
                    let (l, r, _ty) = self.unify_ints(left, right)?;
                    let result = self
                        .builder
                        .build_right_shift(l, r, true, "shrtmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid types for right shift".to_string(),
                    ))
                }
            }

            BinaryOperator::And => {
                let l_bool = self.ensure_bool_value(left)?;
                let r_bool = self.ensure_bool_value(right)?;
                let result = self
                    .builder
                    .build_and(l_bool, r_bool, "andtmp")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                Ok(result.into())
            }

            BinaryOperator::Or => {
                let l_bool = self.ensure_bool_value(left)?;
                let r_bool = self.ensure_bool_value(right)?;
                let result = self
                    .builder
                    .build_or(l_bool, r_bool, "ortmp")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                Ok(result.into())
            }

            BinaryOperator::Xor => {
                let l_bool = self.ensure_bool_value(left)?;
                let r_bool = self.ensure_bool_value(right)?;
                let result = self
                    .builder
                    .build_xor(l_bool, r_bool, "xortmp")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                Ok(result.into())
            }

            _ => Err(CodegenError::InvalidOperation(format!(
                "Binary operator {:?} not implemented",
                operator
            ))),
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
                    let result = self
                        .builder
                        .build_int_neg(operand.into_int_value(), "negtmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else if operand.is_float_value() {
                    let result = self
                        .builder
                        .build_float_neg(operand.into_float_value(), "negtmp")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid type for negation".to_string(),
                    ))
                }
            }

            UnaryOperator::Not => {
                if operand.is_int_value() {
                    let int_ty = operand.get_type().into_int_type();
                    let zero = int_ty.const_zero();
                    let operand_int = operand.into_int_value();
                    let result = self
                        .builder
                        .build_int_compare(IntPredicate::EQ, operand_int, zero, "not")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid type for logical not".to_string(),
                    ))
                }
            }

            UnaryOperator::BitwiseNot => {
                if operand.is_int_value() {
                    let result = self
                        .builder
                        .build_not(operand.into_int_value(), "bnot")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation(
                        "Invalid type for bitwise not".to_string(),
                    ))
                }
            }

            UnaryOperator::Deref => {
                if operand.is_pointer_value() {
                    // Best-effort: load as i64 by default; many consumers will cast/widen as needed.
                    let i64_ty = self.context.i64_type();
                    let bte: BasicTypeEnum<'ctx> = i64_ty.into();
                    let loaded = self
                        .builder
                        .build_load(bte, operand.into_pointer_value(), "deref")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(loaded)
                } else {
                    // Graceful fallback: return 0 instead of failing
                    Ok(self.context.i64_type().const_zero().into())
                }
            }

            _ => Err(CodegenError::InvalidOperation(format!(
                "Unary operator {:?} not implemented",
                operator
            ))),
        }
    }

    fn cast_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        to_type: &Type,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let target_ty = self.map_ast_type(to_type).unwrap_or(value.get_type());
        if value.get_type() == target_ty {
            Ok(value)
        } else {
            self.cast_basic_to_type(value, target_ty)
        }
    }

    fn generate_call(
        &mut self,
        function: &Expression,
        type_args: &[Type],
        arguments: &[Argument],
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match function {
            Expression::Identifier(func_name) => {
                if func_name == "println" {
                    return self.generate_println_call(arguments);
                }

                if func_name == "len" {
                    if let Some(first) = arguments.get(0) {
                        let is_string_arg = match &first.value {
                            Expression::Literal(Literal::String(_)) => true,
                            Expression::Identifier(name) => self
                                .local_types
                                .get(name)
                                .or_else(|| self.semantic.get_variable_type(name))
                                .map(|t| {
                                    matches!(
                                        t,
                                        Type::Identifier { name: s, type_args: _ }
                                            if s == "string" || s == "String" || s == "str"
                                    )
                                })
                                .unwrap_or(false),
                            _ => false,
                        };

                        if is_string_arg {
                            let strlen_fn =
                                self.functions.get("len").cloned().ok_or_else(|| {
                                    CodegenError::UndefinedFunction("len".to_string())
                                })?;
                            let arg_val = self.generate_expression(&first.value)?;
                            let call = self
                                .builder
                                .build_call(strlen_fn, &[arg_val.into()], "strlen_call")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            if let Some(len_val) = call.try_as_basic_value().left() {
                                return Ok(len_val);
                            }
                            return Ok(self.context.i64_type().const_zero().into());
                        }

                        if let Expression::Matrix { rows } = &first.value {
                            if rows.len() <= 1 {
                                let len = rows.first().map(|r| r.len()).unwrap_or(0) as u64;
                                return Ok(self.context.i64_type().const_int(len, false).into());
                            }
                        }

                        let arg_val = self.generate_expression(&first.value)?;
                        let (vec_struct, _, len_idx, _) = self.vector_field_indices()?;
                        if let BasicValueEnum::StructValue(vec_val) = arg_val {
                            if vec_val.get_type() == vec_struct {
                                let len_val = self
                                    .builder
                                    .build_extract_value(vec_val, len_idx, "vec_len_extract")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                return Ok(len_val);
                            }
                        }
                        // Fallback when not a pointer/string: return zero
                        return Ok(self.context.i64_type().const_zero().into());
                    }

                    return Ok(self.context.i64_type().const_zero().into());
                }

                // Built-in slice helpers (i64/bool variants)
                if matches!(
                    func_name.as_str(),
                    "slice_len" | "slice_is_empty" | "slice_len_bool" | "slice_is_empty_bool"
                ) {
                    let (struct_key, returns_len, returns_bool) = match func_name.as_str() {
                        "slice_len" => ("slice_i64", true, false),
                        "slice_is_empty" => ("slice_i64", false, true),
                        "slice_len_bool" => ("slice_bool", true, false),
                        "slice_is_empty_bool" => ("slice_bool", false, true),
                        _ => unreachable!(),
                    };

                    let default = if returns_bool {
                        self.context.bool_type().const_zero().into()
                    } else {
                        self.context.i64_type().const_zero().into()
                    };

                    if let Some(first) = arguments.get(0) {
                        let (mut use_st, order_vec) =
                            if let Some((st_ref, order)) = self.struct_types.get(struct_key) {
                                (*st_ref, order.clone())
                            } else {
                                return Ok(default);
                            };

                        let sptr_opt: Option<PointerValue<'ctx>> = match &first.value {
                            Expression::Identifier(var) => {
                                if let Some((alloca, bte)) = self.variables.get(var) {
                                    if let BasicTypeEnum::StructType(st_var) = bte {
                                        use_st = *st_var;
                                        Some(*alloca)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };

                        let sptr = if let Some(p) = sptr_opt {
                            p
                        } else {
                            let av = self.generate_expression(&first.value)?;
                            let alloca = self.create_entry_block_alloca(
                                &format!("{}_tmp", struct_key),
                                use_st.into(),
                            )?;
                            self.builder
                                .build_store(alloca, av)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            alloca
                        };

                        let len_idx = order_vec.iter().position(|n| n == "len").unwrap_or(0) as u32;
                        let len_ty = use_st.get_field_type_at_index(len_idx).ok_or_else(|| {
                            CodegenError::InvalidOperation(format!("{}.len type", struct_key))
                        })?;
                        let len_ptr = self
                            .builder
                            .build_struct_gep(use_st, sptr, len_idx, "slen")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let len_val = self
                            .builder
                            .build_load(len_ty, len_ptr, "slenv")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();

                        if returns_len {
                            return Ok(len_val.into());
                        } else {
                            let is_empty = self
                                .builder
                                .build_int_compare(
                                    IntPredicate::EQ,
                                    len_val,
                                    self.context.i64_type().const_zero(),
                                    "isempty",
                                )
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            return Ok(is_empty.into());
                        }
                    }

                    return Ok(default);
                }

                if matches!(func_name.as_str(), "slice_get" | "slice_get_bool") {
                    let enum_ty = self.enum_struct.ok_or_else(|| {
                        CodegenError::InvalidOperation(
                            "Optional enum representation unavailable".to_string(),
                        )
                    })?;
                    let (struct_key, elem_ty) = match func_name.as_str() {
                        "slice_get" => ("slice_i64", self.context.i64_type().into()),
                        "slice_get_bool" => ("slice_bool", self.context.bool_type().into()),
                        _ => unreachable!(),
                    };
                    let none_struct = enum_ty.const_named_struct(&[
                        self.context.i64_type().const_zero().into(),
                        self.context.i64_type().const_zero().into(),
                    ]);
                    let default = none_struct.as_basic_value_enum();

                    if arguments.len() >= 2 {
                        let first = &arguments[0];
                        let second = &arguments[1];

                        let (mut use_st, order_vec) =
                            if let Some((st_ref, order)) = self.struct_types.get(struct_key) {
                                (*st_ref, order.clone())
                            } else {
                                return Ok(default);
                            };

                        let sptr_opt: Option<PointerValue<'ctx>> = match &first.value {
                            Expression::Identifier(var) => {
                                if let Some((alloca, bte)) = self.variables.get(var) {
                                    if let BasicTypeEnum::StructType(st_var) = bte {
                                        use_st = *st_var;
                                        Some(*alloca)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };

                        let sptr = if let Some(p) = sptr_opt {
                            p
                        } else {
                            let av = self.generate_expression(&first.value)?;
                            let alloca = self.create_entry_block_alloca(
                                &format!("{}_tmp", struct_key),
                                use_st.into(),
                            )?;
                            self.builder
                                .build_store(alloca, av)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            alloca
                        };

                        let ptr_idx = order_vec.iter().position(|n| n == "ptr").unwrap_or(0) as u32;
                        let len_idx = order_vec.iter().position(|n| n == "len").unwrap_or(1) as u32;
                        let ptr_ty = use_st.get_field_type_at_index(ptr_idx).ok_or_else(|| {
                            CodegenError::InvalidOperation("slice ptr field type".to_string())
                        })?;
                        let len_ty = use_st.get_field_type_at_index(len_idx).ok_or_else(|| {
                            CodegenError::InvalidOperation("slice len field type".to_string())
                        })?;
                        let ptr_ptr = self
                            .builder
                            .build_struct_gep(use_st, sptr, ptr_idx, "s.ptr")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let len_ptr = self
                            .builder
                            .build_struct_gep(use_st, sptr, len_idx, "s.len")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let base_ptr = self
                            .builder
                            .build_load(ptr_ty, ptr_ptr, "base")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_pointer_value();
                        let lenv = self
                            .builder
                            .build_load(len_ty, len_ptr, "len")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_int_value();

                        let idx_val_raw = self.generate_expression(&second.value)?;
                        let idx_val = match idx_val_raw {
                            BasicValueEnum::IntValue(iv) => iv,
                            _ => self.cast_to_int(idx_val_raw, self.context.i64_type())?,
                        };

                        let cmp = self
                            .builder
                            .build_int_compare(IntPredicate::UGE, idx_val, lenv, "out_of_bounds")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let current_fn = self.current_function.ok_or_else(|| {
                            CodegenError::CompilationError(
                                "No current function for slice_get".to_string(),
                            )
                        })?;

                        let result_alloca =
                            self.create_entry_block_alloca("slice_get_res", enum_ty.into())?;
                        let none_bb = self
                            .context
                            .append_basic_block(current_fn, "slice_get.none");
                        let some_bb = self
                            .context
                            .append_basic_block(current_fn, "slice_get.some");
                        let merge_bb = self
                            .context
                            .append_basic_block(current_fn, "slice_get.merge");

                        self.builder
                            .build_conditional_branch(cmp, none_bb, some_bb)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                        self.builder.position_at_end(none_bb);
                        self.builder
                            .build_store(result_alloca, none_struct)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder
                            .build_unconditional_branch(merge_bb)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                        self.builder.position_at_end(some_bb);
                        let elem_val = match elem_ty {
                            BasicTypeEnum::IntType(it) => {
                                let ptr = unsafe {
                                    self.builder.build_in_bounds_gep(
                                        it,
                                        base_ptr,
                                        &[idx_val],
                                        "elt",
                                    )
                                }
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_load(it, ptr, "load")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            }
                            BasicTypeEnum::PointerType(pt) => {
                                let ptr = unsafe {
                                    self.builder.build_in_bounds_gep(
                                        pt,
                                        base_ptr,
                                        &[idx_val],
                                        "elt",
                                    )
                                }
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_load(pt, ptr, "load")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            }
                            _ => {
                                return Err(CodegenError::InvalidOperation(
                                    "Unsupported slice element type for optional result"
                                        .to_string(),
                                ))
                            }
                        };
                        let payload = self.cast_to_int(elem_val, self.context.i64_type())?;
                        let tagged = self
                            .builder
                            .build_insert_value(
                                enum_ty.get_undef(),
                                self.context.i64_type().const_int(1, false),
                                0,
                                "slice_get_tag",
                            )
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_struct_value();
                        let with_payload = self
                            .builder
                            .build_insert_value(tagged, payload, 1, "slice_get_payload")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                            .into_struct_value();
                        self.builder
                            .build_store(result_alloca, with_payload)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder
                            .build_unconditional_branch(merge_bb)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                        self.builder.position_at_end(merge_bb);
                        let loaded = self
                            .builder
                            .build_load(enum_ty, result_alloca, "slice_get_result")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        return Ok(loaded);
                    }

                    return Ok(default);
                }

                // Built-in sizeof<T>() -> u64
                if func_name == "sizeof" {
                    if type_args.len() != 1 {
                        return Err(CodegenError::InvalidOperation(
                            "sizeof requires exactly one type argument".to_string(),
                        ));
                    }
                    let ty = self.map_ast_type(&type_args[0]).ok_or_else(|| {
                        CodegenError::InvalidOperation("Cannot map type for sizeof".to_string())
                    })?;
                    let size = ty.size_of().ok_or_else(|| {
                        CodegenError::InvalidOperation("Cannot get size of opaque type".to_string())
                    })?;
                    return Ok(size.into());
                }

                // Built-in drop(value) -> none (dispatch to Drop trait if implemented)
                if func_name == "drop" {
                    if arguments.len() != 1 {
                        return Err(CodegenError::InvalidOperation(
                            "drop requires exactly one argument".to_string(),
                        ));
                    }

                    let arg_expr = &arguments[0].value;
                    if let Expression::Identifier(var_name) = arg_expr {
                        self.drop_current_value(var_name)?;
                        self.unregister_owned(var_name);
                        self.local_types.remove(var_name);
                        self.variables.remove(var_name);
                        self.mark_expr_moved(arg_expr);
                        return Ok(self.context.i64_type().const_zero().into());
                    } else {
                        return Err(CodegenError::InvalidOperation(
                            "drop currently only supports identifier arguments".to_string(),
                        ));
                    }
                }

                // Built-in __memmov(dst: *none, src: *none, size: u64) -> none
                if func_name == "__memmov" {
                    if arguments.len() != 3 {
                        return Err(CodegenError::InvalidOperation(
                            "__memmov requires 3 arguments".to_string(),
                        ));
                    }
                    // For now, this is a no-op. In a real implementation, this would use memcpy
                    return Ok(self.context.i64_type().const_zero().into());
                }

                // Built-in __builtin_clzll(n: u64) -> i32
                if func_name == "__builtin_clzll" {
                    if arguments.len() != 1 {
                        return Err(CodegenError::InvalidOperation(
                            "__builtin_clzll requires 1 argument".to_string(),
                        ));
                    }
                    let arg_val = self.generate_expression(&arguments[0].value)?;
                    let int_val = match arg_val {
                        BasicValueEnum::IntValue(iv) => iv,
                        _ => self.cast_to_int(arg_val, self.context.i64_type())?,
                    };
                    // For now, return a placeholder. In a real implementation, this would use LLVM's ctlz
                    let clz = self
                        .builder
                        .build_call(
                            self.module
                                .get_function("__builtin_clzll")
                                .unwrap_or_else(|| {
                                    // Declare it if not found
                                    let fn_type = self
                                        .context
                                        .i32_type()
                                        .fn_type(&[self.context.i64_type().into()], false);
                                    self.module.add_function("__builtin_clzll", fn_type, None)
                                }),
                            &[int_val.into()],
                            "clz",
                        )
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    if let Some(value) = clz.try_as_basic_value().left() {
                        return Ok(value);
                    }
                    return Ok(self.context.i32_type().const_zero().into());
                }

                // Deterministic: exact name, with support for trait static-path calls Trait_method(x, ...)
                let mut resolved: Option<FunctionValue<'ctx>> =
                    self.functions.get(func_name).cloned();
                if resolved.is_none() {
                    if let Some((trait_name, method_name)) = func_name.split_once('_') {
                        if let Some(first) = arguments.get(0) {
                            if let Expression::Identifier(var) = &first.value {
                                if let Some(ty_name) = self.semantic_struct_name_of_var(var) {
                                    let mangled =
                                        format!("{}_{}_{}", trait_name, ty_name, method_name);
                                    resolved = self.functions.get(&mangled).cloned();
                                }
                            }
                        }
                    }
                }
                let function_val = match resolved {
                    Some(f) => f,
                    None => {
                        return Ok(self.context.i64_type().const_zero().into());
                    }
                };

                // Build arguments, casting to declared metadata param types
                let param_metas = function_val.get_type().get_param_types();
                let mut arg_values: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
                for (i, arg) in arguments.iter().enumerate() {
                    // Heuristic: if callee expects a pointer for the first param and the argument is a local variable,
                    // pass its address (pointer) instead of loading the value.
                    if i == 0 {
                        if let Some(inkwell::types::BasicMetadataTypeEnum::PointerType(expect_pt)) =
                            param_metas.get(i)
                        {
                            if let Expression::Identifier(var) = &arg.value {
                                if let Some((ptr, ty)) = self.variables.get(var) {
                                    // If the variable itself holds a pointer (e.g., string), load and pass the pointer value.
                                    if ty.is_pointer_type() {
                                        let loaded = self
                                            .builder
                                            .build_load(*ty, *ptr, "loadptrarg")
                                            .map_err(|e| {
                                                CodegenError::CompilationError(e.to_string())
                                            })?;
                                        let casted = self
                                            .builder
                                            .build_pointer_cast(
                                                loaded.into_pointer_value(),
                                                *expect_pt,
                                                "ptrarg",
                                            )
                                            .map_err(|e| {
                                                CodegenError::CompilationError(e.to_string())
                                            })?;
                                        arg_values.push(casted.into());
                                        continue;
                                    } else {
                                        // Otherwise pass the address-of the alloca as a pointer parameter.
                                        let casted_ptr = self
                                            .builder
                                            .build_pointer_cast(*ptr, *expect_pt, "addrarg")
                                            .map_err(|e| {
                                                CodegenError::CompilationError(e.to_string())
                                            })?;
                                        arg_values.push(casted_ptr.into());
                                        continue;
                                    }
                                }
                            }
                        }
                    }

                    let value = self.generate_expression(&arg.value)?;
                    let casted_meta: BasicMetadataValueEnum<'ctx> = if let Some(meta_ty) =
                        param_metas.get(i)
                    {
                        match meta_ty {
                            inkwell::types::BasicMetadataTypeEnum::IntType(it) => {
                                self.cast_to_int(value, *it)?.into()
                            }
                            inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => {
                                self.cast_to_float(value, *ft)?.into()
                            }
                            inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => {
                                self.cast_to_ptr(value, *pt)?.into()
                            }
                            inkwell::types::BasicMetadataTypeEnum::StructType(st) => {
                                if value.get_type() == (*st).into() {
                                    value.into()
                                } else {
                                    let tmp_alloc =
                                        self.create_entry_block_alloca("struct_tmp", (*st).into())?;
                                    self.builder.build_store(tmp_alloc, value).map_err(|e| {
                                        CodegenError::CompilationError(e.to_string())
                                    })?;
                                    self.builder
                                        .build_load(*st, tmp_alloc, "struct_arg")
                                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                        .into()
                                }
                            }
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
                        inkwell::types::BasicMetadataTypeEnum::IntType(it) => {
                            it.const_zero().into()
                        }
                        inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => {
                            ft.const_zero().into()
                        }
                        inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => {
                            pt.const_zero().into()
                        }
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
                    eprintln!(
                        "call {} produced no direct value; fn type {}",
                        func_name,
                        function_val.get_type().print_to_string().to_string()
                    );
                    // Void function
                    Ok(self.context.i64_type().const_zero().into())
                }
            }
            Expression::FieldAccess { object, field } => {
                let mut cached_object_value: Option<BasicValueEnum<'ctx>> = None;
                if !matches!(object.as_ref(), Expression::Identifier(_)) {
                    cached_object_value = Some(self.generate_expression(object)?);
                }

                // Method call lowering: expr.method(args) => Type_method(self, args)
                // Only support when expr is an identifier bound to a known struct
                if let Expression::Identifier(var_name) = object.as_ref() {
                    eprintln!("method call {}.{}", var_name, field);
                    if let Some(vector_result) =
                        self.try_generate_vector_method_call(var_name, field, arguments)?
                    {
                        return Ok(vector_result);
                    }
                    // no-op here; real reduce implementation below handles both semantic and tracked lengths
                    // Special-case: 1D vector methods like reduce over arrays
                    if let Some(Type::Matrix {
                        element_type: _,
                        dimensions,
                    }) = self.semantic.get_variable_type(var_name)
                    {
                        if field == "reduce" {
                            // Expect a single closure argument: (acc, x) => expr
                            // Compute length from semantic dims (product)
                            let len = if dimensions.is_empty() {
                                0
                            } else {
                                dimensions.iter().product::<usize>()
                            } as u64;
                            if len == 0 {
                                return Ok(self.context.i64_type().const_zero().into());
                            }

                            // Evaluate base pointer to the first element
                            let base_val = self.generate_expression(object)?;
                            if !base_val.is_pointer_value() {
                                return Ok(self.context.i64_type().const_zero().into());
                            }
                            let base_ptr = base_val.into_pointer_value();

                            // Validate argument shape
                            if let Some(first_arg) = arguments.get(0) {
                                if let Expression::Function {
                                    parameters, body, ..
                                } = &first_arg.value
                                {
                                    // Expect 2 params (acc, x)
                                    let (p_acc_name, p_x_name) = if parameters.len() >= 2 {
                                        (parameters[0].name.clone(), parameters[1].name.clone())
                                    } else {
                                        ("acc".to_string(), "x".to_string())
                                    };
                                    // Create accumulator alloca initialized to 0
                                    let i64_bte: BasicTypeEnum<'ctx> =
                                        self.context.i64_type().into();
                                    let acc_alloca =
                                        self.create_entry_block_alloca(&p_acc_name, i64_bte)?;
                                    self.builder
                                        .build_store(
                                            acc_alloca,
                                            self.context.i64_type().const_zero(),
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    // Bind acc into variables table (save previous)
                                    let prev_acc = self
                                        .variables
                                        .insert(p_acc_name.to_string(), (acc_alloca, i64_bte));

                                    // Also prepare a temporary alloca for the element (x)
                                    let x_alloca =
                                        self.create_entry_block_alloca(&p_x_name, i64_bte)?;
                                    let prev_x = self
                                        .variables
                                        .insert(p_x_name.to_string(), (x_alloca, i64_bte));

                                    // Build loop blocks
                                    let current_fn = self.current_function.ok_or_else(|| {
                                        CodegenError::CompilationError(
                                            "No current function".to_string(),
                                        )
                                    })?;
                                    let idx_alloca =
                                        self.create_entry_block_alloca("idx", i64_bte)?;
                                    self.builder
                                        .build_store(
                                            idx_alloca,
                                            self.context.i64_type().const_zero(),
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let cond_bb =
                                        self.context.append_basic_block(current_fn, "reduce.cond");
                                    let body_bb =
                                        self.context.append_basic_block(current_fn, "reduce.body");
                                    let inc_bb =
                                        self.context.append_basic_block(current_fn, "reduce.inc");
                                    let end_bb =
                                        self.context.append_basic_block(current_fn, "reduce.end");

                                    self.builder.build_unconditional_branch(cond_bb).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    // cond
                                    self.builder.position_at_end(cond_bb);
                                    let idx_cur = self
                                        .builder
                                        .build_load(i64_bte, idx_alloca, "idx")
                                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                        .into_int_value();
                                    let endc = self.context.i64_type().const_int(len, false);
                                    let cmp = self
                                        .builder
                                        .build_int_compare(IntPredicate::SLT, idx_cur, endc, "rcmp")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    self.builder
                                        .build_conditional_branch(cmp, body_bb, end_bb)
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;

                                    // body: load element into x, evaluate closure body, store back to acc
                                    self.builder.position_at_end(body_bb);
                                    let elem_ptr = unsafe {
                                        self.builder.build_in_bounds_gep(
                                            self.context.i64_type(),
                                            base_ptr,
                                            &[idx_cur],
                                            "ridx",
                                        )
                                    }
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let elem_val = self
                                        .builder
                                        .build_load(self.context.i64_type(), elem_ptr, "elem")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    self.builder.build_store(x_alloca, elem_val).map_err(|e| {
                                        CodegenError::CompilationError(e.to_string())
                                    })?;

                                    // Evaluate closure body with current bindings
                                    let step_val: BasicValueEnum<'ctx> = match body {
                                        FunctionBody::Expression(expr) => {
                                            let v = self.generate_expression(expr)?;
                                            self.cast_to_int(v, self.context.i64_type())?.into()
                                        }
                                        FunctionBody::Block(stmts) => {
                                            // Execute statements; if last is an expression, take its value
                                            let mut last_expr_value: Option<BasicValueEnum<'ctx>> =
                                                None;
                                            let slice: &[Statement] = &stmts[..];
                                            if let Some((last, prefix)) = slice.split_last() {
                                                for s in prefix {
                                                    let _ = self.generate_statement(s);
                                                }
                                                if let Statement::Expression(expr) = last {
                                                    let v = self.generate_expression(expr)?;
                                                    last_expr_value = Some(v);
                                                }
                                            }
                                            if let Some(v) = last_expr_value {
                                                self.cast_to_int(v, self.context.i64_type())?.into()
                                            } else {
                                                self.context.i64_type().const_zero().into()
                                            }
                                        }
                                    };
                                    // Store new acc value
                                    self.builder.build_store(acc_alloca, step_val).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    if let Some(current_block) = self.builder.get_insert_block() {
                                        if current_block.get_terminator().is_none() {
                                            self.builder
                                                .build_unconditional_branch(inc_bb)
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
                                        }
                                    }

                                    // inc
                                    self.builder.position_at_end(inc_bb);
                                    let idx_cur2 = self
                                        .builder
                                        .build_load(i64_bte, idx_alloca, "idx")
                                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                        .into_int_value();
                                    let next = self
                                        .builder
                                        .build_int_add(
                                            idx_cur2,
                                            self.context.i64_type().const_int(1, false),
                                            "inc",
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    self.builder.build_store(idx_alloca, next).map_err(|e| {
                                        CodegenError::CompilationError(e.to_string())
                                    })?;
                                    self.builder.build_unconditional_branch(cond_bb).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;

                                    // end
                                    self.builder.position_at_end(end_bb);
                                    // Restore previous bindings
                                    if let Some(prev) = prev_x {
                                        self.variables.insert(p_x_name.to_string(), prev);
                                    } else {
                                        self.variables.remove(&p_x_name);
                                    }
                                    if let Some(prev) = prev_acc {
                                        self.variables.insert(p_acc_name.to_string(), prev);
                                    } else {
                                        self.variables.remove(&p_acc_name);
                                    }
                                    // Load final accumulator and return it
                                    let final_acc = self
                                        .builder
                                        .build_load(i64_bte, acc_alloca, "acc")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    return Ok(final_acc);
                                }
                            }
                            // If not a closure arg, return 0 for now
                            return Ok(self.context.i64_type().const_zero().into());
                        }
                    }
                    // Fallback: if we don't have semantic type, but we tracked a vector length, run the same reduce lowering
                    if field == "reduce" {
                        if let Some(len) = self.vector_lengths.get(var_name).cloned() {
                            if len > 0 {
                                let base_val = self.generate_expression(object)?;
                                if base_val.is_pointer_value() {
                                    let base_ptr = base_val.into_pointer_value();
                                    if let Some(first_arg) = arguments.get(0) {
                                        if let Expression::Function {
                                            parameters, body, ..
                                        } = &first_arg.value
                                        {
                                            let (p_acc_name, p_x_name) = if parameters.len() >= 2 {
                                                (
                                                    parameters[0].name.clone(),
                                                    parameters[1].name.clone(),
                                                )
                                            } else {
                                                ("acc".to_string(), "x".to_string())
                                            };
                                            let i64_bte: BasicTypeEnum<'ctx> =
                                                self.context.i64_type().into();
                                            let acc_alloca = self
                                                .create_entry_block_alloca(&p_acc_name, i64_bte)?;
                                            self.builder
                                                .build_store(
                                                    acc_alloca,
                                                    self.context.i64_type().const_zero(),
                                                )
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
                                            let prev_acc = self.variables.insert(
                                                p_acc_name.to_string(),
                                                (acc_alloca, i64_bte),
                                            );
                                            let x_alloca =
                                                self.create_entry_block_alloca(&p_x_name, i64_bte)?;
                                            let prev_x = self
                                                .variables
                                                .insert(p_x_name.to_string(), (x_alloca, i64_bte));

                                            let current_fn =
                                                self.current_function.ok_or_else(|| {
                                                    CodegenError::CompilationError(
                                                        "No current function".to_string(),
                                                    )
                                                })?;
                                            let idx_alloca =
                                                self.create_entry_block_alloca("idx", i64_bte)?;
                                            self.builder
                                                .build_store(
                                                    idx_alloca,
                                                    self.context.i64_type().const_zero(),
                                                )
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
                                            let cond_bb = self
                                                .context
                                                .append_basic_block(current_fn, "reduce.cond");
                                            let body_bb = self
                                                .context
                                                .append_basic_block(current_fn, "reduce.body");
                                            let inc_bb = self
                                                .context
                                                .append_basic_block(current_fn, "reduce.inc");
                                            let end_bb = self
                                                .context
                                                .append_basic_block(current_fn, "reduce.end");

                                            self.builder
                                                .build_unconditional_branch(cond_bb)
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
                                            self.builder.position_at_end(cond_bb);
                                            let idx_cur = self
                                                .builder
                                                .build_load(i64_bte, idx_alloca, "idx")
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?
                                                .into_int_value();
                                            let endc =
                                                self.context.i64_type().const_int(len, false);
                                            let cmp = self
                                                .builder
                                                .build_int_compare(
                                                    IntPredicate::SLT,
                                                    idx_cur,
                                                    endc,
                                                    "rcmp",
                                                )
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
                                            self.builder
                                                .build_conditional_branch(cmp, body_bb, end_bb)
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;

                                            self.builder.position_at_end(body_bb);
                                            let elem_ptr = unsafe {
                                                self.builder.build_in_bounds_gep(
                                                    self.context.i64_type(),
                                                    base_ptr,
                                                    &[idx_cur],
                                                    "ridx",
                                                )
                                            }
                                            .map_err(|e| {
                                                CodegenError::CompilationError(e.to_string())
                                            })?;
                                            let elem_val = self
                                                .builder
                                                .build_load(
                                                    self.context.i64_type(),
                                                    elem_ptr,
                                                    "elem",
                                                )
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
                                            self.builder.build_store(x_alloca, elem_val).map_err(
                                                |e| CodegenError::CompilationError(e.to_string()),
                                            )?;

                                            let step_val: BasicValueEnum<'ctx> = match body {
                                                FunctionBody::Expression(expr) => {
                                                    let v = self.generate_expression(expr)?;
                                                    self.cast_to_int(v, self.context.i64_type())?
                                                        .into()
                                                }
                                                FunctionBody::Block(stmts) => {
                                                    let mut last_expr_value: Option<
                                                        BasicValueEnum<'ctx>,
                                                    > = None;
                                                    let slice: &[Statement] = &stmts[..];
                                                    if let Some((last, prefix)) = slice.split_last()
                                                    {
                                                        for s in prefix {
                                                            let _ = self.generate_statement(s);
                                                        }
                                                        if let Statement::Expression(expr) = last {
                                                            let v =
                                                                self.generate_expression(expr)?;
                                                            last_expr_value = Some(v);
                                                        }
                                                    }
                                                    if let Some(v) = last_expr_value {
                                                        self.cast_to_int(
                                                            v,
                                                            self.context.i64_type(),
                                                        )?
                                                        .into()
                                                    } else {
                                                        self.context.i64_type().const_zero().into()
                                                    }
                                                }
                                            };
                                            self.builder
                                                .build_store(acc_alloca, step_val)
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
                                            if let Some(current_block) =
                                                self.builder.get_insert_block()
                                            {
                                                if current_block.get_terminator().is_none() {
                                                    self.builder
                                                        .build_unconditional_branch(inc_bb)
                                                        .map_err(|e| {
                                                            CodegenError::CompilationError(
                                                                e.to_string(),
                                                            )
                                                        })?;
                                                }
                                            }

                                            self.builder.position_at_end(inc_bb);
                                            let idx_cur2 = self
                                                .builder
                                                .build_load(i64_bte, idx_alloca, "idx")
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?
                                                .into_int_value();
                                            let next = self
                                                .builder
                                                .build_int_add(
                                                    idx_cur2,
                                                    self.context.i64_type().const_int(1, false),
                                                    "inc",
                                                )
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
                                            self.builder.build_store(idx_alloca, next).map_err(
                                                |e| CodegenError::CompilationError(e.to_string()),
                                            )?;
                                            self.builder
                                                .build_unconditional_branch(cond_bb)
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;

                                            self.builder.position_at_end(end_bb);
                                            if let Some(prev) = prev_x {
                                                self.variables.insert(p_x_name.to_string(), prev);
                                            } else {
                                                self.variables.remove(&p_x_name);
                                            }
                                            if let Some(prev) = prev_acc {
                                                self.variables.insert(p_acc_name.to_string(), prev);
                                            } else {
                                                self.variables.remove(&p_acc_name);
                                            }
                                            let final_acc = self
                                                .builder
                                                .build_load(i64_bte, acc_alloca, "acc")
                                                .map_err(|e| {
                                                    CodegenError::CompilationError(e.to_string())
                                                })?;
                                            return Ok(final_acc);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Some((base_ptr, base_ty)) = self.variables.get(var_name) {
                        eprintln!(
                            "have variable {} with type {}",
                            var_name,
                            base_ty.print_to_string().to_string()
                        );
                        if let BasicTypeEnum::StructType(st) = base_ty {
                            if let Some((struct_name, (_llvm_st, _))) = self
                                .struct_types
                                .iter()
                                .find(|(_, (llvm_st, _))| llvm_st == st)
                            {
                                eprintln!("struct name {}", struct_name);
                                // Try inherent impl first
                                let mangled_inherent = format!("{}_{}", struct_name, field);
                                let mut selected_fn: Option<FunctionValue<'ctx>> =
                                    self.functions.get(&mangled_inherent).cloned();

                                // If not found, try trait impls for this struct type deterministically
                                if selected_fn.is_none() {
                                    eprintln!("searching traits for {}", struct_name);
                                    // Collect candidate trait mangled names that have this method for this type
                                    let mut candidates: Vec<String> = Vec::new();
                                    for (trait_name, impls_for_trait) in &self.semantic.trait_impls
                                    {
                                        if let Some(info) = impls_for_trait.get(struct_name) {
                                            if info.methods.contains_key(field) {
                                                candidates.push(format!(
                                                    "{}_{}_{}",
                                                    trait_name, struct_name, field
                                                ));
                                            }
                                        }
                                    }
                                    eprintln!("candidates: {:?}", candidates);
                                    // Choose lexicographically smallest for determinism if multiple traits match
                                    if let Some(best) = candidates.into_iter().min() {
                                        eprintln!("best candidate {}", best);
                                        if let Some(fv) = self.functions.get(&best).cloned() {
                                            selected_fn = Some(fv);
                                        }
                                    }
                                }

                                if let Some(function_val) = selected_fn {
                                    eprintln!(
                                        "selected function {}",
                                        function_val.get_name().to_string_lossy()
                                    );
                                    if let Some(sig) =
                                        self.semantic.functions.get(&mangled_inherent)
                                    {
                                        eprintln!(
                                            "semantic function {} return {:?}",
                                            mangled_inherent, sig.return_type
                                        );
                                    }
                                    if let Some(impl_info) =
                                        self.semantic.inherent_impls.get(struct_name)
                                    {
                                        if let Some(sig) = impl_info.methods.get(field) {
                                            eprintln!(
                                                "semantic method {}.{} return {:?}",
                                                struct_name, field, sig.return_type
                                            );
                                        }
                                    }
                                    // Prepare args: first receiver, then others cast to param types
                                    let param_metas = function_val.get_type().get_param_types();
                                    let mut arg_values: Vec<BasicMetadataValueEnum<'ctx>> =
                                        Vec::new();
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
                                        let casted_meta: BasicMetadataValueEnum<'ctx> =
                                            if let Some(meta_ty) = param_metas.get(i + 1) {
                                                match meta_ty {
                                                inkwell::types::BasicMetadataTypeEnum::IntType(it) => self.cast_to_int(value, *it)?.into(),
                                                inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => self.cast_to_float(value, *ft)?.into(),
                                                inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => self.cast_to_ptr(value, *pt)?.into(),
                                                _ => self.cast_to_int(value, self.context.i64_type())?.into(),
                                            }
                                            } else {
                                                self.cast_to_int(value, self.context.i64_type())?
                                                    .into()
                                            };
                                        arg_values.push(casted_meta);
                                    }
                                    // Pad if necessary
                                    for i in (arguments.len() + 1)..param_metas.len() {
                                        let pad: BasicMetadataValueEnum<'ctx> = match param_metas[i]
                                        {
                                            inkwell::types::BasicMetadataTypeEnum::IntType(it) => {
                                                it.const_zero().into()
                                            }
                                            inkwell::types::BasicMetadataTypeEnum::FloatType(
                                                ft,
                                            ) => ft.const_zero().into(),
                                            inkwell::types::BasicMetadataTypeEnum::PointerType(
                                                pt,
                                            ) => pt.const_zero().into(),
                                            _ => self.context.i64_type().const_zero().into(),
                                        };
                                        arg_values.push(pad);
                                    }

                                    let result = self
                                        .builder
                                        .build_call(function_val, &arg_values, "calltmp")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    if let Some(value) = result.try_as_basic_value().left() {
                                        return Ok(value);
                                    } else {
                                        return Ok(self.context.i64_type().const_zero().into());
                                    }
                                }
                                eprintln!("no function found for {}.{}", struct_name, field);
                                let mut keys: Vec<String> =
                                    self.functions.keys().cloned().collect();
                                keys.sort();
                                eprintln!("available functions: {:?}", keys);
                            }
                        }
                    }
                }
                if field == "to_string" && arguments.is_empty() {
                    let value = match cached_object_value {
                        Some(v) => v,
                        None => self.generate_expression(object)?,
                    };
                    return Ok(value);
                }
                // Fallback when we can't lower method
                if cached_object_value.is_some() {
                    // Already evaluated for side effects
                } else if !matches!(object.as_ref(), Expression::Identifier(_)) {
                    let _ = self.generate_expression(object)?;
                }
                Ok(self.context.i64_type().const_zero().into())
            }
            _ => Err(CodegenError::InvalidOperation(
                "Function calls on expressions not supported yet".to_string(),
            )),
        }
    }

    fn generate_println_call(
        &mut self,
        arguments: &[Argument],
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if arguments.is_empty() {
            // Just print a newline
            let newline_str = self
                .builder
                .build_global_string_ptr("\n", "newline")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

            let printf_fn = *self.functions.get("printf").unwrap();
            let args = vec![newline_str.as_pointer_value().into()];
            self.builder
                .build_call(printf_fn, &args, "printf_call")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        } else {
            // For each argument, print it. Special-case: arr.filter(closure) -> print elements like [..]
            let printf_fn = *self.functions.get("printf").unwrap();
            for (i, arg) in arguments.iter().enumerate() {
                // Try special-case pattern match on arg expression
                let mut handled_special = match &arg.value {
                    Expression::Call {
                        function,
                        type_args: _,
                        arguments: hof_args,
                    } => {
                        if let Expression::FieldAccess { object, field } = function.as_ref() {
                            if field == "map" {
                                // Determine length of vector from semantics or tracked lengths
                                let mut len_opt: Option<u64> = None;
                                if let Expression::Identifier(var_name) = object.as_ref() {
                                    if let Some(Type::Matrix {
                                        element_type: _,
                                        dimensions,
                                    }) = self.semantic.get_variable_type(var_name)
                                    {
                                        let l = if dimensions.is_empty() {
                                            0
                                        } else {
                                            dimensions.iter().product::<usize>()
                                        } as u64;
                                        if l > 0 {
                                            len_opt = Some(l);
                                        }
                                    } else if let Some(l) =
                                        self.vector_lengths.get(var_name).cloned()
                                    {
                                        len_opt = Some(l);
                                    }
                                } else if let Expression::Matrix { rows } = object.as_ref() {
                                    let l = if rows.len() <= 1 {
                                        rows.first().map(|r| r.len()).unwrap_or(0)
                                    } else {
                                        rows.len()
                                    } as u64;
                                    if l > 0 {
                                        len_opt = Some(l);
                                    }
                                }
                                let base_ptr_opt: Option<PointerValue<'ctx>> = {
                                    let v = self.generate_expression(object).ok();
                                    v.and_then(|bv| {
                                        if bv.is_pointer_value() {
                                            Some(bv.into_pointer_value())
                                        } else {
                                            None
                                        }
                                    })
                                };
                                if let (Some(len), Some(base_ptr)) = (len_opt, base_ptr_opt) {
                                    // Print opening bracket
                                    let open =
                                        self.builder.build_global_string_ptr("[", "obrm").map_err(
                                            |e| CodegenError::CompilationError(e.to_string()),
                                        )?;
                                    let args_open = vec![open.as_pointer_value().into()];
                                    self.builder
                                        .build_call(printf_fn, &args_open, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;

                                    // Prepare idx alloca
                                    let i64_bte: BasicTypeEnum<'ctx> =
                                        self.context.i64_type().into();
                                    let idx_alloca =
                                        self.create_entry_block_alloca("idx", i64_bte)?;
                                    self.builder
                                        .build_store(
                                            idx_alloca,
                                            self.context.i64_type().const_zero(),
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let current_fn = self.current_function.ok_or_else(|| {
                                        CodegenError::CompilationError(
                                            "No current function".to_string(),
                                        )
                                    })?;
                                    let cond_bb = self
                                        .context
                                        .append_basic_block(current_fn, "printmap.cond");
                                    let body_bb = self
                                        .context
                                        .append_basic_block(current_fn, "printmap.body");
                                    let inc_bb =
                                        self.context.append_basic_block(current_fn, "printmap.inc");
                                    let end_bb =
                                        self.context.append_basic_block(current_fn, "printmap.end");

                                    self.builder.build_unconditional_branch(cond_bb).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    // cond
                                    self.builder.position_at_end(cond_bb);
                                    let idx_cur = self
                                        .builder
                                        .build_load(i64_bte, idx_alloca, "idx")
                                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                        .into_int_value();
                                    let endc = self.context.i64_type().const_int(len, false);
                                    let cmp = self
                                        .builder
                                        .build_int_compare(IntPredicate::SLT, idx_cur, endc, "cmp")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    self.builder
                                        .build_conditional_branch(cmp, body_bb, end_bb)
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    // body
                                    self.builder.position_at_end(body_bb);
                                    let elem_ptr = unsafe {
                                        self.builder.build_in_bounds_gep(
                                            self.context.i64_type(),
                                            base_ptr,
                                            &[idx_cur],
                                            "idx",
                                        )
                                    }
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let elem_val = self
                                        .builder
                                        .build_load(self.context.i64_type(), elem_ptr, "elem")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    // Evaluate closure body on x
                                    let out_val: BasicValueEnum<'ctx> = if let Some(farg) =
                                        hof_args.get(0)
                                    {
                                        if let Expression::Function {
                                            parameters, body, ..
                                        } = &farg.value
                                        {
                                            let p_name = parameters
                                                .get(0)
                                                .map(|p| p.name.clone())
                                                .unwrap_or("x".to_string());
                                            let x_alloca =
                                                self.create_entry_block_alloca(&p_name, i64_bte)?;
                                            let prev_x = self
                                                .variables
                                                .insert(p_name.clone(), (x_alloca, i64_bte));
                                            self.builder.build_store(x_alloca, elem_val).map_err(
                                                |e| CodegenError::CompilationError(e.to_string()),
                                            )?;
                                            let v: BasicValueEnum<'ctx> = match body {
                                                FunctionBody::Expression(expr) => {
                                                    let bv = self.generate_expression(expr)?;
                                                    self.cast_to_int(bv, self.context.i64_type())?
                                                        .into()
                                                }
                                                FunctionBody::Block(stmts) => {
                                                    let mut last_expr_value: Option<
                                                        BasicValueEnum<'ctx>,
                                                    > = None;
                                                    let slice: &[Statement] = &stmts[..];
                                                    if let Some((last, prefix)) = slice.split_last()
                                                    {
                                                        for s in prefix {
                                                            let _ = self.generate_statement(s);
                                                        }
                                                        if let Statement::Expression(expr) = last {
                                                            let bv =
                                                                self.generate_expression(expr)?;
                                                            last_expr_value = Some(bv);
                                                        }
                                                    }
                                                    if let Some(bv) = last_expr_value {
                                                        self.cast_to_int(
                                                            bv,
                                                            self.context.i64_type(),
                                                        )?
                                                        .into()
                                                    } else {
                                                        self.context.i64_type().const_zero().into()
                                                    }
                                                }
                                            };
                                            if let Some(prev) = prev_x {
                                                self.variables.insert(p_name, prev);
                                            } else {
                                                self.variables.remove("x");
                                            }
                                            v
                                        } else if let Expression::Identifier(fname) = &farg.value {
                                            // If a named function exists (e.g., from bind lowering), call it with the element
                                            if let Some(fun) = self.functions.get(fname).cloned() {
                                                let fparam = fun.get_type().get_param_types();
                                                // Only support unary functions here
                                                if fparam.len() == 1 {
                                                    let arg_meta: BasicMetadataValueEnum = match fparam[0] {
                                                        inkwell::types::BasicMetadataTypeEnum::IntType(it) => self.cast_to_int(elem_val, it)?.into(),
                                                        inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => self.cast_to_float(elem_val, ft)?.into(),
                                                        inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => self.cast_to_ptr(elem_val, pt)?.into(),
                                                        _ => self.cast_to_int(elem_val, self.context.i64_type())?.into(),
                                                    };
                                                    let cres = self
                                                        .builder
                                                        .build_call(fun, &[arg_meta], "callmap")
                                                        .map_err(|e| {
                                                            CodegenError::CompilationError(
                                                                e.to_string(),
                                                            )
                                                        })?;
                                                    if let Some(bv) =
                                                        cres.try_as_basic_value().left()
                                                    {
                                                        self.cast_to_int(
                                                            bv,
                                                            self.context.i64_type(),
                                                        )?
                                                        .into()
                                                    } else {
                                                        self.context.i64_type().const_zero().into()
                                                    }
                                                } else {
                                                    self.context.i64_type().const_zero().into()
                                                }
                                            } else {
                                                self.context.i64_type().const_zero().into()
                                            }
                                        } else {
                                            self.context.i64_type().const_zero().into()
                                        }
                                    } else {
                                        self.context.i64_type().const_zero().into()
                                    };
                                    // Print space if idx != 0
                                    let is_zero = self
                                        .builder
                                        .build_int_compare(
                                            IntPredicate::EQ,
                                            idx_cur,
                                            self.context.i64_type().const_zero(),
                                            "iszero",
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let sp_then =
                                        self.context.append_basic_block(current_fn, "pm.sp.then");
                                    let sp_cont =
                                        self.context.append_basic_block(current_fn, "pm.sp.cont");
                                    self.builder
                                        .build_conditional_branch(is_zero, sp_cont, sp_then)
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    self.builder.position_at_end(sp_then);
                                    let fmt_space = self
                                        .builder
                                        .build_global_string_ptr("%s", "fmtsm")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let space =
                                        self.builder.build_global_string_ptr(" ", "spm").map_err(
                                            |e| CodegenError::CompilationError(e.to_string()),
                                        )?;
                                    let args_sp: Vec<BasicValueEnum<'ctx>> = vec![
                                        fmt_space.as_pointer_value().into(),
                                        space.as_pointer_value().into(),
                                    ];
                                    let args_spm: Vec<_> =
                                        args_sp.into_iter().map(|v| v.into()).collect();
                                    self.builder
                                        .build_call(printf_fn, &args_spm, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    self.builder.build_unconditional_branch(sp_cont).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    self.builder.position_at_end(sp_cont);
                                    // Print number
                                    let fmt_num = self
                                        .builder
                                        .build_global_string_ptr("%lld", "fmnm")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let args_num: Vec<BasicValueEnum<'ctx>> =
                                        vec![fmt_num.as_pointer_value().into(), out_val];
                                    let args_numm: Vec<_> =
                                        args_num.into_iter().map(|v| v.into()).collect();
                                    self.builder
                                        .build_call(printf_fn, &args_numm, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    // jump to inc
                                    self.builder.build_unconditional_branch(inc_bb).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    // inc
                                    self.builder.position_at_end(inc_bb);
                                    let idx_cur2 = self
                                        .builder
                                        .build_load(i64_bte, idx_alloca, "idx")
                                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                        .into_int_value();
                                    let next = self
                                        .builder
                                        .build_int_add(
                                            idx_cur2,
                                            self.context.i64_type().const_int(1, false),
                                            "inc",
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    self.builder.build_store(idx_alloca, next).map_err(|e| {
                                        CodegenError::CompilationError(e.to_string())
                                    })?;
                                    self.builder.build_unconditional_branch(cond_bb).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    // end
                                    self.builder.position_at_end(end_bb);
                                    let close =
                                        self.builder.build_global_string_ptr("]", "cbrm").map_err(
                                            |e| CodegenError::CompilationError(e.to_string()),
                                        )?;
                                    let args_close = vec![close.as_pointer_value().into()];
                                    self.builder
                                        .build_call(printf_fn, &args_close, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let after_bb = self
                                        .context
                                        .append_basic_block(current_fn, "printmap.after");
                                    self.builder.build_unconditional_branch(after_bb).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    self.builder.position_at_end(after_bb);
                                    true
                                } else {
                                    false
                                }
                            } else if field == "filter" {
                                // Temporary: print placeholder for filter until CFG is fully stabilized
                                let open = self
                                    .builder
                                    .build_global_string_ptr("[filter]", "fltph")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let args_open = vec![open.as_pointer_value().into()];
                                self.builder
                                    .build_call(printf_fn, &args_open, "printf_call")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                    _ => false,
                };

                // 2D matrix literal pretty-print: [a b; c d; ...]
                if !handled_special {
                    if let Expression::Matrix { rows } = &arg.value {
                        if rows.len() > 1 {
                            let open = self
                                .builder
                                .build_global_string_ptr("[", "obrm2d")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_open = vec![open.as_pointer_value().into()];
                            self.builder
                                .build_call(printf_fn, &args_open, "printf_call")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            for (ri, row) in rows.iter().enumerate() {
                                if ri > 0 {
                                    let fmt = self
                                        .builder
                                        .build_global_string_ptr("%s", "fmtrowsep")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let sep = self
                                        .builder
                                        .build_global_string_ptr("; ", "rowsep")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let args_sep: Vec<BasicValueEnum<'ctx>> = vec![
                                        fmt.as_pointer_value().into(),
                                        sep.as_pointer_value().into(),
                                    ];
                                    let argsm: Vec<_> =
                                        args_sep.into_iter().map(|v| v.into()).collect();
                                    self.builder
                                        .build_call(printf_fn, &argsm, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                }
                                for (ci, cell) in row.iter().enumerate() {
                                    if ci > 0 {
                                        let fmt = self
                                            .builder
                                            .build_global_string_ptr("%s", "fmtsp2d")
                                            .map_err(|e| {
                                                CodegenError::CompilationError(e.to_string())
                                            })?;
                                        let sp = self
                                            .builder
                                            .build_global_string_ptr(" ", "sp2d")
                                            .map_err(|e| {
                                                CodegenError::CompilationError(e.to_string())
                                            })?;
                                        let args_sp: Vec<BasicValueEnum<'ctx>> = vec![
                                            fmt.as_pointer_value().into(),
                                            sp.as_pointer_value().into(),
                                        ];
                                        let argsm: Vec<_> =
                                            args_sp.into_iter().map(|v| v.into()).collect();
                                        self.builder
                                            .build_call(printf_fn, &argsm, "printf_call")
                                            .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    }
                                    let v = self.generate_expression(cell)?;
                                    let iv = self.cast_to_int(v, self.context.i64_type())?;
                                    let fmt = self
                                        .builder
                                        .build_global_string_ptr("%lld", "fmtn2d")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let args_num: Vec<BasicValueEnum<'ctx>> =
                                        vec![fmt.as_pointer_value().into(), iv.into()];
                                    let argsm: Vec<_> =
                                        args_num.into_iter().map(|vv| vv.into()).collect();
                                    self.builder
                                        .build_call(printf_fn, &argsm, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                }
                            }
                            let close = self
                                .builder
                                .build_global_string_ptr("]", "cbrm2d")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_close = vec![close.as_pointer_value().into()];
                            self.builder
                                .build_call(printf_fn, &args_close, "printf_call")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            handled_special = true;
                        }
                    }
                }

                // If not handled, try general 1D vector printing: [e0 e1 ...]
                if !handled_special {
                    // Determine if arg is a 1D matrix/vector and get base pointer and length
                    let mut len_opt: Option<u64> = None;
                    let mut base_ptr_opt: Option<PointerValue<'ctx>> = None;
                    let mut is_matrix = false;
                    match &arg.value {
                        Expression::Matrix { rows } => {
                            // Only support single-row vector literals for now
                            if rows.len() <= 1 {
                                let l = rows.first().map(|r| r.len()).unwrap_or(0) as u64;
                                if l > 0 {
                                    len_opt = Some(l);
                                }
                                let v = self.generate_expression(&arg.value).ok();
                                base_ptr_opt = v.and_then(|bv| {
                                    if bv.is_pointer_value() {
                                        Some(bv.into_pointer_value())
                                    } else {
                                        None
                                    }
                                });
                                is_matrix = true;
                            }
                        }
                        Expression::Identifier(name) => {
                            if let Some(Type::Matrix {
                                element_type: _,
                                dimensions,
                            }) = self.semantic.get_variable_type(name)
                            {
                                if dimensions.len() == 1 {
                                    let l = dimensions[0] as u64;
                                    if l > 0 {
                                        len_opt = Some(l);
                                    }
                                    let v = self.generate_expression(&arg.value).ok();
                                    base_ptr_opt = v.and_then(|bv| {
                                        if bv.is_pointer_value() {
                                            Some(bv.into_pointer_value())
                                        } else {
                                            None
                                        }
                                    });
                                    is_matrix = true;
                                } else if dimensions.len() > 1 {
                                    // Multi-dimensional matrix: print placeholder for now
                                    let placeholder = self
                                        .builder
                                        .build_global_string_ptr("[matrix]", "matph")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let args = vec![placeholder.as_pointer_value().into()];
                                    self.builder
                                        .build_call(printf_fn, &args, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    handled_special = true;
                                }
                            } else if let Some(l) = self.vector_lengths.get(name).cloned() {
                                if l > 0 {
                                    let v = self.generate_expression(&arg.value).ok();
                                    base_ptr_opt = v.and_then(|bv| {
                                        if bv.is_pointer_value() {
                                            Some(bv.into_pointer_value())
                                        } else {
                                            None
                                        }
                                    });
                                    len_opt = Some(l);
                                    is_matrix = true;
                                }
                            } else if let Some(rank) = self.matrix_rank.get(name).cloned() {
                                if rank > 1 {
                                    let placeholder = self
                                        .builder
                                        .build_global_string_ptr("[matrix]", "matph2")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let args = vec![placeholder.as_pointer_value().into()];
                                    self.builder
                                        .build_call(printf_fn, &args, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    handled_special = true;
                                }
                            }
                        }
                        _ => {}
                    }

                    if is_matrix {
                        if let (Some(len), Some(base_ptr)) = (len_opt, base_ptr_opt) {
                            // Print opening bracket
                            let open = self
                                .builder
                                .build_global_string_ptr("[", "obrv")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_open = vec![open.as_pointer_value().into()];
                            self.builder
                                .build_call(printf_fn, &args_open, "printf_call")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                            let i64_bte: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                            let idx_alloca = self.create_entry_block_alloca("idx", i64_bte)?;
                            self.builder
                                .build_store(idx_alloca, self.context.i64_type().const_zero())
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let current_fn = self.current_function.ok_or_else(|| {
                                CodegenError::CompilationError("No current function".to_string())
                            })?;
                            let cond_bb =
                                self.context.append_basic_block(current_fn, "printvec.cond");
                            let body_bb =
                                self.context.append_basic_block(current_fn, "printvec.body");
                            let inc_bb =
                                self.context.append_basic_block(current_fn, "printvec.inc");
                            let end_bb =
                                self.context.append_basic_block(current_fn, "printvec.end");

                            self.builder
                                .build_unconditional_branch(cond_bb)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // cond
                            self.builder.position_at_end(cond_bb);
                            let idx_cur = self
                                .builder
                                .build_load(i64_bte, idx_alloca, "idx")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into_int_value();
                            let endc = self.context.i64_type().const_int(len, false);
                            let cmp = self
                                .builder
                                .build_int_compare(IntPredicate::SLT, idx_cur, endc, "cmp")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder
                                .build_conditional_branch(cmp, body_bb, end_bb)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // body
                            self.builder.position_at_end(body_bb);
                            let elem_ptr = unsafe {
                                self.builder.build_in_bounds_gep(
                                    self.context.i64_type(),
                                    base_ptr,
                                    &[idx_cur],
                                    "vidx",
                                )
                            }
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let elem_val = self
                                .builder
                                .build_load(self.context.i64_type(), elem_ptr, "velem")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // Print space if idx != 0
                            let is_zero = self
                                .builder
                                .build_int_compare(
                                    IntPredicate::EQ,
                                    idx_cur,
                                    self.context.i64_type().const_zero(),
                                    "iszero",
                                )
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let sp_then = self.context.append_basic_block(current_fn, "pv.sp.then");
                            let sp_cont = self.context.append_basic_block(current_fn, "pv.sp.cont");
                            self.builder
                                .build_conditional_branch(is_zero, sp_cont, sp_then)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder.position_at_end(sp_then);
                            let fmt_space = self
                                .builder
                                .build_global_string_ptr("%s", "fmtsv")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let space = self
                                .builder
                                .build_global_string_ptr(" ", "spv")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_sp: Vec<BasicValueEnum<'ctx>> = vec![
                                fmt_space.as_pointer_value().into(),
                                space.as_pointer_value().into(),
                            ];
                            let args_spm: Vec<_> = args_sp.into_iter().map(|v| v.into()).collect();
                            self.builder
                                .build_call(printf_fn, &args_spm, "printf_call")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder
                                .build_unconditional_branch(sp_cont)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder.position_at_end(sp_cont);
                            // Print number
                            let fmt_num = self
                                .builder
                                .build_global_string_ptr("%lld", "fmnv")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_num: Vec<BasicValueEnum<'ctx>> =
                                vec![fmt_num.as_pointer_value().into(), elem_val];
                            let args_numm: Vec<_> =
                                args_num.into_iter().map(|v| v.into()).collect();
                            self.builder
                                .build_call(printf_fn, &args_numm, "printf_call")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // jump to inc
                            self.builder
                                .build_unconditional_branch(inc_bb)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // inc
                            self.builder.position_at_end(inc_bb);
                            let idx_cur2 = self
                                .builder
                                .build_load(i64_bte, idx_alloca, "idx")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                .into_int_value();
                            let next = self
                                .builder
                                .build_int_add(
                                    idx_cur2,
                                    self.context.i64_type().const_int(1, false),
                                    "inc",
                                )
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder
                                .build_store(idx_alloca, next)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder
                                .build_unconditional_branch(cond_bb)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // end
                            self.builder.position_at_end(end_bb);
                            let close = self
                                .builder
                                .build_global_string_ptr("]", "cbrv")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_close = vec![close.as_pointer_value().into()];
                            self.builder
                                .build_call(printf_fn, &args_close, "printf_call")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let after_bb = self
                                .context
                                .append_basic_block(current_fn, "printvec.after");
                            self.builder
                                .build_unconditional_branch(after_bb)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder.position_at_end(after_bb);
                            handled_special = true;
                        }
                    }
                }

                if !handled_special {
                    // Fallback to simple formatted printing for this argument
                    let value = self.generate_expression(&arg.value)?;
                    let is_unsigned_arg = self.expression_is_unsigned(&arg.value);
                    // Detect string arguments: string literal or identifier typed as string in semantics
                    let is_string_arg = match &arg.value {
                        Expression::Literal(Literal::String(_)) => true,
                        Expression::Identifier(name) => {
                            self.local_types.get(name)
                                .or_else(|| self.semantic.get_variable_type(name))
                                .map(|t| matches!(t, crate::ast::Type::Identifier { name: s, type_args: _ } if s == "string" || s == "String" || s == "str"))
                                .unwrap_or(false)
                        }
                        _ => false,
                    };
                    if value.is_int_value()
                        && value.into_int_value().get_type().get_bit_width() == 1
                    {
                        // bool -> "true"/"false" via %s
                        let iv = value.into_int_value();
                        let fmt = self
                            .builder
                            .build_global_string_ptr("%s", "fmtb")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let t = self
                            .builder
                            .build_global_string_ptr("true", "true_str")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let f = self
                            .builder
                            .build_global_string_ptr("false", "false_str")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let sel_ptr = self
                            .builder
                            .build_select(iv, t.as_pointer_value(), f.as_pointer_value(), "boolstr")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let args: Vec<BasicValueEnum<'ctx>> =
                            vec![fmt.as_pointer_value().into(), sel_ptr.into()];
                        let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                        self.builder
                            .build_call(printf_fn, &argsm, "printf_call")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    } else if value.is_int_value()
                        && value.into_int_value().get_type().get_bit_width() == 32
                    {
                        // Treat i32 as potential char: cast to i32 and use %c
                        let iv = value.into_int_value();
                        let fmt = self
                            .builder
                            .build_global_string_ptr("%c", "fmtc")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let args: Vec<BasicValueEnum<'ctx>> =
                            vec![fmt.as_pointer_value().into(), iv.into()];
                        let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                        self.builder
                            .build_call(printf_fn, &argsm, "printf_call")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    } else if value.is_int_value() {
                        let iv = value.into_int_value();
                        let bw = iv.get_type().get_bit_width();
                        let widened: BasicValueEnum<'ctx> = if bw < 64 {
                            if is_unsigned_arg {
                                self.builder
                                    .build_int_z_extend(iv, self.context.i64_type(), "ext")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                    .into()
                            } else {
                                self.builder
                                    .build_int_s_extend(iv, self.context.i64_type(), "ext")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                    .into()
                            }
                        } else {
                            iv.into()
                        };
                        let fmt = self
                            .builder
                            .build_global_string_ptr("%lld", "fmti")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let args: Vec<BasicValueEnum<'ctx>> =
                            vec![fmt.as_pointer_value().into(), widened];
                        let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                        self.builder
                            .build_call(printf_fn, &argsm, "printf_call")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    } else if value.is_float_value() {
                        let fmt = self
                            .builder
                            .build_global_string_ptr("%f", "fmtf")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let args: Vec<BasicValueEnum<'ctx>> =
                            vec![fmt.as_pointer_value().into(), value.into()];
                        let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                        self.builder
                            .build_call(printf_fn, &argsm, "printf_call")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    } else if value.is_struct_value() {
                        let mut handled_struct = false;
                        if let Some(enum_ty) = self.enum_struct {
                            let struct_val = value.into_struct_value();
                            if struct_val.get_type() == enum_ty {
                                handled_struct = true;

                                let tag_val = self
                                    .builder
                                    .build_extract_value(struct_val, 0, "print_enum_tag")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                    .into_int_value();
                                let payload_val = self
                                    .builder
                                    .build_extract_value(struct_val, 1, "print_enum_payload")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                    .into_int_value();

                                let current_fn = self.current_function.ok_or_else(|| {
                                    CodegenError::CompilationError(
                                        "println enum printing requires current function"
                                            .to_string(),
                                    )
                                })?;
                                let check_none_bb = self
                                    .context
                                    .append_basic_block(current_fn, "printenum.check_none");
                                let some_bb = self
                                    .context
                                    .append_basic_block(current_fn, "printenum.some");
                                let none_bb = self
                                    .context
                                    .append_basic_block(current_fn, "printenum.none");
                                let other_bb = self
                                    .context
                                    .append_basic_block(current_fn, "printenum.other");
                                let cont_bb = self
                                    .context
                                    .append_basic_block(current_fn, "printenum.cont");

                                let one = self.context.i64_type().const_int(1, false);
                                let zero = self.context.i64_type().const_zero();

                                let is_some = self
                                    .builder
                                    .build_int_compare(
                                        IntPredicate::EQ,
                                        tag_val,
                                        one,
                                        "printenum_is_some",
                                    )
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_conditional_branch(is_some, some_bb, check_none_bb)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                self.builder.position_at_end(check_none_bb);
                                let is_none = self
                                    .builder
                                    .build_int_compare(
                                        IntPredicate::EQ,
                                        tag_val,
                                        zero,
                                        "printenum_is_none",
                                    )
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_conditional_branch(is_none, none_bb, other_bb)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                self.builder.position_at_end(some_bb);
                                let fmt_some = self
                                    .builder
                                    .build_global_string_ptr("%s", "fmt_some")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let some_str = self
                                    .builder
                                    .build_global_string_ptr("some ", "str_some")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let args_some: Vec<BasicValueEnum<'ctx>> = vec![
                                    fmt_some.as_pointer_value().into(),
                                    some_str.as_pointer_value().into(),
                                ];
                                let args_some_meta: Vec<_> =
                                    args_some.into_iter().map(|v| v.into()).collect();
                                self.builder
                                    .build_call(printf_fn, &args_some_meta, "printf_call")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let fmt_payload = self
                                    .builder
                                    .build_global_string_ptr("%lld", "fmt_some_payload")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let args_payload: Vec<BasicValueEnum<'ctx>> =
                                    vec![fmt_payload.as_pointer_value().into(), payload_val.into()];
                                let args_payload_meta: Vec<_> =
                                    args_payload.into_iter().map(|v| v.into()).collect();
                                self.builder
                                    .build_call(printf_fn, &args_payload_meta, "printf_call")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_unconditional_branch(cont_bb)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                self.builder.position_at_end(none_bb);
                                let fmt_none = self
                                    .builder
                                    .build_global_string_ptr("%s", "fmt_none")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let none_str = self
                                    .builder
                                    .build_global_string_ptr("none", "str_none")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let args_none: Vec<BasicValueEnum<'ctx>> = vec![
                                    fmt_none.as_pointer_value().into(),
                                    none_str.as_pointer_value().into(),
                                ];
                                let args_none_meta: Vec<_> =
                                    args_none.into_iter().map(|v| v.into()).collect();
                                self.builder
                                    .build_call(printf_fn, &args_none_meta, "printf_call")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_unconditional_branch(cont_bb)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                self.builder.position_at_end(other_bb);
                                let fmt_tag = self
                                    .builder
                                    .build_global_string_ptr("%lld", "fmt_enum_tag")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let args_tag: Vec<BasicValueEnum<'ctx>> =
                                    vec![fmt_tag.as_pointer_value().into(), tag_val.into()];
                                let args_tag_meta: Vec<_> =
                                    args_tag.into_iter().map(|v| v.into()).collect();
                                self.builder
                                    .build_call(printf_fn, &args_tag_meta, "printf_call")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder
                                    .build_unconditional_branch(cont_bb)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                self.builder.position_at_end(cont_bb);
                            }
                        }

                        if !handled_struct {
                            let fmt = self
                                .builder
                                .build_global_string_ptr("%lld", "fmt_struct_fallback")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let zero: BasicValueEnum<'ctx> =
                                self.context.i64_type().const_zero().into();
                            let args: Vec<BasicValueEnum<'ctx>> =
                                vec![fmt.as_pointer_value().into(), zero];
                            let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                            self.builder
                                .build_call(printf_fn, &argsm, "printf_call")
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        }
                    } else if value.is_pointer_value() {
                        let mut did_matrix = false;
                        // If this is a matrix-typed identifier, prefer special handling
                        if let Expression::Identifier(name) = &arg.value {
                            if let Some(Type::Matrix {
                                element_type: _,
                                dimensions,
                            }) = self.semantic.get_variable_type(name)
                            {
                                if dimensions.len() == 1 {
                                    // Print as 1D vector
                                    let len = dimensions[0] as u64;
                                    let base_ptr = value.into_pointer_value();
                                    let open = self
                                        .builder
                                        .build_global_string_ptr("[", "obrvb")
                                        .map_err(|e| {
                                        CodegenError::CompilationError(e.to_string())
                                    })?;
                                    let args_open = vec![open.as_pointer_value().into()];
                                    self.builder
                                        .build_call(printf_fn, &args_open, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;

                                    let i64_bte: BasicTypeEnum<'ctx> =
                                        self.context.i64_type().into();
                                    let idx_alloca =
                                        self.create_entry_block_alloca("idx", i64_bte)?;
                                    self.builder
                                        .build_store(
                                            idx_alloca,
                                            self.context.i64_type().const_zero(),
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let current_fn = self.current_function.ok_or_else(|| {
                                        CodegenError::CompilationError(
                                            "No current function".to_string(),
                                        )
                                    })?;
                                    let cond_bb = self
                                        .context
                                        .append_basic_block(current_fn, "printvecb.cond");
                                    let body_bb = self
                                        .context
                                        .append_basic_block(current_fn, "printvecb.body");
                                    let inc_bb = self
                                        .context
                                        .append_basic_block(current_fn, "printvecb.inc");
                                    let end_bb = self
                                        .context
                                        .append_basic_block(current_fn, "printvecb.end");

                                    self.builder.build_unconditional_branch(cond_bb).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    // cond
                                    self.builder.position_at_end(cond_bb);
                                    let idx_cur = self
                                        .builder
                                        .build_load(i64_bte, idx_alloca, "idx")
                                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                        .into_int_value();
                                    let endc = self.context.i64_type().const_int(len, false);
                                    let cmp = self
                                        .builder
                                        .build_int_compare(IntPredicate::SLT, idx_cur, endc, "cmp")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    self.builder
                                        .build_conditional_branch(cmp, body_bb, end_bb)
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    // body
                                    self.builder.position_at_end(body_bb);
                                    let elem_ptr = unsafe {
                                        self.builder.build_in_bounds_gep(
                                            self.context.i64_type(),
                                            base_ptr,
                                            &[idx_cur],
                                            "v2idx",
                                        )
                                    }
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let elem_val = self
                                        .builder
                                        .build_load(self.context.i64_type(), elem_ptr, "v2elem")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    // spacing
                                    let is_zero = self
                                        .builder
                                        .build_int_compare(
                                            IntPredicate::EQ,
                                            idx_cur,
                                            self.context.i64_type().const_zero(),
                                            "iszero",
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let sp_then =
                                        self.context.append_basic_block(current_fn, "pvb.sp.then");
                                    let sp_cont =
                                        self.context.append_basic_block(current_fn, "pvb.sp.cont");
                                    self.builder
                                        .build_conditional_branch(is_zero, sp_cont, sp_then)
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    self.builder.position_at_end(sp_then);
                                    let fmt_space = self
                                        .builder
                                        .build_global_string_ptr("%s", "fmtsvb")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let space =
                                        self.builder.build_global_string_ptr(" ", "spvb").map_err(
                                            |e| CodegenError::CompilationError(e.to_string()),
                                        )?;
                                    let args_sp: Vec<BasicValueEnum<'ctx>> = vec![
                                        fmt_space.as_pointer_value().into(),
                                        space.as_pointer_value().into(),
                                    ];
                                    let args_spm: Vec<_> =
                                        args_sp.into_iter().map(|v| v.into()).collect();
                                    self.builder
                                        .build_call(printf_fn, &args_spm, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    self.builder.build_unconditional_branch(sp_cont).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    self.builder.position_at_end(sp_cont);
                                    // number
                                    let fmt_num = self
                                        .builder
                                        .build_global_string_ptr("%lld", "fmnvb")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let args_num: Vec<BasicValueEnum<'ctx>> =
                                        vec![fmt_num.as_pointer_value().into(), elem_val];
                                    let args_numm: Vec<_> =
                                        args_num.into_iter().map(|v| v.into()).collect();
                                    self.builder
                                        .build_call(printf_fn, &args_numm, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    // inc
                                    self.builder.build_unconditional_branch(inc_bb).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    self.builder.position_at_end(inc_bb);
                                    let idx_cur2 = self
                                        .builder
                                        .build_load(i64_bte, idx_alloca, "idx")
                                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?
                                        .into_int_value();
                                    let next = self
                                        .builder
                                        .build_int_add(
                                            idx_cur2,
                                            self.context.i64_type().const_int(1, false),
                                            "inc",
                                        )
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    self.builder.build_store(idx_alloca, next).map_err(|e| {
                                        CodegenError::CompilationError(e.to_string())
                                    })?;
                                    self.builder.build_unconditional_branch(cond_bb).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    // end
                                    self.builder.position_at_end(end_bb);
                                    let close = self
                                        .builder
                                        .build_global_string_ptr("]", "cbrvb")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let args_close = vec![close.as_pointer_value().into()];
                                    self.builder
                                        .build_call(printf_fn, &args_close, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let after_bb = self
                                        .context
                                        .append_basic_block(current_fn, "printvecb.after");
                                    self.builder.build_unconditional_branch(after_bb).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                    self.builder.position_at_end(after_bb);
                                    did_matrix = true;
                                } else if dimensions.len() > 1 {
                                    // Multi-dimensional: safe placeholder
                                    let placeholder = self
                                        .builder
                                        .build_global_string_ptr("[matrix]", "matphb")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    let args = vec![placeholder.as_pointer_value().into()];
                                    self.builder
                                        .build_call(printf_fn, &args, "printf_call")
                                        .map_err(|e| {
                                            CodegenError::CompilationError(e.to_string())
                                        })?;
                                    did_matrix = true;
                                }
                            }
                        }
                        if !did_matrix {
                            if is_string_arg {
                                // Print as C string
                                let fmt = self
                                    .builder
                                    .build_global_string_ptr("%s", "fmts")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let args: Vec<BasicValueEnum<'ctx>> =
                                    vec![fmt.as_pointer_value().into(), value.into()];
                                let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                                self.builder
                                    .build_call(printf_fn, &argsm, "printf_call")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            } else {
                                // Non-string pointers as %p
                                let fmt = self
                                    .builder
                                    .build_global_string_ptr("%p", "fmtp")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let args: Vec<BasicValueEnum<'ctx>> =
                                    vec![fmt.as_pointer_value().into(), value.into()];
                                let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                                self.builder
                                    .build_call(printf_fn, &argsm, "printf_call")
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            }
                        }
                    } else {
                        let fmt = self
                            .builder
                            .build_global_string_ptr("%lld", "fmti")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let zero: BasicValueEnum<'ctx> =
                            self.context.i64_type().const_zero().into();
                        let args: Vec<BasicValueEnum<'ctx>> =
                            vec![fmt.as_pointer_value().into(), zero];
                        let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                        self.builder
                            .build_call(printf_fn, &argsm, "printf_call")
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    }
                }

                // Add a space between arguments (not at end)
                if i < arguments.len() - 1 {
                    let space = self
                        .builder
                        .build_global_string_ptr(" ", "spc")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let args: Vec<BasicValueEnum<'ctx>> = vec![space.as_pointer_value().into()];
                    let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                    self.builder
                        .build_call(printf_fn, &argsm, "printf_call")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                }
            }
            // finally, newline
            let newline = self
                .builder
                .build_global_string_ptr("\n", "nl")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            let args_nl = vec![newline.as_pointer_value().into()];
            self.builder
                .build_call(printf_fn, &args_nl, "printf_call")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        }

        Ok(self.context.i32_type().const_int(0, false).into())
    }

    fn generate_for_loop_over_slice(
        &mut self,
        variable: &str,
        body: &[Statement],
        slice_alloca: PointerValue<'ctx>,
        slice_struct_ty: StructType<'ctx>,
        field_order: Vec<String>,
        struct_name: &str,
        element_ty: BasicTypeEnum<'ctx>,
    ) -> Result<(), CodegenError> {
        let ptr_idx = field_order.iter().position(|n| n == "ptr").ok_or_else(|| {
            CodegenError::InvalidOperation(format!(
                "{}.ptr field missing for slice iteration",
                struct_name
            ))
        })? as u32;
        let len_idx = field_order.iter().position(|n| n == "len").ok_or_else(|| {
            CodegenError::InvalidOperation(format!(
                "{}.len field missing for slice iteration",
                struct_name
            ))
        })? as u32;

        let ptr_field_ty = slice_struct_ty
            .get_field_type_at_index(ptr_idx)
            .ok_or_else(|| {
                CodegenError::InvalidOperation(format!(
                    "{}.ptr type missing for slice iteration",
                    struct_name
                ))
            })?;
        let len_field_ty = slice_struct_ty
            .get_field_type_at_index(len_idx)
            .ok_or_else(|| {
                CodegenError::InvalidOperation(format!(
                    "{}.len type missing for slice iteration",
                    struct_name
                ))
            })?;

        let ptr_gep = self
            .builder
            .build_struct_gep(slice_struct_ty, slice_alloca, ptr_idx, "slice.ptr")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let len_gep = self
            .builder
            .build_struct_gep(slice_struct_ty, slice_alloca, len_idx, "slice.len")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        let base_ptr_val = self
            .builder
            .build_load(ptr_field_ty, ptr_gep, "slice.ptrv")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let len_val = self
            .builder
            .build_load(len_field_ty, len_gep, "slice.lenv")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();

        let base_ptr = base_ptr_val.into_pointer_value();

        let idx_alloca = self.create_entry_block_alloca("idx", self.context.i64_type().into())?;
        self.builder
            .build_store(idx_alloca, self.context.i64_type().const_zero())
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let elem_alloca = self.create_entry_block_alloca(variable, element_ty)?;
        let prev_binding = self
            .variables
            .insert(variable.to_string(), (elem_alloca, element_ty));

        let current_fn = self
            .current_function
            .ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
        let cond_bb = self.context.append_basic_block(current_fn, "for.cond");
        let body_bb = self.context.append_basic_block(current_fn, "for.body");
        let inc_bb = self.context.append_basic_block(current_fn, "for.inc");
        let end_bb = self.context.append_basic_block(current_fn, "for.end");

        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        // cond block
        self.builder.position_at_end(cond_bb);
        let i64_bte: BasicTypeEnum<'ctx> = self.context.i64_type().into();
        let idx_cur = self
            .builder
            .build_load(i64_bte, idx_alloca, "idx")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let cmp = self
            .builder
            .build_int_compare(IntPredicate::SLT, idx_cur, len_val, "forcmp")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_conditional_branch(cmp, body_bb, end_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        // body block
        self.builder.position_at_end(body_bb);
        let elem_ptr = unsafe {
            match element_ty {
                BasicTypeEnum::IntType(it) => {
                    self.builder
                        .build_in_bounds_gep(it, base_ptr, &[idx_cur], "slice.idx")
                }
                BasicTypeEnum::FloatType(ft) => {
                    self.builder
                        .build_in_bounds_gep(ft, base_ptr, &[idx_cur], "slice.idx")
                }
                BasicTypeEnum::PointerType(pt) => {
                    self.builder
                        .build_in_bounds_gep(pt, base_ptr, &[idx_cur], "slice.idx")
                }
                BasicTypeEnum::StructType(st) => {
                    self.builder
                        .build_in_bounds_gep(st, base_ptr, &[idx_cur], "slice.idx")
                }
                BasicTypeEnum::ArrayType(at) => {
                    self.builder
                        .build_in_bounds_gep(at, base_ptr, &[idx_cur], "slice.idx")
                }
                BasicTypeEnum::VectorType(vt) => {
                    self.builder
                        .build_in_bounds_gep(vt, base_ptr, &[idx_cur], "slice.idx")
                }
                BasicTypeEnum::ScalableVectorType(_) => {
                    return Err(CodegenError::InvalidOperation(
                        "Unsupported scalable vector slice element".to_string(),
                    ))
                }
            }
        }
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        let loaded_elem = self
            .builder
            .build_load(element_ty, elem_ptr, "slice.elem")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(elem_alloca, loaded_elem)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        {
            let _loop_scope = LoopScope::new(&mut self.loop_stack, inc_bb, end_bb);
            for stmt in body {
                self.generate_statement(stmt)?;
            }
            self.branch_to(inc_bb)?;
        }

        // inc block
        self.builder.position_at_end(inc_bb);
        let idx_cur2 = self
            .builder
            .build_load(i64_bte, idx_alloca, "idx")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?
            .into_int_value();
        let next = self
            .builder
            .build_int_add(idx_cur2, self.context.i64_type().const_int(1, false), "inc")
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_store(idx_alloca, next)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(cond_bb)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        // end block
        self.builder.position_at_end(end_bb);
        if let Some(prev) = prev_binding {
            self.variables.insert(variable.to_string(), prev);
        } else {
            self.variables.remove(variable);
        }

        Ok(())
    }

    fn create_entry_block_alloca(
        &self,
        name: &str,
        ty: BasicTypeEnum<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let current_function = self
            .current_function
            .ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;

        let builder = self.context.create_builder();
        let entry = current_function
            .get_first_basic_block()
            .ok_or_else(|| CodegenError::CompilationError("No entry block".to_string()))?;

        match entry.get_first_instruction() {
            Some(first_instr) => builder.position_before(&first_instr),
            None => builder.position_at_end(entry),
        }

        builder
            .build_alloca(ty, name)
            .map_err(|e| CodegenError::CompilationError(e.to_string()))
    }

    pub fn print_ir(&self) {
        self.module.print_to_stderr();
    }

    pub fn ir_to_string(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn write_object_file(&self, filename: &str) -> Result<(), CodegenError> {
        // Verify module before emitting object to avoid backend crashes
        if let Err(msg) = self.module.verify() {
            self.module.print_to_stderr();
            return Err(CodegenError::CompilationError(format!(
                "LLVM IR verification failed: {}",
                msg
            )));
        }
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
                inkwell::targets::RelocMode::PIC,
                inkwell::targets::CodeModel::Default,
            )
            .ok_or_else(|| {
                CodegenError::CompilationError("Failed to create target machine".to_string())
            })?;

        self.module.set_triple(&target_triple);

        target_machine
            .write_to_file(
                &self.module,
                inkwell::targets::FileType::Object,
                filename.as_ref(),
            )
            .map_err(|e| CodegenError::CompilationError(e.to_string()))
    }

    fn declare_and_define_functions(&mut self, program: &Program) -> Result<(), CodegenError> {
        // First pass: declare all functions with mapped param/ret types (skip top-level main; we'll build the real C entry separately)
        for stmt in &program.statements {
            match stmt {
                Statement::ConstDecl {
                    name,
                    value,
                    extern_linkage,
                    ..
                } => {
                    if let ConstValue::Expression(Expression::Function {
                        parameters,
                        return_type,
                        ..
                    }) = value
                    {
                        let ret_ty = return_type
                            .as_ref()
                            .and_then(|t| self.map_ast_type(t))
                            .unwrap_or(self.context.i64_type().into());
                        let param_tys_bte: Vec<BasicTypeEnum<'ctx>> = parameters
                            .iter()
                            .map(|p| {
                                p.param_type
                                    .as_ref()
                                    .and_then(|t| self.map_ast_type(t))
                                    .unwrap_or(self.context.i64_type().into())
                            })
                            .collect();
                        let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> =
                            param_tys_bte.iter().map(|t| (*t).into()).collect();
                        let fn_type = match ret_ty {
                            BasicTypeEnum::IntType(it) => it.fn_type(&param_meta, false),
                            BasicTypeEnum::FloatType(ft) => ft.fn_type(&param_meta, false),
                            BasicTypeEnum::PointerType(pt) => pt.fn_type(&param_meta, false),
                            BasicTypeEnum::StructType(st) => st.fn_type(&param_meta, false),
                            _ => self.context.i64_type().fn_type(&param_meta, false),
                        };
                        // In runtime mode, emit user main as `tricti_main` symbol
                        let fname = if name == "main" { "tricti_main" } else { name };
                        // If extern_linkage is present, declare as external function
                        let linkage = if extern_linkage.is_some() {
                            Some(Linkage::External)
                        } else {
                            None
                        };
                        let f = self.module.add_function(fname, fn_type, linkage);
                        self.functions.insert(fname.to_string(), f);
                    } else if let ConstValue::SystemDef(system_def) = value {
                        // Handle system function declarations
                        let ret_ty = system_def
                            .return_type
                            .as_ref()
                            .and_then(|t| self.map_ast_type(t))
                            .unwrap_or(self.context.i64_type().into());

                        let param_tys_bte: Vec<BasicTypeEnum<'ctx>> = system_def
                            .parameters
                            .iter()
                            .map(|p| {
                                match p {
                                    SystemParameter::Query { .. } => {
                                        // Query parameters are passed as opaque pointers to query results
                                        self.context.ptr_type(AddressSpace::default()).into()
                                    }
                                    SystemParameter::Resource {
                                        resource_type,
                                        access,
                                        ..
                                    } => {
                                        match access {
                                            ResourceAccess::Immutable | ResourceAccess::Mutable => {
                                                // Reference parameters are passed as pointers
                                                self.context
                                                    .ptr_type(AddressSpace::default())
                                                    .into()
                                            }
                                            ResourceAccess::Owned => {
                                                // Owned parameters use the actual type
                                                self.map_ast_type(resource_type)
                                                    .unwrap_or(self.context.i64_type().into())
                                            }
                                        }
                                    }
                                    SystemParameter::Regular { value_type, .. } => self
                                        .map_ast_type(value_type)
                                        .unwrap_or(self.context.i64_type().into()),
                                }
                            })
                            .collect();

                        let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> =
                            param_tys_bte.iter().map(|t| (*t).into()).collect();

                        let fn_type = match ret_ty {
                            BasicTypeEnum::IntType(it) => it.fn_type(&param_meta, false),
                            BasicTypeEnum::FloatType(ft) => ft.fn_type(&param_meta, false),
                            BasicTypeEnum::PointerType(pt) => pt.fn_type(&param_meta, false),
                            BasicTypeEnum::StructType(st) => st.fn_type(&param_meta, false),
                            _ => self.context.i64_type().fn_type(&param_meta, false),
                        };

                        // System functions get a `sys_` prefix
                        let fname = format!("sys_{}", name);
                        let f = self.module.add_function(&fname, fn_type, None);
                        self.functions.insert(fname, f);
                    }
                }
                Statement::ImplMethod {
                    name,
                    type_params: _,
                    parameters,
                    return_type,
                    body,
                } => {
                    // Generate method function
                    let mangled = format!(
                        "{}_{}",
                        self.current_impl_struct.as_deref().unwrap_or("unknown"),
                        name
                    );
                    let ret_ty = return_type
                        .as_ref()
                        .and_then(|t| self.map_ast_type(t))
                        .unwrap_or(self.context.i64_type().into());
                    if name == "get" {
                        let ty_str = ret_ty.print_to_string().to_string();
                        eprintln!("declaring method {} return type {}", name, ty_str);
                    }
                    let mut param_tys_bte: Vec<BasicTypeEnum<'ctx>> = Vec::new();
                    // Prepend receiver pointer parameter for methods
                    param_tys_bte.push(self.context.ptr_type(AddressSpace::default()).into());
                    param_tys_bte.extend(parameters.iter().map(|p| {
                        p.param_type
                            .as_ref()
                            .and_then(|t| self.map_ast_type(t))
                            .unwrap_or(self.context.i64_type().into())
                    }));
                    let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> =
                        param_tys_bte.iter().map(|t| (*t).into()).collect();
                    let fn_type = match ret_ty {
                        BasicTypeEnum::IntType(it) => it.fn_type(&param_meta, false),
                        BasicTypeEnum::FloatType(ft) => ft.fn_type(&param_meta, false),
                        BasicTypeEnum::PointerType(pt) => pt.fn_type(&param_meta, false),
                        BasicTypeEnum::StructType(st) => st.fn_type(&param_meta, false),
                        BasicTypeEnum::ArrayType(at) => at.fn_type(&param_meta, false),
                        _ => self.context.i64_type().fn_type(&param_meta, false),
                    };
                    let function = self.module.add_function(&mangled, fn_type, None);
                    if name == "get" {
                        eprintln!(
                            "declaring method {} fn_type {}",
                            mangled,
                            fn_type.print_to_string().to_string()
                        );
                    }
                    self.functions.insert(mangled.clone(), function);

                    // Define the function body
                    let entry = self.context.append_basic_block(function, "entry");
                    let prev_insert_block = self.builder.get_insert_block();
                    let prev_fn = self.current_function;
                    let prev_vars = std::mem::take(&mut self.variables);
                    let prev_owned = std::mem::take(&mut self.owned_locals);
                    self.current_function = Some(function);
                    let prev_ret_ast = self.current_function_return_ast.clone();
                    let ret_ast = return_type.clone().unwrap_or(Type::None);
                    self.current_function_return_ast = Some(ret_ast.clone());
                    self.builder.position_at_end(entry);

                    // Bind parameters to allocas
                    for (i, param) in function.get_param_iter().enumerate() {
                        let param_ty = param_tys_bte[i];
                        let alloca =
                            self.create_entry_block_alloca(&format!("arg{}", i), param_ty)?;
                        self.builder
                            .build_store(alloca, param)
                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        if i == 0 {
                            // Receiver (self)
                            self.variables
                                .insert("self".to_string(), (alloca, param_ty));
                            if let Some(struct_name) = &self.current_impl_struct {
                                let self_ty = crate::ast::Type::Pointer {
                                    is_mutable: false,
                                    pointee: Box::new(crate::ast::Type::Identifier {
                                        name: struct_name.clone(),
                                        type_args: vec![],
                                    }),
                                };
                                self.local_types.insert("self".to_string(), self_ty);
                                self.track_owned_binding("self");
                            }
                        } else {
                            let param_name = parameters
                                .get(i - 1)
                                .map(|p| p.name.clone())
                                .unwrap_or(format!("arg{}", i - 1));
                            self.variables
                                .insert(param_name.clone(), (alloca, param_ty));
                            if let Some(ast_ty) =
                                parameters.get(i - 1).and_then(|p| p.param_type.clone())
                            {
                                self.record_unsigned_binding(&param_name, &ast_ty);
                                self.local_types.insert(param_name.clone(), ast_ty);
                                self.track_owned_binding(&param_name);
                            }
                        }
                    }

                    // Generate body
                    match body {
                        FunctionBody::Expression(expr) => {
                            let val = self.generate_expression(expr)?;
                            if let Some(current_bb) = self.builder.get_insert_block() {
                                if current_bb.get_terminator().is_some() {
                                    // Body already emitted a terminator (e.g., return inside expression)
                                    self.variables = prev_vars;
                                    self.current_function = prev_fn;
                                    self.current_function_return_ast = prev_ret_ast;
                                    self.owned_locals = prev_owned;
                                    if let Some(bb) = prev_insert_block {
                                        self.builder.position_at_end(bb);
                                    }
                                    continue;
                                }
                            }
                            self.mark_expr_moved(expr);
                            self.drop_all_owned_locals()?;
                            self.builder
                                .build_return(Some(&val))
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        }
                        FunctionBody::Block(statements) => {
                            for stmt in statements {
                                self.generate_statement(stmt)?;
                            }
                            // If no return, add implicit return
                            if !Self::statements_contain_return(statements) {
                                self.drop_all_owned_locals()?;
                                let ret_ast = self
                                    .current_function_return_ast
                                    .clone()
                                    .unwrap_or(crate::ast::Type::None);
                                if let Some(ret_ty) = self.map_ast_type(&ret_ast) {
                                    let default_value = self.default_value_for_type(ret_ty);
                                    self.builder.build_return(Some(&default_value)).map_err(
                                        |e| CodegenError::CompilationError(e.to_string()),
                                    )?;
                                } else {
                                    self.builder.build_return(None).map_err(|e| {
                                        CodegenError::CompilationError(e.to_string())
                                    })?;
                                }
                            }
                        }
                    }

                    // Restore state
                    self.variables = prev_vars;
                    self.current_function = prev_fn;
                    self.current_function_return_ast = prev_ret_ast;
                    self.owned_locals = prev_owned;
                    if let Some(bb) = prev_insert_block {
                        self.builder.position_at_end(bb);
                    }
                }
                Statement::ImplBlock {
                    type_params: _,
                    trait_name,
                    type_name,
                    self_type: _,
                    methods,
                } => {
                    for m in methods {
                        match m {
                            Statement::ConstDecl {
                                name: mname, value, ..
                            } => {
                                if let ConstValue::Expression(Expression::Function {
                                    parameters,
                                    return_type,
                                    ..
                                }) = value
                                {
                                    let mangled = match trait_name {
                                        Some(tn) => format!("{}_{}_{}", tn, type_name, mname),
                                        None => format!("{}_{}", type_name, mname),
                                    };
                                    let ret_ty = return_type
                                        .as_ref()
                                        .and_then(|t| self.map_ast_type(t))
                                        .unwrap_or(self.context.i64_type().into());
                                    let mut param_tys_bte: Vec<BasicTypeEnum<'ctx>> = Vec::new();
                                    param_tys_bte.push(
                                        self.context.ptr_type(AddressSpace::default()).into(),
                                    );
                                    param_tys_bte.extend(parameters.iter().map(|p| {
                                        p.param_type
                                            .as_ref()
                                            .and_then(|t| self.map_ast_type(t))
                                            .unwrap_or(self.context.i64_type().into())
                                    }));
                                    let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> =
                                        param_tys_bte.iter().map(|t| (*t).into()).collect();
                                    let fn_type = match ret_ty {
                                        BasicTypeEnum::IntType(it) => {
                                            it.fn_type(&param_meta, false)
                                        }
                                        BasicTypeEnum::FloatType(ft) => {
                                            ft.fn_type(&param_meta, false)
                                        }
                                        BasicTypeEnum::PointerType(pt) => {
                                            pt.fn_type(&param_meta, false)
                                        }
                                        BasicTypeEnum::StructType(st) => {
                                            st.fn_type(&param_meta, false)
                                        }
                                        _ => self.context.i64_type().fn_type(&param_meta, false),
                                    };
                                    let f = self.module.add_function(&mangled, fn_type, None);
                                    self.functions.insert(mangled, f);
                                }
                            }
                            Statement::ImplMethod {
                                name: mname,
                                parameters,
                                return_type,
                                ..
                            } => {
                                let mangled = match trait_name {
                                    Some(tn) => format!("{}_{}_{}", tn, type_name, mname),
                                    None => format!("{}_{}", type_name, mname),
                                };
                                let ret_ty = return_type
                                    .as_ref()
                                    .and_then(|t| self.map_ast_type(t))
                                    .unwrap_or(self.context.i64_type().into());
                                let mut param_tys_bte: Vec<BasicTypeEnum<'ctx>> = Vec::new();
                                param_tys_bte
                                    .push(self.context.ptr_type(AddressSpace::default()).into());
                                param_tys_bte.extend(parameters.iter().map(|p| {
                                    p.param_type
                                        .as_ref()
                                        .and_then(|t| self.map_ast_type(t))
                                        .unwrap_or(self.context.i64_type().into())
                                }));
                                let param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> =
                                    param_tys_bte.iter().map(|t| (*t).into()).collect();
                                let fn_type = match ret_ty {
                                    BasicTypeEnum::IntType(it) => it.fn_type(&param_meta, false),
                                    BasicTypeEnum::FloatType(ft) => ft.fn_type(&param_meta, false),
                                    BasicTypeEnum::PointerType(pt) => {
                                        pt.fn_type(&param_meta, false)
                                    }
                                    BasicTypeEnum::StructType(st) => st.fn_type(&param_meta, false),
                                    _ => self.context.i64_type().fn_type(&param_meta, false),
                                };
                                let f = self.module.add_function(&mangled, fn_type, None);
                                self.functions.insert(mangled, f);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Second pass: define bodies
        for stmt in &program.statements {
            match stmt {
                Statement::ConstDecl {
                    name,
                    value,
                    extern_linkage,
                    ..
                } => {
                    // Skip extern functions - they're declared but not defined
                    if extern_linkage.is_some() {
                        continue;
                    }
                    if let ConstValue::Expression(Expression::Function {
                        parameters,
                        body,
                        return_type,
                        ..
                    }) = value
                    {
                        let fname = if name == "main" { "tricti_main" } else { name };
                        let f = match self.functions.get(fname) {
                            Some(f) => *f,
                            None => continue,
                        };
                        let entry = self.context.append_basic_block(f, "entry");
                        self.builder.position_at_end(entry);
                        let prev_fn = self.current_function;
                        let prev_vars = std::mem::take(&mut self.variables);
                        let prev_owned = std::mem::take(&mut self.owned_locals);
                        self.current_function = Some(f);
                        let prev_ret_ast = self.current_function_return_ast.clone();
                        let ret_ast = return_type.clone().unwrap_or(Type::None);
                        self.current_function_return_ast = Some(ret_ast.clone());

                        // Bind params to allocas
                        for (i, param) in f.get_param_iter().enumerate() {
                            let p_name = parameters
                                .get(i)
                                .map(|p| p.name.clone())
                                .unwrap_or(format!("arg{}", i));
                            let p_ty = parameters
                                .get(i)
                                .and_then(|p| p.param_type.as_ref())
                                .and_then(|t| self.map_ast_type(t))
                                .unwrap_or(self.context.i64_type().into());
                            let alloca = self.create_entry_block_alloca(&p_name, p_ty)?;
                            self.builder
                                .build_store(alloca, param)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.variables.insert(p_name.clone(), (alloca, p_ty));
                            let ast_ty = parameters
                                .get(i)
                                .and_then(|p| p.param_type.clone())
                                .unwrap_or(crate::ast::Type::Identifier {
                                    name: "i64".to_string(),
                                    type_args: vec![],
                                });
                            self.record_unsigned_binding(&p_name, &ast_ty);
                            self.local_types.insert(p_name.clone(), ast_ty);
                            self.track_owned_binding(&p_name);
                        }

                        match body {
                            FunctionBody::Expression(expr) => {
                                let ret_ty = return_type
                                    .as_ref()
                                    .and_then(|t| self.map_ast_type(t))
                                    .unwrap_or(self.context.i64_type().into());
                                // If returning a struct and the body is a struct literal or a block whose last expr is a struct literal, build using known struct layout
                                if let BasicTypeEnum::StructType(st_ret) = ret_ty {
                                    // Direct struct literal
                                    if let Expression::StructLiteral {
                                        type_name: _,
                                        fields,
                                    } = expr.as_ref()
                                    {
                                        if let Some((struct_name, (st_known, order))) = self
                                            .struct_types
                                            .iter()
                                            .find(|(_, (llvm_st, _))| *llvm_st == st_ret)
                                        {
                                            let sname = struct_name.clone();
                                            let st_copy = *st_known;
                                            let order_clone = order.clone();
                                            let sval = self.build_struct_literal_value(
                                                &sname,
                                                fields,
                                                st_copy,
                                                &order_clone,
                                            )?;
                                            self.mark_expr_moved(expr);
                                            self.drop_all_owned_locals()?;
                                            self.try_build_return(Some(
                                                &sval as &dyn inkwell::values::BasicValue,
                                            ))?;
                                            // Early return from function body handler
                                            // Restore state happens after match
                                        } else {
                                            // No known layout; fall through to generic expression path
                                            let v = self.generate_expression(expr)?;
                                            self.mark_expr_moved(expr);
                                            self.drop_all_owned_locals()?;
                                            let casted = self.cast_basic_to_type(v, ret_ty)?;
                                            self.try_build_return(Some(
                                                &casted as &dyn inkwell::values::BasicValue,
                                            ))?;
                                        }
                                    } else if let Expression::Block { statements } = expr.as_ref() {
                                        // If last statement is an expression struct literal, build it
                                        if let Some((last, prefix)) = statements.split_last() {
                                            for s in prefix {
                                                let _ = self.generate_statement(s);
                                            }
                                            if let Statement::Expression(
                                                Expression::StructLiteral {
                                                    type_name: _,
                                                    fields,
                                                },
                                            ) = last
                                            {
                                                if let Some((struct_name, (st_known, order))) = self
                                                    .struct_types
                                                    .iter()
                                                    .find(|(_, (llvm_st, _))| *llvm_st == st_ret)
                                                {
                                                    let sname = struct_name.clone();
                                                    let st_copy = *st_known;
                                                    let order_clone = order.clone();
                                                    let sval = self.build_struct_literal_value(
                                                        &sname,
                                                        fields,
                                                        st_copy,
                                                        &order_clone,
                                                    )?;
                                                    self.mark_expr_moved(expr);
                                                    self.drop_all_owned_locals()?;
                                                    self.try_build_return(Some(
                                                        &sval as &dyn inkwell::values::BasicValue,
                                                    ))?;
                                                } else {
                                                    // Fall back to evaluating the block as a value
                                                    let v = self.generate_expression(expr)?;
                                                    self.mark_expr_moved(expr);
                                                    self.drop_all_owned_locals()?;
                                                    let casted =
                                                        self.cast_basic_to_type(v, ret_ty)?;
                                                    self.try_build_return(Some(
                                                        &casted as &dyn inkwell::values::BasicValue,
                                                    ))?;
                                                }
                                            } else {
                                                // No struct-literal tail; evaluate the block
                                                let v = self.generate_expression(expr)?;
                                                self.mark_expr_moved(expr);
                                                self.drop_all_owned_locals()?;
                                                let casted = self.cast_basic_to_type(v, ret_ty)?;
                                                self.try_build_return(Some(
                                                    &casted as &dyn inkwell::values::BasicValue,
                                                ))?;
                                            }
                                        } else {
                                            // Empty block, return zero init for struct
                                            if let Some((struct_name, (st_known, order))) = self
                                                .struct_types
                                                .iter()
                                                .find(|(_, (llvm_st, _))| *llvm_st == st_ret)
                                            {
                                                let empty_fields: std::collections::HashMap<
                                                    String,
                                                    Expression,
                                                > = std::collections::HashMap::new();
                                                let sname = struct_name.clone();
                                                let st_copy = *st_known;
                                                let order_clone = order.clone();
                                                let sval = self.build_struct_literal_value(
                                                    &sname,
                                                    &empty_fields,
                                                    st_copy,
                                                    &order_clone,
                                                )?;
                                                self.mark_expr_moved(expr);
                                                self.drop_all_owned_locals()?;
                                                self.try_build_return(Some(
                                                    &sval as &dyn inkwell::values::BasicValue,
                                                ))?;
                                            } else {
                                                // Unknown layout but we still know the struct type; return a zero-initialized struct
                                                let zero = st_ret.const_zero();
                                                self.mark_expr_moved(expr);
                                                self.drop_all_owned_locals()?;
                                                self.try_build_return(Some(
                                                    &zero as &dyn inkwell::values::BasicValue,
                                                ))?;
                                            }
                                        }
                                    } else {
                                        let v = self.generate_expression(expr)?;
                                        self.mark_expr_moved(expr);
                                        self.drop_all_owned_locals()?;
                                        let casted = self.cast_basic_to_type(v, ret_ty)?;
                                        self.try_build_return(Some(
                                            &casted as &dyn inkwell::values::BasicValue,
                                        ))?;
                                    }
                                } else {
                                    let v = self.generate_expression(expr)?;
                                    self.mark_expr_moved(expr);
                                    self.drop_all_owned_locals()?;
                                    let casted = self.cast_basic_to_type(v, ret_ty)?;
                                    self.try_build_return(Some(
                                        &casted as &dyn inkwell::values::BasicValue,
                                    ))?;
                                }
                            }
                            FunctionBody::Block(stmts) => {
                                // Execute all statements; if the last is an expression, return its value
                                let ret_ty = return_type
                                    .as_ref()
                                    .and_then(|t| self.map_ast_type(t))
                                    .unwrap_or(self.context.i64_type().into());
                                let mut last_expr_value: Option<BasicValueEnum<'ctx>> = None;
                                let slice: &[Statement] = &stmts[..];
                                if let Some((last, prefix)) = slice.split_last() {
                                    for s in prefix {
                                        let _ = self.generate_statement(s);
                                    }
                                    match last {
                                        Statement::Expression(expr) => {
                                            // If returning a struct and the last expr is a struct literal, build it using layout
                                            if let BasicTypeEnum::StructType(st_ret) = ret_ty {
                                                if let Expression::StructLiteral {
                                                    type_name: _,
                                                    fields,
                                                } = expr
                                                {
                                                    if let Some((struct_name, (st_known, order))) =
                                                        self.struct_types.iter().find(
                                                            |(_, (llvm_st, _))| *llvm_st == st_ret,
                                                        )
                                                    {
                                                        let sname = struct_name.clone();
                                                        let st_copy = *st_known;
                                                        let order_clone = order.clone();
                                                        let sval = self
                                                            .build_struct_literal_value(
                                                                &sname,
                                                                fields,
                                                                st_copy,
                                                                &order_clone,
                                                            )?;
                                                        last_expr_value = Some(sval);
                                                    } else {
                                                        let v = self.generate_expression(expr)?;
                                                        last_expr_value = Some(v);
                                                    }
                                                } else {
                                                    let v = self.generate_expression(expr)?;
                                                    last_expr_value = Some(v);
                                                }
                                            } else {
                                                let v = self.generate_expression(expr)?;
                                                last_expr_value = Some(v);
                                            }
                                        }
                                        other => {
                                            let _ = self.generate_statement(other);
                                        }
                                    }
                                    if let Statement::Expression(expr) = last {
                                        self.mark_expr_moved(expr);
                                    }
                                }
                                let ret_val: BasicValueEnum<'ctx> = if let Some(v) = last_expr_value
                                {
                                    self.cast_basic_to_type(v, ret_ty)?
                                } else {
                                    match ret_ty {
                                        BasicTypeEnum::IntType(it) => it.const_zero().into(),
                                        BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
                                        BasicTypeEnum::PointerType(pt) => pt.const_zero().into(),
                                        BasicTypeEnum::StructType(st_ret) => {
                                            if let Some((struct_name, (st_known, order))) = self
                                                .struct_types
                                                .iter()
                                                .find(|(_, (llvm_st, _))| *llvm_st == st_ret)
                                            {
                                                let empty_fields: std::collections::HashMap<
                                                    String,
                                                    Expression,
                                                > = std::collections::HashMap::new();
                                                let sname = struct_name.clone();
                                                let st_copy = *st_known;
                                                let order_clone = order.clone();
                                                self.build_struct_literal_value(
                                                    &sname,
                                                    &empty_fields,
                                                    st_copy,
                                                    &order_clone,
                                                )?
                                            } else {
                                                st_ret.const_zero().into()
                                            }
                                        }
                                        _ => self.context.i64_type().const_zero().into(),
                                    }
                                };
                                self.drop_all_owned_locals()?;
                                self.builder
                                    .build_return(Some(&ret_val))
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            }
                        }

                        // Restore state for next
                        self.current_function_return_ast = prev_ret_ast;
                        self.variables = prev_vars;
                        self.owned_locals = prev_owned;
                        self.local_types.clear();
                        self.unsigned_variables.clear();
                        self.current_function = prev_fn;
                    } else if let ConstValue::SystemDef(system_def) = value {
                        // Handle system function body generation
                        let fname = format!("sys_{}", name);
                        let f = match self.functions.get(&fname) {
                            Some(f) => *f,
                            None => continue,
                        };

                        let entry = self.context.append_basic_block(f, "entry");
                        self.builder.position_at_end(entry);
                        let prev_fn = self.current_function;
                        let prev_vars = std::mem::take(&mut self.variables);
                        let prev_owned = std::mem::take(&mut self.owned_locals);
                        self.current_function = Some(f);
                        let prev_ret_ast = self.current_function_return_ast.clone();
                        let ret_ast = system_def.return_type.clone().unwrap_or(Type::None);
                        self.current_function_return_ast = Some(ret_ast);

                        // Bind system parameters to allocas
                        for (i, param) in f.get_param_iter().enumerate() {
                            if let Some(system_param) = system_def.parameters.get(i) {
                                let (p_name, p_ty, ast_ty_opt) = match system_param {
                                    SystemParameter::Query { name, .. } => (
                                        name.clone(),
                                        self.context.ptr_type(AddressSpace::default()).into(),
                                        None,
                                    ),
                                    SystemParameter::Resource {
                                        param_type: _,
                                        name,
                                        resource_type,
                                        access,
                                    } => match access {
                                        ResourceAccess::Immutable | ResourceAccess::Mutable => {
                                            let ty = self
                                                .context
                                                .ptr_type(AddressSpace::default())
                                                .into();
                                            (
                                                name.clone(),
                                                ty,
                                                Some(crate::ast::Type::Pointer {
                                                    is_mutable: matches!(
                                                        access,
                                                        ResourceAccess::Mutable
                                                    ),
                                                    pointee: Box::new(resource_type.clone()),
                                                }),
                                            )
                                        }
                                        ResourceAccess::Owned => {
                                            let ty = self
                                                .map_ast_type(resource_type)
                                                .unwrap_or(self.context.i64_type().into());
                                            (name.clone(), ty, Some(resource_type.clone()))
                                        }
                                    },
                                    SystemParameter::Regular {
                                        param_type: _,
                                        name,
                                        value_type,
                                        ..
                                    } => {
                                        let ty = self
                                            .map_ast_type(&value_type)
                                            .unwrap_or(self.context.i64_type().into());
                                        (name.clone(), ty, Some(value_type.clone()))
                                    }
                                };

                                let alloca = self.create_entry_block_alloca(&p_name, p_ty)?;
                                self.builder
                                    .build_store(alloca, param)
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.variables.insert(p_name.clone(), (alloca, p_ty));
                                if let Some(ast_ty) = ast_ty_opt.clone() {
                                    self.record_unsigned_binding(&p_name, &ast_ty);
                                    self.local_types.insert(p_name.clone(), ast_ty);
                                    self.track_owned_binding(&p_name);
                                }
                            }
                        }

                        // Generate system function body
                        for stmt in &system_def.body {
                            self.generate_statement(stmt)?;
                        }

                        // Ensure function has return if needed
                        let current_block = self.builder.get_insert_block();
                        let needs_return =
                            current_block.map_or(true, |block| block.get_terminator().is_none());
                        if needs_return {
                            if let Some(ret_type) = &system_def.return_type {
                                if ret_type != &Type::None {
                                    // Build default return value
                                    let default_val: BasicValueEnum = match self
                                        .map_ast_type(ret_type)
                                    {
                                        Some(BasicTypeEnum::IntType(it)) => it.const_zero().into(),
                                        Some(BasicTypeEnum::FloatType(ft)) => {
                                            ft.const_zero().into()
                                        }
                                        Some(BasicTypeEnum::PointerType(pt)) => {
                                            pt.const_zero().into()
                                        }
                                        _ => self.context.i64_type().const_zero().into(),
                                    };
                                    self.try_build_return(Some(&default_val))?;
                                }
                            } else {
                                // No return type specified, return i64(0)
                                let ret_val = self.context.i64_type().const_zero();
                                self.try_build_return(Some(&ret_val))?;
                            }
                        }

                        // Restore context
                        self.current_function_return_ast = prev_ret_ast;
                        self.variables = prev_vars;
                        self.owned_locals = prev_owned;
                        self.local_types.clear();
                        self.unsigned_variables.clear();
                        self.current_function = prev_fn;
                    }
                }
                Statement::ImplBlock {
                    type_params: _,
                    trait_name,
                    type_name,
                    self_type,
                    methods,
                } => {
                    for m in methods {
                        let extracted = match m {
                            Statement::ConstDecl { name, value, .. } => {
                                if let ConstValue::Expression(Expression::Function {
                                    parameters,
                                    body,
                                    return_type,
                                    ..
                                }) = value
                                {
                                    Some((name.clone(), parameters, return_type, body))
                                } else {
                                    None
                                }
                            }
                            Statement::ImplMethod {
                                name,
                                parameters,
                                return_type,
                                body,
                                ..
                            } => Some((name.clone(), parameters, return_type, body)),
                            _ => None,
                        };

                        let Some((mname, parameters, return_type, body)) = extracted else {
                            continue;
                        };

                        let mangled = match trait_name {
                            Some(tn) => format!("{}_{}_{}", tn, type_name, mname),
                            None => format!("{}_{}", type_name, mname),
                        };
                        let f = match self.functions.get(&mangled) {
                            Some(f) => *f,
                            None => continue,
                        };
                        let entry = self.context.append_basic_block(f, "entry");
                        self.builder.position_at_end(entry);
                        let prev_fn = self.current_function;
                        let prev_impl_struct = self.current_impl_struct.clone();
                        let prev_vars = std::mem::take(&mut self.variables);
                        let prev_owned = std::mem::take(&mut self.owned_locals);
                        self.current_function = Some(f);
                        self.current_impl_struct = Some(type_name.clone());
                        let prev_ret_ast = self.current_function_return_ast.clone();
                        let ret_ast = return_type.clone().unwrap_or(Type::None);
                        self.current_function_return_ast = Some(ret_ast.clone());

                        let mut param_index = 0usize;
                        if trait_name.is_none() {
                            if let Some((_st, _)) = self.struct_types.get(type_name) {
                                if let Some(first_param) = f.get_param_iter().next() {
                                    let p_ty =
                                        self.context.ptr_type(AddressSpace::default()).into();
                                    let alloca = self.create_entry_block_alloca("self", p_ty)?;
                                    self.builder.build_store(alloca, first_param).map_err(|e| {
                                        CodegenError::CompilationError(e.to_string())
                                    })?;
                                    self.variables.insert("self".to_string(), (alloca, p_ty));
                                    self.local_types.insert(
                                        "self".to_string(),
                                        crate::ast::Type::Pointer {
                                            is_mutable: false,
                                            pointee: Box::new(self_type.clone()),
                                        },
                                    );
                                    self.track_owned_binding("self");
                                    param_index = 1;
                                }
                            }
                        }

                        for (i, param) in f.get_param_iter().enumerate().skip(param_index) {
                            let p_name = parameters
                                .get(i - param_index)
                                .map(|p| p.name.clone())
                                .unwrap_or(format!("arg{}", i - param_index));
                            let p_ty = parameters
                                .get(i - param_index)
                                .and_then(|p| p.param_type.as_ref())
                                .and_then(|t| self.map_ast_type(t))
                                .unwrap_or(self.context.i64_type().into());
                            let alloca = self.create_entry_block_alloca(&p_name, p_ty)?;
                            self.builder
                                .build_store(alloca, param)
                                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.variables.insert(p_name.clone(), (alloca, p_ty));
                            let ast_ty = parameters
                                .get(i - param_index)
                                .and_then(|p| p.param_type.clone())
                                .unwrap_or(crate::ast::Type::Identifier {
                                    name: "i64".to_string(),
                                    type_args: vec![],
                                });
                            self.record_unsigned_binding(&p_name, &ast_ty);
                            self.local_types.insert(p_name.clone(), ast_ty);
                            self.track_owned_binding(&p_name);
                        }

                        match body {
                            FunctionBody::Expression(expr) => {
                                let v = self.generate_expression(expr)?;
                                self.mark_expr_moved(expr);
                                self.drop_all_owned_locals()?;
                                let ret_ty = return_type
                                    .as_ref()
                                    .and_then(|t| self.map_ast_type(t))
                                    .unwrap_or(self.context.i64_type().into());
                                if mname == "get" {
                                    let ty_str = ret_ty.print_to_string().to_string();
                                    eprintln!(
                                        "impl method {}::{} return type {}",
                                        type_name, mname, ty_str
                                    );
                                }
                                let casted = self.cast_basic_to_type(v, ret_ty)?;
                                self.builder
                                    .build_return(Some(&casted))
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            }
                            FunctionBody::Block(stmts) => {
                                let mut last_expr_value: Option<BasicValueEnum<'ctx>> = None;
                                let slice: &[Statement] = &stmts[..];
                                if let Some((last, prefix)) = slice.split_last() {
                                    for s in prefix {
                                        let _ = self.generate_statement(s);
                                    }
                                    match last {
                                        Statement::Expression(expr) => {
                                            let v = self.generate_expression(expr)?;
                                            last_expr_value = Some(v);
                                            self.mark_expr_moved(expr);
                                        }
                                        other => {
                                            let _ = self.generate_statement(other);
                                        }
                                    }
                                }
                                let ret_ty = return_type
                                    .as_ref()
                                    .and_then(|t| self.map_ast_type(t))
                                    .unwrap_or(self.context.i64_type().into());
                                if mname == "get" {
                                    let ty_str = ret_ty.print_to_string().to_string();
                                    eprintln!(
                                        "impl method {}::{} return type {}",
                                        type_name, mname, ty_str
                                    );
                                }
                                let ret_val: BasicValueEnum<'ctx> = if let Some(v) = last_expr_value
                                {
                                    self.cast_basic_to_type(v, ret_ty)?
                                } else {
                                    match ret_ty {
                                        BasicTypeEnum::IntType(it) => it.const_zero().into(),
                                        BasicTypeEnum::FloatType(ft) => ft.const_zero().into(),
                                        BasicTypeEnum::PointerType(pt) => pt.const_zero().into(),
                                        _ => self.context.i64_type().const_zero().into(),
                                    }
                                };
                                self.drop_all_owned_locals()?;
                                self.builder
                                    .build_return(Some(&ret_val))
                                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            }
                        }

                        self.current_function_return_ast = prev_ret_ast;
                        self.variables = prev_vars;
                        self.owned_locals = prev_owned;
                        self.local_types.clear();
                        self.unsigned_variables.clear();
                        self.current_function = prev_fn;
                        self.current_impl_struct = prev_impl_struct;
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }
}
