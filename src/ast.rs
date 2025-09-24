use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
```rust
Char(u32),
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
        path: Vec<String>,
    },
    ImplBlock {
        trait_name: Option<String>,
        type_name: String,
        methods: Vec<Statement>,
    },
    ForLoop {
        variable: String,
        type_annotation: Option<Type>,
        iterable: Expression,
        body: Vec<Statement>,
    },
    // Rust-style module declaration: `mod name;` or `mod name { .. }`
    ModuleDecl {
        name: String,
        items: Option<Vec<Statement>>, // None => external file module to be loaded by driver
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Type(Type),
    Expression(Expression),
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
    Loop {
        body: Vec<Statement>,
    },
    Block {
        statements: Vec<Statement>,
    },
    Function {
        is_async: bool,
        type_params: Vec<String>,
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
        fields: HashMap<String, Expression>,
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
    Shader {
        shader_type: ShaderType,
        fields: Vec<ShaderField>,
        constants: Vec<Statement>,
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
    And,
    Or,
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
    Identifier(String),
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
}
