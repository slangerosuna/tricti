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
        type_annotation: Option<Type>,
        value: ConstValue,
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
        parameters: Vec<Parameter>,
        return_type: Option<Type>,
        body: FunctionBody,
    },
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
    Integer(i64),
    Float(f64),
    String(String),
    Boolean(bool),
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
    },
    Trait {
        associated_types: Vec<String>,
        methods: HashMap<String, Type>, // function types
    },
}
