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
    // Track vector lengths for variables bound to 1D matrix literals
    vector_lengths: HashMap<String, u64>,
    // Track matrix rank (1 for 1D vector, 2+ for multi-dim) per variable name
    matrix_rank: HashMap<String, usize>,
    // When true, emit a freestanding runtime entry (_start) and do not emit a C main().
    runtime_mode: bool,
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
            vector_lengths: HashMap::new(),
            matrix_rank: HashMap::new(),
            runtime_mode: false,
        };
        
    generator.declare_external_functions()?;
    generator.build_struct_types()?;
        
        Ok(generator)
    }

    /// Enable or disable freestanding runtime mode. When enabled, the codegen will:
    /// - Declare user `main` function as `peano_main` (not emit a C `main`)
    /// - Emit a `_start` symbol that calls `peano_main` and then `exit(code)`
    pub fn enable_runtime_mode(&mut self, enabled: bool) {
        self.runtime_mode = enabled;
    }

    // Build LLVM struct types for all user-declared struct types in semantics
    fn build_struct_types(&mut self) -> Result<(), CodegenError> {
        use crate::ast::Type as AstType;
        // Iterate semantic types and create LLVM struct definitions
        for (name, ty) in &self.semantic.types {
            if let AstType::Struct { fields } = ty {
                // Determine a stable field order (sorted by field name) so we can map names -> indices
                let mut order: Vec<String> = fields.keys().cloned().collect();
                order.sort();

                // Create or fetch an opaque struct with this name, then set its body
                let st = self.context.opaque_struct_type(name);
                let mut llvm_fields: Vec<BasicTypeEnum<'ctx>> = Vec::with_capacity(order.len());
                for fname in &order {
                    if let Some(fty) = fields.get(fname) {
                        // Map field type to an LLVM type; default to i64 when unknown
                        let bte = self.map_ast_type(fty).unwrap_or(self.context.i64_type().into());
                        llvm_fields.push(bte);
                    } else {
                        llvm_fields.push(self.context.i64_type().into());
                    }
                }
                st.set_body(&llvm_fields, false);
                self.struct_types.insert(name.clone(), (st, order));
            }
        }
        Ok(())
    }

    // Try to map an AST type to a concrete LLVM BasicTypeEnum; fallback to i64
    fn map_ast_type(&self, t: &crate::ast::Type) -> Option<BasicTypeEnum<'ctx>> {
        use crate::ast::Type as AstType;
        match t {
            AstType::Identifier(name) => match name.as_str() {
                "i32" => Some(self.context.i32_type().into()),
                "i64" => Some(self.context.i64_type().into()),
                "f32" => Some(self.context.f32_type().into()),
                "f64" => Some(self.context.f64_type().into()),
                "bool" => Some(self.context.bool_type().into()),
                "string" | "String" | "str" => Some(self.context.ptr_type(AddressSpace::default()).into()),
                _ => None,
            },
            AstType::Pointer { .. } | AstType::RawPointer { .. } => Some(self.context.ptr_type(AddressSpace::default()).into()),
            AstType::Optional { .. } | AstType::Result { .. } => Some(self.context.i64_type().into()),
            AstType::Struct { .. } | AstType::Enum { .. } | AstType::Trait { .. } | AstType::Function { .. } => None,
            AstType::Matrix { .. } => Some(self.context.ptr_type(AddressSpace::default()).into()),
            AstType::None => Some(self.context.i64_type().into()),
        }
    }

    // Cast a BasicValueEnum to another BasicTypeEnum best-effort
    fn cast_basic_to_type(&self, v: BasicValueEnum<'ctx>, target: BasicTypeEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        Ok(match target {
            BasicTypeEnum::IntType(it) => self.cast_to_int(v, it)?.into(),
            BasicTypeEnum::FloatType(ft) => self.cast_to_float(v, ft)?.into(),
            BasicTypeEnum::PointerType(pt) => self.cast_to_ptr(v, pt)?.into(),
            _ => v,
        })
    }

    // Unify two integers to a common width (use i64) and return both cast plus chosen type
    fn unify_ints(&self, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>) -> Result<(inkwell::values::IntValue<'ctx>, inkwell::values::IntValue<'ctx>, IntType<'ctx>), CodegenError> {
        let ty = self.context.i64_type();
        let li = self.cast_to_int(l, ty)?;
        let ri = self.cast_to_int(r, ty)?;
        Ok((li, ri, ty))
    }

    #[allow(dead_code)]
    fn get_int_type_of(&self, v: BasicValueEnum<'ctx>) -> Option<IntType<'ctx>> {
        if v.is_int_value() { Some(v.into_int_value().get_type()) } else { None }
    }

    // Resolve the semantic struct name for a variable (peels pointers/optionals/results)
    fn semantic_struct_name_of_var(&self, var_name: &str) -> Option<String> {
        use crate::ast::Type as AstType;
        // First prefer semantic types (covers globals and some params)
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
            if let AstType::Identifier(name) = peel(t) { return Some(name.clone()); }
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

    fn cast_to_int(&self, value: BasicValueEnum<'ctx>, int_ty: IntType<'ctx>) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        if value.is_int_value() {
            let iv = value.into_int_value();
            if iv.get_type() == int_ty {
                Ok(iv)
            } else {
                self.builder.build_int_cast(iv, int_ty, "icast").map_err(|e| CodegenError::CompilationError(e.to_string()))
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

    // Basic allocator hooks via libc malloc/free
    let malloc_ty = i8_ptr_type.fn_type(&[self.context.i64_type().into()], false);
    let malloc_fn = self.module.add_function("malloc", malloc_ty, None);
    // Expose as `alloc` to peano source
    self.functions.insert("alloc".to_string(), malloc_fn);

    let free_ty = self.context.void_type().fn_type(&[i8_ptr_type.into()], false);
    let free_fn = self.module.add_function("free", free_ty, None);
    self.functions.insert("dealloc".to_string(), free_fn);

    // Process exit (libc)
    let exit_ty = self.context.void_type().fn_type(&[i32_type.into()], false);
    let exit_fn = self.module.add_function("exit", exit_ty, None);
    self.functions.insert("exit".to_string(), exit_fn);

    // strlen for string length (bytes); expose as `len`
    let strlen_ty = self.context.i64_type().fn_type(&[i8_ptr_type.into()], false);
    let strlen_fn = self.module.add_function("strlen", strlen_ty, None);
    self.functions.insert("len".to_string(), strlen_fn);

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
    let call = self.builder
        .build_call(strcmp_fn, &[a.into(), b.into()], "strcmp_call")
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
    let rv = call
        .try_as_basic_value()
        .left()
        .ok_or_else(|| CodegenError::CompilationError("strcmp returned void?".to_string()))?
        .into_int_value();
    let eqz = self.builder
        .build_int_compare(IntPredicate::EQ, rv, i32_type.const_zero(), "eqz")
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
    self.builder
        .build_return(Some(&eqz))
        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
    if let Some(bb) = prev_bb { self.builder.position_at_end(bb); }
    self.functions.insert("streq".to_string(), streq_fn);
        
        Ok(())
    }
    
    pub fn generate_program(&mut self, program: &Program) -> Result<(), CodegenError> {
        // First declare and define user functions from const decls
        self.declare_and_define_functions(program)?;

        if self.runtime_mode {
            // Freestanding: emit _start that calls peano_main (if present) then exit
            let start_ty = self.context.void_type().fn_type(&[], false);
            let start_fn = self.module.add_function("_start", start_ty, None);
            let entry = self.context.append_basic_block(start_fn, "entry");
            self.builder.position_at_end(entry);
            self.current_function = Some(start_fn);

            // Call peano_main if it exists
            let exit_fn = *self
                .functions
                .get("exit")
                .ok_or_else(|| CodegenError::CompilationError("exit not declared".to_string()))?;

            let code_i32 = if let Some(main_fn) = self.functions.get("peano_main").cloned() {
                let call = self
                    .builder
                    .build_call(main_fn, &[], "call_peano_main")
                    .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                // If main returns a value, cast to i32; else 0
                if let Some(bv) = call.try_as_basic_value().left() {
                    let iv = if bv.is_int_value() {
                        bv.into_int_value()
                    } else if bv.is_float_value() {
                        self.builder
                            .build_float_to_signed_int(bv.into_float_value(), self.context.i32_type(), "retf2i")
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
                // No peano_main: run top-level in a minimal block like legacy main()
                let code_alloca = self
                    .create_entry_block_alloca("retcode", self.context.i32_type().into())?;
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

        // Hosted mode: synthesize a C main() and run top-level main body
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
        self.builder
            .build_return(Some(&zero))
            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;

        Ok(())
    }
    
    fn generate_statement(&mut self, statement: &Statement) -> Result<(), CodegenError> {
        
        match statement {
            Statement::VariableDecl { name, type_annotation, value } => {
                // Special-case: function.bind(...) assigned to a name -> synthesize a wrapper function with that name
                if let Expression::Call { function, arguments: bind_args } = value {
                    if let Expression::FieldAccess { object, field } = function.as_ref() {
                        if field == "bind" {
                            // Only support binding on a known function identifier, and bind the last K params
                            if let Expression::Identifier(base_fn_name) = object.as_ref() {
                                if let Some(base_fn) = self.functions.get(base_fn_name).cloned() {
                                    // Base function param metadata
                                    let base_param_metas = base_fn.get_type().get_param_types();
                                    let total_params = base_param_metas.len();
                                    let bound_n = bind_args.len();
                                    let unbound_n = if total_params >= bound_n { total_params - bound_n } else { 0 };
                                    // Wrapper returns i64 (best-effort) and takes the first unbound_n params
                                    let wrapper_param_meta: Vec<inkwell::types::BasicMetadataTypeEnum> = base_param_metas
                                        .iter()
                                        .take(unbound_n)
                                        .cloned()
                                        .collect();
                                    let wrapper_ty = self.context.i64_type().fn_type(&wrapper_param_meta, false);
                                    let wrapper_fn = self.module.add_function(name.as_str(), wrapper_ty, None);
                                    self.functions.insert(name.clone(), wrapper_fn);

                                    // Define the wrapper body now
                                    // Save current insertion point to restore later
                                    let prev_insert_block = self.builder.get_insert_block();
                                    let entry = self.context.append_basic_block(wrapper_fn, "entry");
                                    let prev_fn = self.current_function;
                                    let prev_vars = std::mem::take(&mut self.variables);
                                    self.current_function = Some(wrapper_fn);
                                    self.builder.position_at_end(entry);

                                    // Bind wrapper params to allocas for reuse/casts
                                    for (i, param) in wrapper_fn.get_param_iter().enumerate() {
                                        let p_ty: BasicTypeEnum = match base_param_metas.get(i) {
                                            Some(inkwell::types::BasicMetadataTypeEnum::IntType(it)) => (*it).into(),
                                            Some(inkwell::types::BasicMetadataTypeEnum::FloatType(ft)) => (*ft).into(),
                                            Some(inkwell::types::BasicMetadataTypeEnum::PointerType(pt)) => (*pt).into(),
                                            Some(inkwell::types::BasicMetadataTypeEnum::StructType(st)) => (*st).into(),
                                            _ => self.context.i64_type().into(),
                                        };
                                        let alloca = self.create_entry_block_alloca(&format!("arg{}", i), p_ty)?;
                                        self.builder.build_store(alloca, param).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                        self.variables.insert(format!("arg{}", i), (alloca, p_ty));
                                    }

                                    // Build call args to base: first unbound wrapper params, then bound values
                                    let mut call_args: Vec<BasicMetadataValueEnum> = Vec::new();
                                    for i in 0..unbound_n {
                                        let p_meta = base_param_metas[i];
                                        // Load wrapper arg i
                                        let (alloca, p_ty) = self.variables.get(&format!("arg{}", i)).unwrap().clone();
                                        let loaded = self.builder.build_load(p_ty, alloca, &format!("arg{}", i)).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                        let casted: BasicMetadataValueEnum = match p_meta {
                                            inkwell::types::BasicMetadataTypeEnum::IntType(it) => self.cast_to_int(loaded, it)?.into(),
                                            inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => self.cast_to_float(loaded, ft)?.into(),
                                            inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => self.cast_to_ptr(loaded, pt)?.into(),
                                            inkwell::types::BasicMetadataTypeEnum::StructType(st) => {
                                                let bte: BasicTypeEnum = st.into();
                                                let val = self.cast_basic_to_type(loaded, bte)?;
                                                val.into()
                                            }
                                            _ => loaded.into(),
                                        };
                                        call_args.push(casted);
                                    }
                                    for (j, barg) in bind_args.iter().enumerate() {
                                        let base_index = unbound_n + j;
                                        if let Some(p_meta) = base_param_metas.get(base_index) {
                                            let v = self.generate_expression(&barg.value)?;
                                            let casted: BasicMetadataValueEnum = match p_meta {
                                                inkwell::types::BasicMetadataTypeEnum::IntType(it) => self.cast_to_int(v, *it)?.into(),
                                                inkwell::types::BasicMetadataTypeEnum::FloatType(ft) => self.cast_to_float(v, *ft)?.into(),
                                                inkwell::types::BasicMetadataTypeEnum::PointerType(pt) => self.cast_to_ptr(v, *pt)?.into(),
                                                inkwell::types::BasicMetadataTypeEnum::StructType(st) => {
                                                    let bte: BasicTypeEnum = (*st).into();
                                                    let val = self.cast_basic_to_type(v, bte)?;
                                                    val.into()
                                                }
                                                _ => self.cast_to_int(v, self.context.i64_type())?.into(),
                                            };
                                            call_args.push(casted);
                                        }
                                    }

                                    let call_res = self.builder.build_call(base_fn, &call_args, "calltmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let ret_val: BasicValueEnum<'ctx> = match call_res.try_as_basic_value().left() {
                                        Some(bv) => {
                                            let iv = self.cast_to_int(bv, self.context.i64_type())?;
                                            iv.into()
                                        }
                                        None => self.context.i64_type().const_zero().into(),
                                    };
                                    self.builder.build_return(Some(&ret_val)).map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                    // Restore state and finish var-decl handling for this case without creating a local variable
                                    self.variables = prev_vars;
                                    self.current_function = prev_fn;
                                    // Restore builder insertion point
                                    if let Some(bb) = prev_insert_block { self.builder.position_at_end(bb); }
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
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
                // Track vector length and rank if RHS is a matrix literal assigned to a variable
                if let Expression::Matrix { rows } = value {
                    let rank = if rows.len() <= 1 { 1 } else { 2 };
                    self.matrix_rank.insert(name.clone(), rank);
                    if rank == 1 {
                        let len = rows.first().map(|r| r.len()).unwrap_or(0) as u64;
                        self.vector_lengths.insert(name.clone(), len);
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
                // Support: for i in N and for i in start:end[:step]; also iterate elements for 1D matrices (vectors)
                // Try vector element iteration first
                if let Expression::Matrix { rows } = iterable {
                    // Inline literal vector: get pointer and length from literal
                    let base = self.generate_expression(iterable)?;
                    if base.is_pointer_value() {
                        let len = if rows.len() <= 1 { rows.first().map(|r| r.len()).unwrap_or(0) } else { rows.len() } as u64;
                        // idx and loop var allocas
                        let idx_allo = self.create_entry_block_alloca("idx", self.context.i64_type().into())?;
                        self.builder.build_store(idx_allo, self.context.i64_type().const_zero()).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let elem_allo = self.create_entry_block_alloca(variable, self.context.i64_type().into())?;
                        let prev = self.variables.insert(variable.clone(), (elem_allo, self.context.i64_type().into()));

                        let current_fn = self.current_function.ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
                        let cond_bb = self.context.append_basic_block(current_fn, "for.cond");
                        let body_bb = self.context.append_basic_block(current_fn, "for.body");
                        let inc_bb = self.context.append_basic_block(current_fn, "for.inc");
                        let end_bb = self.context.append_basic_block(current_fn, "for.end");

                        self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        // cond
                        self.builder.position_at_end(cond_bb);
                        let i64_bte: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                        let idx_cur = self.builder.build_load(i64_bte, idx_allo, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                        let endc = self.context.i64_type().const_int(len, false);
                        let cmp = self.builder.build_int_compare(IntPredicate::SLT, idx_cur, endc, "forcmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder.build_conditional_branch(cmp, body_bb, end_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        // body
                        self.builder.position_at_end(body_bb);
                        let elem_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i64_type(), base.into_pointer_value(), &[idx_cur], "idx") }.map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let loaded = self.builder.build_load(self.context.i64_type(), elem_ptr, "elem").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder.build_store(elem_allo, loaded).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        for s in body { let _ = self.generate_statement(s); }
                        if body_bb.get_terminator().is_none() { self.builder.build_unconditional_branch(inc_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?; }
                        // inc
                        self.builder.position_at_end(inc_bb);
                        let i64_bte2: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                        let idx_cur2 = self.builder.build_load(i64_bte2, idx_allo, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                        let next = self.builder.build_int_add(idx_cur2, self.context.i64_type().const_int(1, false), "inc").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder.build_store(idx_allo, next).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        // end
                        self.builder.position_at_end(end_bb);
                        if let Some(prev_binding) = prev { self.variables.insert(variable.clone(), prev_binding); } else { self.variables.remove(variable); }
                        return Ok(());
                    }
                } else if let Expression::Identifier(name) = iterable {
                    // Variable that may be a vector: use semantic type to get length and load base pointer
                    if let Some(Type::Matrix { element_type: _, dimensions }) = self.semantic.get_variable_type(name) {
                        let len = if dimensions.is_empty() { 0 } else { dimensions.iter().product::<usize>() } as u64;
                        if len > 0 {
                            let base_val = self.generate_expression(iterable)?;
                            if base_val.is_pointer_value() {
                                let base = base_val.into_pointer_value();
                                let idx_allo = self.create_entry_block_alloca("idx", self.context.i64_type().into())?;
                                self.builder.build_store(idx_allo, self.context.i64_type().const_zero()).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let elem_allo = self.create_entry_block_alloca(variable, self.context.i64_type().into())?;
                                let prev = self.variables.insert(variable.clone(), (elem_allo, self.context.i64_type().into()));

                                let current_fn = self.current_function.ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
                                let cond_bb = self.context.append_basic_block(current_fn, "for.cond");
                                let body_bb = self.context.append_basic_block(current_fn, "for.body");
                                let inc_bb = self.context.append_basic_block(current_fn, "for.inc");
                                let end_bb = self.context.append_basic_block(current_fn, "for.end");

                                self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // cond
                                self.builder.position_at_end(cond_bb);
                                let i64_bte3: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                let idx_cur = self.builder.build_load(i64_bte3, idx_allo, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                let endc = self.context.i64_type().const_int(len, false);
                                let cmp = self.builder.build_int_compare(IntPredicate::SLT, idx_cur, endc, "forcmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder.build_conditional_branch(cmp, body_bb, end_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // body
                                self.builder.position_at_end(body_bb);
                                let elem_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i64_type(), base, &[idx_cur], "idx") }.map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let loaded = self.builder.build_load(self.context.i64_type(), elem_ptr, "elem").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder.build_store(elem_allo, loaded).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                for s in body { let _ = self.generate_statement(s); }
                                if body_bb.get_terminator().is_none() { self.builder.build_unconditional_branch(inc_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?; }
                                // inc
                                self.builder.position_at_end(inc_bb);
                                let i64_bte4: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                let idx_cur2 = self.builder.build_load(i64_bte4, idx_allo, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                let next = self.builder.build_int_add(idx_cur2, self.context.i64_type().const_int(1, false), "inc").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder.build_store(idx_allo, next).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // end
                                self.builder.position_at_end(end_bb);
                                if let Some(prev_binding) = prev { self.variables.insert(variable.clone(), prev_binding); } else { self.variables.remove(variable); }
                                return Ok(());
                            }
                        }
                    } else if let Some(len) = self.vector_lengths.get(name).cloned() {
                        if len > 0 {
                            let base_val = self.generate_expression(iterable)?;
                            if base_val.is_pointer_value() {
                                let base = base_val.into_pointer_value();
                                let idx_allo = self.create_entry_block_alloca("idx", self.context.i64_type().into())?;
                                self.builder.build_store(idx_allo, self.context.i64_type().const_zero()).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let elem_allo = self.create_entry_block_alloca(variable, self.context.i64_type().into())?;
                                let prev = self.variables.insert(variable.clone(), (elem_allo, self.context.i64_type().into()));

                                let current_fn = self.current_function.ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
                                let cond_bb = self.context.append_basic_block(current_fn, "for.cond");
                                let body_bb = self.context.append_basic_block(current_fn, "for.body");
                                let inc_bb = self.context.append_basic_block(current_fn, "for.inc");
                                let end_bb = self.context.append_basic_block(current_fn, "for.end");

                                self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // cond
                                self.builder.position_at_end(cond_bb);
                                let i64_bte3: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                let idx_cur = self.builder.build_load(i64_bte3, idx_allo, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                let endc = self.context.i64_type().const_int(len, false);
                                let cmp = self.builder.build_int_compare(IntPredicate::SLT, idx_cur, endc, "forcmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder.build_conditional_branch(cmp, body_bb, end_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // body
                                self.builder.position_at_end(body_bb);
                                let elem_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i64_type(), base, &[idx_cur], "idx") }.map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let loaded = self.builder.build_load(self.context.i64_type(), elem_ptr, "elem").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder.build_store(elem_allo, loaded).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                for s in body { let _ = self.generate_statement(s); }
                                if body_bb.get_terminator().is_none() { self.builder.build_unconditional_branch(inc_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?; }
                                // inc
                                self.builder.position_at_end(inc_bb);
                                let i64_bte4: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                let idx_cur2 = self.builder.build_load(i64_bte4, idx_allo, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                let next = self.builder.build_int_add(idx_cur2, self.context.i64_type().const_int(1, false), "inc").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder.build_store(idx_allo, next).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                // end
                                self.builder.position_at_end(end_bb);
                                if let Some(prev_binding) = prev { self.variables.insert(variable.clone(), prev_binding); } else { self.variables.remove(variable); }
                                return Ok(());
                            }
                        }
                    }
                }

                // Fallback: index-based numeric or range loop
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
                    Expression::Matrix { rows } => {
                        // iterate over a vector: treat as 0..len
                        let len = if rows.len() <= 1 { rows.first().map(|r| r.len()).unwrap_or(0) } else { rows.len() } as u64;
                        (Some(self.context.i64_type().const_zero()), Some(self.context.i64_type().const_int(len, false)), Some(self.context.i64_type().const_int(1, false)))
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
                        // exclusive: i > end
                        self.builder.build_int_compare(IntPredicate::SGT, cur_i, end_val, "forcmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?
                    } else if step_val.is_const() {
                        // exclusive: i < end
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
            Statement::ModuleDecl { name: _, items } => {
                if let Some(stmts) = items {
                    for s in stmts { let _ = self.generate_statement(s); }
                }
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
            Expression::Matrix { rows } => {
                // Only support 1D vectors concretely; for multi-dim, return a null i64* placeholder to avoid invalid IR.
                let i64_ptr_ty = self.context.ptr_type(AddressSpace::default());
                if rows.len() > 1 {
                    // Multi-dimensional literal not yet lowered: return null pointer (printing uses placeholder path)
                    return Ok(i64_ptr_ty.const_null().into());
                }
                let empty: [Expression; 0] = [];
                let row0: &[Expression] = rows.get(0).map(|v| v.as_slice()).unwrap_or(&empty);
                let n = row0.len();
                if n == 0 {
                    // Empty vector literal -> null pointer; consumers must use tracked length 0 and avoid deref
                    return Ok(i64_ptr_ty.const_null().into());
                }
                let arr_ty = self.context.i64_type().array_type(n as u32);
                let alloca = self.create_entry_block_alloca("vec", arr_ty.into())?;
                // initialize each element
                for (i, expr) in row0.iter().enumerate() {
                    let v = self.generate_expression(expr)?;
                    let iv = self.cast_to_int(v, self.context.i64_type())?;
                    let elem_ptr = unsafe { self.builder.build_gep(arr_ty, alloca, &[self.context.i32_type().const_zero(), self.context.i32_type().const_int(i as u64, false)], "elemp") }.map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    self.builder.build_store(elem_ptr, iv).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                }
                // decay to pointer-to-first element
                let first_elem_ptr = unsafe { self.builder.build_gep(arr_ty, alloca, &[self.context.i32_type().const_zero(), self.context.i32_type().const_zero()], "firstp") }.map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                Ok(first_elem_ptr.into())
            }
            Expression::Index { object, indices } => {
                // Only support object being a 1D vector pointer and a single index
                let base = self.generate_expression(object)?;
                if base.is_pointer_value() {
                    let idx_val = if let Some(ix) = indices.get(0) { self.generate_expression(ix)? } else { self.context.i64_type().const_zero().into() };
                    let idx_i64 = self.cast_to_int(idx_val, self.context.i64_type())?;
                    let elem_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i64_type(), base.into_pointer_value(), &[idx_i64], "idx") }.map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let loaded = self.builder.build_load(self.context.i64_type(), elem_ptr, "loadidx").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(loaded)
                } else {
                    Ok(self.context.i64_type().const_zero().into())
                }
            }
            
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

            UnaryOperator::BitwiseNot => {
                if operand.is_int_value() {
                    let result = self.builder.build_not(operand.into_int_value(), "bnot")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(result.into())
                } else {
                    Err(CodegenError::InvalidOperation("Invalid type for bitwise not".to_string()))
                }
            }

            UnaryOperator::Deref => {
                if operand.is_pointer_value() {
                    // Best-effort: load as i64 by default; many consumers will cast/widen as needed.
                    let i64_ty = self.context.i64_type();
                    let bte: BasicTypeEnum<'ctx> = i64_ty.into();
                    let loaded = self.builder
                        .build_load(bte, operand.into_pointer_value(), "deref")
                        .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    Ok(loaded)
                } else {
                    // Graceful fallback: return 0 instead of failing
                    Ok(self.context.i64_type().const_zero().into())
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
                                if let Some((ptr, ty)) = self.variables.get(var) {
                                    // If the variable itself holds a pointer (e.g., string), load and pass the pointer value.
                                    if ty.is_pointer_type() {
                                        let loaded = self
                                            .builder
                                            .build_load(*ty, *ptr, "loadptrarg")
                                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                        let casted = self
                                            .builder
                                            .build_pointer_cast(loaded.into_pointer_value(), *expect_pt, "ptrarg")
                                            .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                        arg_values.push(casted.into());
                                        continue;
                                    } else {
                                        // Otherwise pass the address-of the alloca as a pointer parameter.
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
                    // no-op here; real reduce implementation below handles both semantic and tracked lengths
                    // Special-case: 1D vector methods like reduce over arrays
                    if let Some(Type::Matrix { element_type: _, dimensions }) = self.semantic.get_variable_type(var_name) {
                        if field == "reduce" {
                            // Expect a single closure argument: (acc, x) => expr
                            // Compute length from semantic dims (product)
                            let len = if dimensions.is_empty() { 0 } else { dimensions.iter().product::<usize>() } as u64;
                            if len == 0 { return Ok(self.context.i64_type().const_zero().into()); }

                            // Evaluate base pointer to the first element
                            let base_val = self.generate_expression(object)?;
                            if !base_val.is_pointer_value() { return Ok(self.context.i64_type().const_zero().into()); }
                            let base_ptr = base_val.into_pointer_value();

                            // Validate argument shape
                            if let Some(first_arg) = arguments.get(0) {
                                if let Expression::Function { parameters, body, .. } = &first_arg.value {
                                    // Expect 2 params (acc, x)
                                    let (p_acc_name, p_x_name) = if parameters.len() >= 2 {
                                        (parameters[0].name.clone(), parameters[1].name.clone())
                                    } else {
                                        ("acc".to_string(), "x".to_string())
                                    };
                                    // Create accumulator alloca initialized to 0
                                    let i64_bte: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                    let acc_alloca = self.create_entry_block_alloca(&p_acc_name, i64_bte)?;
                                    self.builder.build_store(acc_alloca, self.context.i64_type().const_zero()).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // Bind acc into variables table (save previous)
                                    let prev_acc = self.variables.insert(p_acc_name.clone(), (acc_alloca, i64_bte));

                                    // Also prepare a temporary alloca for the element (x)
                                    let x_alloca = self.create_entry_block_alloca(&p_x_name, i64_bte)?;
                                    let prev_x = self.variables.insert(p_x_name.clone(), (x_alloca, i64_bte));

                                    // Build loop blocks
                                    let current_fn = self.current_function.ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
                                    let idx_alloca = self.create_entry_block_alloca("idx", i64_bte)?;
                                    self.builder.build_store(idx_alloca, self.context.i64_type().const_zero()).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let cond_bb = self.context.append_basic_block(current_fn, "reduce.cond");
                                    let body_bb = self.context.append_basic_block(current_fn, "reduce.body");
                                    let inc_bb = self.context.append_basic_block(current_fn, "reduce.inc");
                                    let end_bb = self.context.append_basic_block(current_fn, "reduce.end");

                                    self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // cond
                                    self.builder.position_at_end(cond_bb);
                                    let idx_cur = self.builder.build_load(i64_bte, idx_alloca, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                    let endc = self.context.i64_type().const_int(len, false);
                                    let cmp = self.builder.build_int_compare(IntPredicate::SLT, idx_cur, endc, "rcmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_conditional_branch(cmp, body_bb, end_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                    // body: load element into x, evaluate closure body, store back to acc
                                    self.builder.position_at_end(body_bb);
                                    let elem_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i64_type(), base_ptr, &[idx_cur], "ridx") }.map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let elem_val = self.builder.build_load(self.context.i64_type(), elem_ptr, "elem").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_store(x_alloca, elem_val).map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                    // Evaluate closure body with current bindings
                                    let step_val: BasicValueEnum<'ctx> = match body {
                                        FunctionBody::Expression(expr) => {
                                            let v = self.generate_expression(expr)?;
                                            self.cast_to_int(v, self.context.i64_type())?.into()
                                        }
                                        FunctionBody::Block(stmts) => {
                                            // Execute statements; if last is an expression, take its value
                                            let mut last_expr_value: Option<BasicValueEnum<'ctx>> = None;
                                            let slice: &[Statement] = &stmts[..];
                                            if let Some((last, prefix)) = slice.split_last() {
                                                for s in prefix { let _ = self.generate_statement(s); }
                                                if let Statement::Expression(expr) = last {
                                                    let v = self.generate_expression(expr)?;
                                                    last_expr_value = Some(v);
                                                }
                                            }
                                            if let Some(v) = last_expr_value { self.cast_to_int(v, self.context.i64_type())?.into() } else { self.context.i64_type().const_zero().into() }
                                        }
                                    };
                                    // Store new acc value
                                    self.builder.build_store(acc_alloca, step_val).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    if body_bb.get_terminator().is_none() { self.builder.build_unconditional_branch(inc_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?; }

                                    // inc
                                    self.builder.position_at_end(inc_bb);
                                    let idx_cur2 = self.builder.build_load(i64_bte, idx_alloca, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                    let next = self.builder.build_int_add(idx_cur2, self.context.i64_type().const_int(1, false), "inc").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_store(idx_alloca, next).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                    // end
                                    self.builder.position_at_end(end_bb);
                                    // Restore previous bindings
                                    if let Some(prev) = prev_x { self.variables.insert(p_x_name.clone(), prev); } else { self.variables.remove(&p_x_name); }
                                    if let Some(prev) = prev_acc { self.variables.insert(p_acc_name.clone(), prev); } else { self.variables.remove(&p_acc_name); }
                                    // Load final accumulator and return it
                                    let final_acc = self.builder.build_load(i64_bte, acc_alloca, "acc").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
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
                                        if let Expression::Function { parameters, body, .. } = &first_arg.value {
                                            let (p_acc_name, p_x_name) = if parameters.len() >= 2 { (parameters[0].name.clone(), parameters[1].name.clone()) } else { ("acc".to_string(), "x".to_string()) };
                                            let i64_bte: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                            let acc_alloca = self.create_entry_block_alloca(&p_acc_name, i64_bte)?;
                                            self.builder.build_store(acc_alloca, self.context.i64_type().const_zero()).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            let prev_acc = self.variables.insert(p_acc_name.clone(), (acc_alloca, i64_bte));
                                            let x_alloca = self.create_entry_block_alloca(&p_x_name, i64_bte)?;
                                            let prev_x = self.variables.insert(p_x_name.clone(), (x_alloca, i64_bte));

                                            let current_fn = self.current_function.ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
                                            let idx_alloca = self.create_entry_block_alloca("idx", i64_bte)?;
                                            self.builder.build_store(idx_alloca, self.context.i64_type().const_zero()).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            let cond_bb = self.context.append_basic_block(current_fn, "reduce.cond");
                                            let body_bb = self.context.append_basic_block(current_fn, "reduce.body");
                                            let inc_bb = self.context.append_basic_block(current_fn, "reduce.inc");
                                            let end_bb = self.context.append_basic_block(current_fn, "reduce.end");

                                            self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            self.builder.position_at_end(cond_bb);
                                            let idx_cur = self.builder.build_load(i64_bte, idx_alloca, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                            let endc = self.context.i64_type().const_int(len, false);
                                            let cmp = self.builder.build_int_compare(IntPredicate::SLT, idx_cur, endc, "rcmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            self.builder.build_conditional_branch(cmp, body_bb, end_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                            self.builder.position_at_end(body_bb);
                                            let elem_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i64_type(), base_ptr, &[idx_cur], "ridx") }.map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            let elem_val = self.builder.build_load(self.context.i64_type(), elem_ptr, "elem").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            self.builder.build_store(x_alloca, elem_val).map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                            let step_val: BasicValueEnum<'ctx> = match body {
                                                FunctionBody::Expression(expr) => { let v = self.generate_expression(expr)?; self.cast_to_int(v, self.context.i64_type())?.into() }
                                                FunctionBody::Block(stmts) => {
                                                    let mut last_expr_value: Option<BasicValueEnum<'ctx>> = None;
                                                    let slice: &[Statement] = &stmts[..];
                                                    if let Some((last, prefix)) = slice.split_last() { for s in prefix { let _ = self.generate_statement(s); } if let Statement::Expression(expr) = last { let v = self.generate_expression(expr)?; last_expr_value = Some(v); } }
                                                    if let Some(v) = last_expr_value { self.cast_to_int(v, self.context.i64_type())?.into() } else { self.context.i64_type().const_zero().into() }
                                                }
                                            };
                                            self.builder.build_store(acc_alloca, step_val).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            if body_bb.get_terminator().is_none() { self.builder.build_unconditional_branch(inc_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?; }

                                            self.builder.position_at_end(inc_bb);
                                            let idx_cur2 = self.builder.build_load(i64_bte, idx_alloca, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                            let next = self.builder.build_int_add(idx_cur2, self.context.i64_type().const_int(1, false), "inc").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            self.builder.build_store(idx_alloca, next).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                            self.builder.position_at_end(end_bb);
                                            if let Some(prev) = prev_x { self.variables.insert(p_x_name.clone(), prev); } else { self.variables.remove(&p_x_name); }
                                            if let Some(prev) = prev_acc { self.variables.insert(p_acc_name.clone(), prev); } else { self.variables.remove(&p_acc_name); }
                                            let final_acc = self.builder.build_load(i64_bte, acc_alloca, "acc").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            return Ok(final_acc);
                                        }
                                    }
                                }
                            }
                        }
                    }
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
            
            let printf_fn = *self.functions.get("printf").unwrap();
            let args = vec![newline_str.as_pointer_value().into()];
            self.builder.build_call(printf_fn, &args, "printf_call")
                .map_err(|e| CodegenError::CompilationError(e.to_string()))?;
    } else {
            // For each argument, print it. Special-case: arr.filter(closure) -> print elements like [..]
            let printf_fn = *self.functions.get("printf").unwrap();
            for (i, arg) in arguments.iter().enumerate() {
                // Try special-case pattern match on arg expression
                let mut handled_special = match &arg.value {
                    Expression::Call { function, arguments: hof_args } => {
                        if let Expression::FieldAccess { object, field } = function.as_ref() {
                            if field == "map" {
                                // Determine length of vector from semantics or tracked lengths
                                let mut len_opt: Option<u64> = None;
                                if let Expression::Identifier(var_name) = object.as_ref() {
                                    if let Some(Type::Matrix { element_type: _, dimensions }) = self.semantic.get_variable_type(var_name) {
                                        let l = if dimensions.is_empty() { 0 } else { dimensions.iter().product::<usize>() } as u64;
                                        if l > 0 { len_opt = Some(l); }
                                    } else if let Some(l) = self.vector_lengths.get(var_name).cloned() {
                                        len_opt = Some(l);
                                    }
                                } else if let Expression::Matrix { rows } = object.as_ref() {
                                    let l = if rows.len() <= 1 { rows.first().map(|r| r.len()).unwrap_or(0) } else { rows.len() } as u64;
                                    if l > 0 { len_opt = Some(l); }
                                }
                                let base_ptr_opt: Option<PointerValue<'ctx>> = {
                                    let v = self.generate_expression(object).ok();
                                    v.and_then(|bv| if bv.is_pointer_value() { Some(bv.into_pointer_value()) } else { None })
                                };
                                if let (Some(len), Some(base_ptr)) = (len_opt, base_ptr_opt) {
                                    // Print opening bracket
                                    let open = self.builder.build_global_string_ptr("[", "obrm").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args_open = vec![open.as_pointer_value().into()];
                                    self.builder.build_call(printf_fn, &args_open, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                    // Prepare idx alloca
                                    let i64_bte: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                    let idx_alloca = self.create_entry_block_alloca("idx", i64_bte)?;
                                    self.builder.build_store(idx_alloca, self.context.i64_type().const_zero()).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let current_fn = self.current_function.ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
                                    let cond_bb = self.context.append_basic_block(current_fn, "printmap.cond");
                                    let body_bb = self.context.append_basic_block(current_fn, "printmap.body");
                                    let inc_bb = self.context.append_basic_block(current_fn, "printmap.inc");
                                    let end_bb = self.context.append_basic_block(current_fn, "printmap.end");

                                    self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // cond
                                    self.builder.position_at_end(cond_bb);
                                    let idx_cur = self.builder.build_load(i64_bte, idx_alloca, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                    let endc = self.context.i64_type().const_int(len, false);
                                    let cmp = self.builder.build_int_compare(IntPredicate::SLT, idx_cur, endc, "cmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_conditional_branch(cmp, body_bb, end_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // body
                                    self.builder.position_at_end(body_bb);
                                    let elem_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i64_type(), base_ptr, &[idx_cur], "idx") }.map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let elem_val = self.builder.build_load(self.context.i64_type(), elem_ptr, "elem").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // Evaluate closure body on x
                                    let out_val: BasicValueEnum<'ctx> = if let Some(farg) = hof_args.get(0) {
                                        if let Expression::Function { parameters, body, .. } = &farg.value {
                                            let p_name = parameters.get(0).map(|p| p.name.clone()).unwrap_or("x".to_string());
                                            let x_alloca = self.create_entry_block_alloca(&p_name, i64_bte)?;
                                            let prev_x = self.variables.insert(p_name.clone(), (x_alloca, i64_bte));
                                            self.builder.build_store(x_alloca, elem_val).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            let v: BasicValueEnum<'ctx> = match body {
                                                FunctionBody::Expression(expr) => {
                                                    let bv = self.generate_expression(expr)?; self.cast_to_int(bv, self.context.i64_type())?.into()
                                                }
                                                FunctionBody::Block(stmts) => {
                                                    let mut last_expr_value: Option<BasicValueEnum<'ctx>> = None;
                                                    let slice: &[Statement] = &stmts[..];
                                                    if let Some((last, prefix)) = slice.split_last() {
                                                        for s in prefix { let _ = self.generate_statement(s); }
                                                        if let Statement::Expression(expr) = last { let bv = self.generate_expression(expr)?; last_expr_value = Some(bv); }
                                                    }
                                                    if let Some(bv) = last_expr_value { self.cast_to_int(bv, self.context.i64_type())?.into() } else { self.context.i64_type().const_zero().into() }
                                                }
                                            };
                                            if let Some(prev) = prev_x { self.variables.insert(p_name, prev); } else { self.variables.remove("x"); }
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
                                                    let cres = self.builder.build_call(fun, &[arg_meta], "callmap").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                                    if let Some(bv) = cres.try_as_basic_value().left() {
                                                        self.cast_to_int(bv, self.context.i64_type())?.into()
                                                    } else { self.context.i64_type().const_zero().into() }
                                                } else { self.context.i64_type().const_zero().into() }
                                            } else { self.context.i64_type().const_zero().into() }
                                        } else { self.context.i64_type().const_zero().into() }
                                    } else { self.context.i64_type().const_zero().into() };
                                    // Print space if idx != 0
                                    let is_zero = self.builder.build_int_compare(IntPredicate::EQ, idx_cur, self.context.i64_type().const_zero(), "iszero").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let sp_then = self.context.append_basic_block(current_fn, "pm.sp.then");
                                    let sp_cont = self.context.append_basic_block(current_fn, "pm.sp.cont");
                                    self.builder.build_conditional_branch(is_zero, sp_cont, sp_then).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.position_at_end(sp_then);
                                    let fmt_space = self.builder.build_global_string_ptr("%s", "fmtsm").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let space = self.builder.build_global_string_ptr(" ", "spm").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args_sp: Vec<BasicValueEnum<'ctx>> = vec![fmt_space.as_pointer_value().into(), space.as_pointer_value().into()];
                                    let args_spm: Vec<_> = args_sp.into_iter().map(|v| v.into()).collect();
                                    self.builder.build_call(printf_fn, &args_spm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_unconditional_branch(sp_cont).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.position_at_end(sp_cont);
                                    // Print number
                                    let fmt_num = self.builder.build_global_string_ptr("%lld", "fmnm").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args_num: Vec<BasicValueEnum<'ctx>> = vec![fmt_num.as_pointer_value().into(), out_val];
                                    let args_numm: Vec<_> = args_num.into_iter().map(|v| v.into()).collect();
                                    self.builder.build_call(printf_fn, &args_numm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // jump to inc
                                    self.builder.build_unconditional_branch(inc_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // inc
                                    self.builder.position_at_end(inc_bb);
                                    let idx_cur2 = self.builder.build_load(i64_bte, idx_alloca, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                    let next = self.builder.build_int_add(idx_cur2, self.context.i64_type().const_int(1, false), "inc").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_store(idx_alloca, next).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // end
                                    self.builder.position_at_end(end_bb);
                                    let close = self.builder.build_global_string_ptr("]", "cbrm").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args_close = vec![close.as_pointer_value().into()];
                                    self.builder.build_call(printf_fn, &args_close, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let after_bb = self.context.append_basic_block(current_fn, "printmap.after");
                                    self.builder.build_unconditional_branch(after_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.position_at_end(after_bb);
                                    true
                                } else { false }
                            } else if field == "filter" {
                                // Temporary: print placeholder for filter until CFG is fully stabilized
                                let open = self.builder.build_global_string_ptr("[filter]", "fltph").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let args_open = vec![open.as_pointer_value().into()];
                                self.builder.build_call(printf_fn, &args_open, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                true
                            } else { false }
                        } else { false }
                    }
                    _ => false,
                };

                // 2D matrix literal pretty-print: [a b; c d; ...]
                if !handled_special {
                    if let Expression::Matrix { rows } = &arg.value {
                        if rows.len() > 1 {
                            let open = self.builder.build_global_string_ptr("[", "obrm2d").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_open = vec![open.as_pointer_value().into()];
                            self.builder.build_call(printf_fn, &args_open, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            for (ri, row) in rows.iter().enumerate() {
                                if ri > 0 {
                                    let fmt = self.builder.build_global_string_ptr("%s", "fmtrowsep").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let sep = self.builder.build_global_string_ptr("; ", "rowsep").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args_sep: Vec<BasicValueEnum<'ctx>> = vec![fmt.as_pointer_value().into(), sep.as_pointer_value().into()];
                                    let argsm: Vec<_> = args_sep.into_iter().map(|v| v.into()).collect();
                                    self.builder.build_call(printf_fn, &argsm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                }
                                for (ci, cell) in row.iter().enumerate() {
                                    if ci > 0 {
                                        let fmt = self.builder.build_global_string_ptr("%s", "fmtsp2d").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                        let sp = self.builder.build_global_string_ptr(" ", "sp2d").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                        let args_sp: Vec<BasicValueEnum<'ctx>> = vec![fmt.as_pointer_value().into(), sp.as_pointer_value().into()];
                                        let argsm: Vec<_> = args_sp.into_iter().map(|v| v.into()).collect();
                                        self.builder.build_call(printf_fn, &argsm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    }
                                    let v = self.generate_expression(cell)?;
                                    let iv = self.cast_to_int(v, self.context.i64_type())?;
                                    let fmt = self.builder.build_global_string_ptr("%lld", "fmtn2d").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args_num: Vec<BasicValueEnum<'ctx>> = vec![fmt.as_pointer_value().into(), iv.into()];
                                    let argsm: Vec<_> = args_num.into_iter().map(|vv| vv.into()).collect();
                                    self.builder.build_call(printf_fn, &argsm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                }
                            }
                            let close = self.builder.build_global_string_ptr("]", "cbrm2d").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_close = vec![close.as_pointer_value().into()];
                            self.builder.build_call(printf_fn, &args_close, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
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
                                if l > 0 { len_opt = Some(l); }
                                let v = self.generate_expression(&arg.value).ok();
                                base_ptr_opt = v.and_then(|bv| if bv.is_pointer_value() { Some(bv.into_pointer_value()) } else { None });
                                is_matrix = true;
                            }
                        }
                        Expression::Identifier(name) => {
                            if let Some(Type::Matrix { element_type: _, dimensions }) = self.semantic.get_variable_type(name) {
                                if dimensions.len() == 1 {
                                    let l = dimensions[0] as u64;
                                    if l > 0 { len_opt = Some(l); }
                                    let v = self.generate_expression(&arg.value).ok();
                                    base_ptr_opt = v.and_then(|bv| if bv.is_pointer_value() { Some(bv.into_pointer_value()) } else { None });
                                    is_matrix = true;
                                } else if dimensions.len() > 1 {
                                    // Multi-dimensional matrix: print placeholder for now
                                    let placeholder = self.builder.build_global_string_ptr("[matrix]", "matph").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args = vec![placeholder.as_pointer_value().into()];
                                    self.builder.build_call(printf_fn, &args, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    handled_special = true;
                                }
                            } else if let Some(l) = self.vector_lengths.get(name).cloned() {
                                if l > 0 {
                                    let v = self.generate_expression(&arg.value).ok();
                                    base_ptr_opt = v.and_then(|bv| if bv.is_pointer_value() { Some(bv.into_pointer_value()) } else { None });
                                    len_opt = Some(l);
                                    is_matrix = true;
                                }
                            } else if let Some(rank) = self.matrix_rank.get(name).cloned() {
                                if rank > 1 {
                                    let placeholder = self.builder.build_global_string_ptr("[matrix]", "matph2").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args = vec![placeholder.as_pointer_value().into()];
                                    self.builder.build_call(printf_fn, &args, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    handled_special = true;
                                }
                            }
                        }
                        _ => {}
                    }

                    if is_matrix {
                        if let (Some(len), Some(base_ptr)) = (len_opt, base_ptr_opt) {
                            // Print opening bracket
                            let open = self.builder.build_global_string_ptr("[", "obrv").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_open = vec![open.as_pointer_value().into()];
                            self.builder.build_call(printf_fn, &args_open, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                            let i64_bte: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                            let idx_alloca = self.create_entry_block_alloca("idx", i64_bte)?;
                            self.builder.build_store(idx_alloca, self.context.i64_type().const_zero()).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let current_fn = self.current_function.ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
                            let cond_bb = self.context.append_basic_block(current_fn, "printvec.cond");
                            let body_bb = self.context.append_basic_block(current_fn, "printvec.body");
                            let inc_bb = self.context.append_basic_block(current_fn, "printvec.inc");
                            let end_bb = self.context.append_basic_block(current_fn, "printvec.end");

                            self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // cond
                            self.builder.position_at_end(cond_bb);
                            let idx_cur = self.builder.build_load(i64_bte, idx_alloca, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                            let endc = self.context.i64_type().const_int(len, false);
                            let cmp = self.builder.build_int_compare(IntPredicate::SLT, idx_cur, endc, "cmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder.build_conditional_branch(cmp, body_bb, end_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // body
                            self.builder.position_at_end(body_bb);
                            let elem_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i64_type(), base_ptr, &[idx_cur], "vidx") }.map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let elem_val = self.builder.build_load(self.context.i64_type(), elem_ptr, "velem").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // Print space if idx != 0
                            let is_zero = self.builder.build_int_compare(IntPredicate::EQ, idx_cur, self.context.i64_type().const_zero(), "iszero").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let sp_then = self.context.append_basic_block(current_fn, "pv.sp.then");
                            let sp_cont = self.context.append_basic_block(current_fn, "pv.sp.cont");
                            self.builder.build_conditional_branch(is_zero, sp_cont, sp_then).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder.position_at_end(sp_then);
                            let fmt_space = self.builder.build_global_string_ptr("%s", "fmtsv").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let space = self.builder.build_global_string_ptr(" ", "spv").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_sp: Vec<BasicValueEnum<'ctx>> = vec![fmt_space.as_pointer_value().into(), space.as_pointer_value().into()];
                            let args_spm: Vec<_> = args_sp.into_iter().map(|v| v.into()).collect();
                            self.builder.build_call(printf_fn, &args_spm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder.build_unconditional_branch(sp_cont).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder.position_at_end(sp_cont);
                            // Print number
                            let fmt_num = self.builder.build_global_string_ptr("%lld", "fmnv").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_num: Vec<BasicValueEnum<'ctx>> = vec![fmt_num.as_pointer_value().into(), elem_val];
                            let args_numm: Vec<_> = args_num.into_iter().map(|v| v.into()).collect();
                            self.builder.build_call(printf_fn, &args_numm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // jump to inc
                            self.builder.build_unconditional_branch(inc_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // inc
                            self.builder.position_at_end(inc_bb);
                            let idx_cur2 = self.builder.build_load(i64_bte, idx_alloca, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                            let next = self.builder.build_int_add(idx_cur2, self.context.i64_type().const_int(1, false), "inc").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder.build_store(idx_alloca, next).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            // end
                            self.builder.position_at_end(end_bb);
                            let close = self.builder.build_global_string_ptr("]", "cbrv").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let args_close = vec![close.as_pointer_value().into()];
                            self.builder.build_call(printf_fn, &args_close, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            let after_bb = self.context.append_basic_block(current_fn, "printvec.after");
                            self.builder.build_unconditional_branch(after_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            self.builder.position_at_end(after_bb);
                            handled_special = true;
                        }
                    }

                    }

                if !handled_special {
                    // Fallback to simple formatted printing for this argument
                    let value = self.generate_expression(&arg.value)?;
                    // Detect string arguments: string literal or identifier typed as string in semantics
                    let is_string_arg = match &arg.value {
                        Expression::Literal(Literal::String(_)) => true,
                        Expression::Identifier(name) => {
                            if let Some(t) = self.semantic.get_variable_type(name) {
                                matches!(t, Type::Identifier(s) if s == "string" || s == "String" || s == "str")
                            } else { false }
                        }
                        _ => false,
                    };
                    if value.is_int_value() && value.into_int_value().get_type().get_bit_width() == 1 {
                        // bool -> "true"/"false" via %s
                        let iv = value.into_int_value();
                        let fmt = self.builder.build_global_string_ptr("%s", "fmtb").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let t = self.builder.build_global_string_ptr("true", "true_str").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let f = self.builder.build_global_string_ptr("false", "false_str").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let sel_ptr = self.builder.build_select(iv, t.as_pointer_value(), f.as_pointer_value(), "boolstr").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let args: Vec<BasicValueEnum<'ctx>> = vec![fmt.as_pointer_value().into(), sel_ptr.into()];
                        let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                        self.builder.build_call(printf_fn, &argsm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    } else if value.is_int_value() {
                        let iv = value.into_int_value();
                        let bw = iv.get_type().get_bit_width();
                        let widened: BasicValueEnum<'ctx> = if bw < 64 { self.builder.build_int_s_extend(iv, self.context.i64_type(), "ext").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into() } else { iv.into() };
                        let fmt = self.builder.build_global_string_ptr("%lld", "fmti").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let args: Vec<BasicValueEnum<'ctx>> = vec![fmt.as_pointer_value().into(), widened];
                        let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                        self.builder.build_call(printf_fn, &argsm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    } else if value.is_float_value() {
                        let fmt = self.builder.build_global_string_ptr("%f", "fmtf").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let args: Vec<BasicValueEnum<'ctx>> = vec![fmt.as_pointer_value().into(), value.into()];
                        let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                        self.builder.build_call(printf_fn, &argsm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    } else if value.is_pointer_value() {
                        let mut did_matrix = false;
                        // If this is a matrix-typed identifier, prefer special handling
                        if let Expression::Identifier(name) = &arg.value {
                            if let Some(Type::Matrix { element_type: _, dimensions }) = self.semantic.get_variable_type(name) {
                                if dimensions.len() == 1 {
                                    // Print as 1D vector
                                    let len = dimensions[0] as u64;
                                    let base_ptr = value.into_pointer_value();
                                    let open = self.builder.build_global_string_ptr("[", "obrvb").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args_open = vec![open.as_pointer_value().into()];
                                    self.builder.build_call(printf_fn, &args_open, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;

                                    let i64_bte: BasicTypeEnum<'ctx> = self.context.i64_type().into();
                                    let idx_alloca = self.create_entry_block_alloca("idx", i64_bte)?;
                                    self.builder.build_store(idx_alloca, self.context.i64_type().const_zero()).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let current_fn = self.current_function.ok_or_else(|| CodegenError::CompilationError("No current function".to_string()))?;
                                    let cond_bb = self.context.append_basic_block(current_fn, "printvecb.cond");
                                    let body_bb = self.context.append_basic_block(current_fn, "printvecb.body");
                                    let inc_bb = self.context.append_basic_block(current_fn, "printvecb.inc");
                                    let end_bb = self.context.append_basic_block(current_fn, "printvecb.end");

                                    self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // cond
                                    self.builder.position_at_end(cond_bb);
                                    let idx_cur = self.builder.build_load(i64_bte, idx_alloca, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                    let endc = self.context.i64_type().const_int(len, false);
                                    let cmp = self.builder.build_int_compare(IntPredicate::SLT, idx_cur, endc, "cmp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_conditional_branch(cmp, body_bb, end_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // body
                                    self.builder.position_at_end(body_bb);
                                    let elem_ptr = unsafe { self.builder.build_in_bounds_gep(self.context.i64_type(), base_ptr, &[idx_cur], "v2idx") }.map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let elem_val = self.builder.build_load(self.context.i64_type(), elem_ptr, "v2elem").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // spacing
                                    let is_zero = self.builder.build_int_compare(IntPredicate::EQ, idx_cur, self.context.i64_type().const_zero(), "iszero").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let sp_then = self.context.append_basic_block(current_fn, "pvb.sp.then");
                                    let sp_cont = self.context.append_basic_block(current_fn, "pvb.sp.cont");
                                    self.builder.build_conditional_branch(is_zero, sp_cont, sp_then).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.position_at_end(sp_then);
                                    let fmt_space = self.builder.build_global_string_ptr("%s", "fmtsvb").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let space = self.builder.build_global_string_ptr(" ", "spvb").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args_sp: Vec<BasicValueEnum<'ctx>> = vec![fmt_space.as_pointer_value().into(), space.as_pointer_value().into()];
                                    let args_spm: Vec<_> = args_sp.into_iter().map(|v| v.into()).collect();
                                    self.builder.build_call(printf_fn, &args_spm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_unconditional_branch(sp_cont).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.position_at_end(sp_cont);
                                    // number
                                    let fmt_num = self.builder.build_global_string_ptr("%lld", "fmnvb").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args_num: Vec<BasicValueEnum<'ctx>> = vec![fmt_num.as_pointer_value().into(), elem_val];
                                    let args_numm: Vec<_> = args_num.into_iter().map(|v| v.into()).collect();
                                    self.builder.build_call(printf_fn, &args_numm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // inc
                                    self.builder.build_unconditional_branch(inc_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.position_at_end(inc_bb);
                                    let idx_cur2 = self.builder.build_load(i64_bte, idx_alloca, "idx").map_err(|e| CodegenError::CompilationError(e.to_string()))?.into_int_value();
                                    let next = self.builder.build_int_add(idx_cur2, self.context.i64_type().const_int(1, false), "inc").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_store(idx_alloca, next).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.build_unconditional_branch(cond_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    // end
                                    self.builder.position_at_end(end_bb);
                                    let close = self.builder.build_global_string_ptr("]", "cbrvb").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args_close = vec![close.as_pointer_value().into()];
                                    self.builder.build_call(printf_fn, &args_close, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let after_bb = self.context.append_basic_block(current_fn, "printvecb.after");
                                    self.builder.build_unconditional_branch(after_bb).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    self.builder.position_at_end(after_bb);
                                    did_matrix = true;
                                } else if dimensions.len() > 1 {
                                    // Multi-dimensional: safe placeholder
                                    let placeholder = self.builder.build_global_string_ptr("[matrix]", "matphb").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    let args = vec![placeholder.as_pointer_value().into()];
                                    self.builder.build_call(printf_fn, &args, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                    did_matrix = true;
                                }
                            }
                        }
                        if !did_matrix {
                            if is_string_arg {
                                // Print as C string
                                let fmt = self.builder.build_global_string_ptr("%s", "fmts").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let args: Vec<BasicValueEnum<'ctx>> = vec![fmt.as_pointer_value().into(), value.into()];
                                let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                                self.builder.build_call(printf_fn, &argsm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            } else {
                                // Non-string pointers as %p
                                let fmt = self.builder.build_global_string_ptr("%p", "fmtp").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                let args: Vec<BasicValueEnum<'ctx>> = vec![fmt.as_pointer_value().into(), value.into()];
                                let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                                self.builder.build_call(printf_fn, &argsm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                            }
                        }
                    } else {
                        let fmt = self.builder.build_global_string_ptr("%lld", "fmti").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                        let zero: BasicValueEnum<'ctx> = self.context.i64_type().const_zero().into();
                        let args: Vec<BasicValueEnum<'ctx>> = vec![fmt.as_pointer_value().into(), zero];
                        let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                        self.builder.build_call(printf_fn, &argsm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    }
                }

                // Add a space between arguments (not at end)
                if i < arguments.len() - 1 {
                    let space = self.builder.build_global_string_ptr(" ", "spc").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                    let args: Vec<BasicValueEnum<'ctx>> = vec![space.as_pointer_value().into()];
                    let argsm: Vec<_> = args.into_iter().map(|v| v.into()).collect();
                    self.builder.build_call(printf_fn, &argsm, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                }
            }
            // finally, newline
            let newline = self.builder.build_global_string_ptr("\n", "nl").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
            let args_nl = vec![newline.as_pointer_value().into()];
            self.builder.build_call(printf_fn, &args_nl, "printf_call").map_err(|e| CodegenError::CompilationError(e.to_string()))?;
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
    // Verify module before emitting object to avoid backend crashes
    if let Err(msg) = self.module.verify() {
        return Err(CodegenError::CompilationError(format!("LLVM IR verification failed: {}", msg)));
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
                    if name == "main" && !self.runtime_mode { continue; }
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
                        // In runtime mode, emit user main as `peano_main` symbol
                        let fname = if self.runtime_mode && name == "main" { "peano_main" } else { name };
                        let f = self.module.add_function(fname, fn_type, None);
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
                                let mut param_tys_bte: Vec<BasicTypeEnum<'ctx>> = Vec::new();
                                // Prepend receiver pointer parameter for methods
                                param_tys_bte.push(self.context.ptr_type(AddressSpace::default()).into());
                                param_tys_bte.extend(
                                    parameters.iter()
                                        .map(|p| p.param_type.as_ref().and_then(|t| self.map_ast_type(t)).unwrap_or(self.context.i64_type().into()))
                                );
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
                    if name == "main" && !self.runtime_mode { continue; }
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

                                // Bind params to allocas. For inherent methods, prepend implicit receiver `self` as &type_name.
                                let mut param_index = 0usize;
                                if trait_name.is_none() {
                                    // Receiver is first param in LLVM function; bind as `self` with pointer type to struct
                                    if let Some((_st, _)) = self.struct_types.get(type_name) {
                                        if let Some(first_param) = f.get_param_iter().next() {
                                            let p_ty = self.context.ptr_type(AddressSpace::default()).into();
                                            let alloca = self.create_entry_block_alloca("self", p_ty)?;
                                            self.builder.build_store(alloca, first_param).map_err(|e| CodegenError::CompilationError(e.to_string()))?;
                                            self.variables.insert("self".to_string(), (alloca, p_ty));
                                            param_index = 1;
                                        }
                                    }
                                }
                                for (i, param) in f.get_param_iter().enumerate().skip(param_index) {
                                    let p_name = parameters.get(i - param_index).map(|p| p.name.clone()).unwrap_or(format!("arg{}", i - param_index));
                                    let p_ty = parameters.get(i - param_index)
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
