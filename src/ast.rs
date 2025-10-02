use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    VariableDecl {
        name: String,
        type_annotation: Option<Type>,
        value: Expression,
    },
    ConstDecl {
        name: String,
        type_params: Vec<TypeParam>,
        type_annotation: Option<Type>,
        value: ConstValue,
        extern_linkage: Option<String>,
    },
    Assignment {
        target: Expression,
        operator: AssignmentOp,
        value: Expression,
    },
    Expression(Expression),
    Return(Option<Expression>),
    Break(Option<Expression>),
    Use {
        is_public: bool,
        path: Vec<String>,
        alias: Option<String>,
    },
    ImplBlock {
        type_params: Vec<TypeParam>,
        trait_name: Option<String>,
        type_name: String,
        methods: Vec<Statement>,
    },
    ImplMethod {
        name: String,
        type_params: Vec<TypeParam>,
        parameters: Vec<Parameter>,
        return_type: Option<Type>,
        body: FunctionBody,
    },
    ForLoop {
        variable: String,
        type_annotation: Option<Type>,
        iterable: Expression,
        body: Vec<Statement>,
    },
    // Rust-style module declaration: `mod name;` or `mod name { .. }`
    ModuleDecl {
        is_public: bool,
        name: String,
        items: Option<Vec<Statement>>, // None => external file module to be loaded by driver
    },
    IfDef {
        condition: String,
        then_branch: Vec<Statement>,
        else_branch: Option<Vec<Statement>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Type(Type),
    Expression(Expression),
    TableDef(TableDef),
    SystemDef(SystemDef),
    ComposeDef(ComposeDef),
    DatabaseDef(DatabaseDef),
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssignmentOp {
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ElementMulAssign,
    ElementDivAssign,
    ModAssign,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Literal(Literal),
    Identifier(String),
    BinaryOp {
        left: Box<Expression>,
        operator: BinaryOperator,
        right: Box<Expression>,
    },
    UnaryOp {
        operator: UnaryOperator,
        operand: Box<Expression>,
    },
    Cast {
        value: Box<Expression>,
        to_type: Type,
    },
    Call {
        function: Box<Expression>,
        type_args: Vec<Type>,
        arguments: Vec<Argument>,
    },
    FieldAccess {
        object: Box<Expression>,
        field: String,
    },
    Index {
        object: Box<Expression>,
        indices: Vec<Expression>,
    },
    If {
        condition: Box<Expression>,
        then_branch: Vec<Statement>,
        else_branch: Option<Vec<Statement>>,
    },
    IfExpr {
        condition: Box<Expression>,
        then_expr: Box<Expression>,
        else_expr: Option<Box<Expression>>,
    },
    Loop {
        body: Vec<Statement>,
    },
    Block {
        statements: Vec<Statement>,
    },
    UnsafeBlock {
        statements: Vec<Statement>,
    },
    Function {
        is_async: bool,
        type_params: Vec<TypeParam>,
        parameters: Vec<Parameter>,
        return_type: Option<Type>,
        body: FunctionBody,
    },
    Tuple(Vec<Expression>),
    Match {
        value: Box<Expression>,
        arms: Vec<MatchArm>,
    },
    StructLiteral {
        type_name: Option<String>,
        fields: HashMap<String, Expression>,
    },
    ArrayNew {
        element_type: Type,
        dimensions: Vec<Expression>,
    },
    Matrix {
        rows: Vec<Vec<Expression>>,
    },
    Range {
        start: Box<Expression>,
        end: Box<Expression>,
        step: Option<Box<Expression>>,
    },
    Question(Box<Expression>),
    Unwrap(Box<Expression>),
    Query(QuerySpec),
    Shader {
        shader_type: ShaderType,
        fields: Vec<ShaderField>,
        constants: Vec<Statement>,
    },
    StaticPath {
        segments: Vec<String>,
        type_args: Vec<Type>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShaderType {
    Vertex,
    Fragment,
    Compute,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShaderField {
    pub name: String,
    pub field_type: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Expression,
    pub body: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FunctionBody {
    Expression(Box<Expression>),
    Block(Vec<Statement>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub param_type: Option<Type>,
    pub default_value: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    pub name: String,
    pub bounds: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Argument {
    pub name: Option<String>,
    pub value: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Integer(IntegerLiteral),
    Float(f64),
    String(String),
    Boolean(bool),
    Char(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntSuffix {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}

impl IntSuffix {
    pub fn type_name(&self) -> &'static str {
        match self {
            IntSuffix::I8 => "i8",
            IntSuffix::I16 => "i16",
            IntSuffix::I32 => "i32",
            IntSuffix::I64 => "i64",
            IntSuffix::U8 => "u8",
            IntSuffix::U16 => "u16",
            IntSuffix::U32 => "u32",
            IntSuffix::U64 => "u64",
        }
    }

    pub fn bit_width(&self) -> u32 {
        match self {
            IntSuffix::I8 | IntSuffix::U8 => 8,
            IntSuffix::I16 | IntSuffix::U16 => 16,
            IntSuffix::I32 | IntSuffix::U32 => 32,
            IntSuffix::I64 | IntSuffix::U64 => 64,
        }
    }

    pub fn is_signed(&self) -> bool {
        matches!(
            self,
            IntSuffix::I8 | IntSuffix::I16 | IntSuffix::I32 | IntSuffix::I64
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntegerLiteral {
    pub raw: String,
    pub value: u128,
    pub suffix: Option<IntSuffix>,
}

impl IntegerLiteral {
    pub fn type_name(&self) -> &'static str {
        self.suffix
            .as_ref()
            .map(IntSuffix::type_name)
            .unwrap_or("i64")
    }

    pub fn bit_width(&self) -> u32 {
        self.suffix.as_ref().map(IntSuffix::bit_width).unwrap_or(64)
    }

    pub fn is_signed(&self) -> bool {
        self.suffix
            .as_ref()
            .map(IntSuffix::is_signed)
            .unwrap_or(true)
    }
}

impl Literal {
    pub fn integer_from_parts(raw: String, value: u128, suffix: Option<IntSuffix>) -> Self {
        Literal::Integer(IntegerLiteral { raw, value, suffix })
    }

    pub fn integer_zero() -> Self {
        Literal::integer_from_parts("0".to_string(), 0, None)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOperator {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    ElementMul,
    ElementDiv,
    ElementMod,
    ShiftLeft,
    ShiftRight,
    And,
    Or,
    LogicalAnd,
    LogicalOr,
    Xor,
    Equal,
    NotEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOperator {
    Negate,
    Not,
    BitwiseNot,
    Deref,
    AddressOf,
    MutAddressOf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    None,
    Identifier {
        name: String,
        type_args: Vec<Type>,
    },
    Pointer {
        is_mutable: bool,
        pointee: Box<Type>,
    },
    RawPointer {
        pointee: Box<Type>,
    },
    Optional {
        inner: Box<Type>,
    },
    Result {
        inner: Box<Type>,
    },
    Tuple(Vec<Type>),
    Matrix {
        element_type: Box<Type>,
        dimensions: Vec<usize>,
    },
    Function {
        parameters: Vec<Type>,
        return_type: Box<Type>,
    },
    Struct {
        fields: HashMap<String, Type>,
    },
    Enum {
        variants: HashMap<String, Option<Type>>,
        order: Vec<String>,
    },
    Trait {
        associated_types: Vec<String>,
        methods: HashMap<String, Type>, // function types
    },
    Reference {
        is_mutable: bool,
        inner: Box<Type>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableDef {
    pub name: String,
    pub columns: Vec<TableColumn>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableColumn {
    pub name: String,
    pub column_type: Type,
    pub annotations: Vec<TableAnnotation>,
    pub default_value: Option<Expression>,
    pub is_computed: bool,
    pub computed_expression: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableAnnotation {
    pub name: String,
    pub args: Vec<Expression>,
}

// System Execution Model AST structures
#[derive(Debug, Clone, PartialEq)]
pub struct SystemDef {
    pub name: String,
    pub parameters: Vec<SystemParameter>,
    pub return_type: Option<Type>,
    pub body: Vec<Statement>,
    pub is_async: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SystemParameter {
    Query {
        name: String,
        query_spec: QuerySpec,
    },
    Resource {
        param_type: String,
        name: String,
        resource_type: Type,
        access: ResourceAccess,
    },
    Regular {
        param_type: String,
        name: String,
        value_type: Type,
        default_value: Option<Expression>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResourceAccess {
    Immutable, // &T
    Mutable,   // &mut T
    Owned,     // T
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuerySpec {
    pub projections: Vec<FieldProjection>,
    pub from_table: String,
    pub where_clause: Option<Box<Expression>>,
    pub joins: Vec<JoinClause>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldProjection {
    pub name: String,
    pub field_type: Option<Type>,
    pub access: Option<ResourceAccess>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JoinClause {
    pub join_type: JoinType,
    pub table: String,
    pub condition: Box<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
}

// Compose and Database definitions for completeness
#[derive(Debug, Clone, PartialEq)]
pub struct ComposeDef {
    pub entries: Vec<ComposeEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComposeEntry {
    pub source: ComposeNode,
    pub targets: Vec<ComposeNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComposeNode {
    Single(String),
    Tuple(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseDef {
    pub entries: Vec<DatabaseEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseEntry {
    pub name: String,
    pub table_type: Option<String>,
}
