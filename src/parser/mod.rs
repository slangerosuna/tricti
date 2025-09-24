use pest::*;
use pest_derive::*;
use std::collections::HashMap;
use crate::ast::*;

#[derive(Parser)]
#[grammar = "parser/grammar.pest"]
struct PnParser;

pub fn parse(file: String) -> Program {
    let successful_parse = PnParser::parse(Rule::program, &file)
        .unwrap_or_else(|e| panic!("Parse error: {}", e));
    
    let program_pair = successful_parse.into_iter().next().unwrap();
    parse_program(program_pair)
}

fn parse_char_literal(content: &str) -> char {
    // content includes surrounding single quotes
    if content.len() < 2 {
        panic!("invalid char literal: {}", content);
    }
    let inner = &content[1..content.len() - 1];
    if inner.starts_with('\\') {
        if inner.len() < 2 {
            panic!("invalid escape in char literal: {}", content);
        }
        match inner.chars().nth(1).unwrap() {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '0' => '\0',
            '\\' => '\\',
            '\'' => '\'',
            other => panic!("unsupported escape \\{} in char literal", other),
        }
    } else {
        let mut chars = inner.chars();
        let ch = chars.next().expect("empty char literal");
        if chars.next().is_some() {
            panic!("char literal must be a single code point: {}", content);
        }
        ch
    }
}

fn parse_integer_literal(pair: pest::iterators::Pair<Rule>) -> Literal {
    let text = pair.as_str();

    const SUFFIXES: [&str; 8] = ["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64"];

    let suffix = SUFFIXES
        .iter()
        .find(|suf| text.len() > suf.len() && text.ends_with(**suf))
        .map(|s| *s);

    let (digits, suffix_enum) = match suffix {
        Some(suf) => {
            let digits = &text[..text.len() - suf.len()];
            let suffix_enum = match suf {
                "i8" => IntSuffix::I8,
                "i16" => IntSuffix::I16,
                "i32" => IntSuffix::I32,
                "i64" => IntSuffix::I64,
                "u8" => IntSuffix::U8,
                "u16" => IntSuffix::U16,
                "u32" => IntSuffix::U32,
                "u64" => IntSuffix::U64,
                _ => unreachable!(),
            };
            (digits, Some(suffix_enum))
        }
        None => (text, None),
    };

    if digits.is_empty() {
        panic!("integer literal missing digits: {}", text);
    }

    let value = digits.parse::<u128>().unwrap_or_else(|_| panic!("invalid integer literal: {}", text));

    Literal::integer_from_parts(text.to_string(), value, suffix_enum)
}

fn parse_program(pair: pest::iterators::Pair<Rule>) -> Program {
    let mut statements = Vec::new();
    
    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::statement => {
                statements.push(parse_statement(inner_pair));
            }
            Rule::EOI => break,
            _ => {}
        }
    }
    
    Program { statements }
}

fn parse_statement(pair: pest::iterators::Pair<Rule>) -> Statement {
    let inner_pair = pair.into_inner().next().unwrap();
    
    match inner_pair.as_rule() {
        Rule::variable_decl => parse_variable_decl(inner_pair),
        Rule::const_decl => parse_const_decl(inner_pair),
        Rule::assignment => parse_assignment(inner_pair),
    Rule::return_statement => parse_return_statement(inner_pair),
    Rule::break_statement => parse_break_statement(inner_pair),
    Rule::for_loop => parse_for_loop(inner_pair),
    Rule::use_statement => parse_use_statement(inner_pair),
    Rule::mod_decl => parse_mod_decl(inner_pair),
    Rule::impl_block => parse_impl_block(inner_pair),
        Rule::expression => Statement::Expression(parse_expression(inner_pair)),
        _ => panic!("Unexpected statement rule: {:?}", inner_pair.as_rule()),
    }
}

fn parse_variable_decl(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    
    let mut type_annotation = None;
    let mut value_pair = None;
    
    for pair in inner {
        match pair.as_rule() {
            Rule::r#type => type_annotation = Some(parse_type(pair)),
            Rule::expression => value_pair = Some(pair),
            _ => {}
        }
    }
    
    let value = parse_expression(value_pair.unwrap());
    
    Statement::VariableDecl {
        name,
        type_annotation,
        value,
    }
}

fn parse_const_decl(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut inner = pair.into_inner();
    
    let mut extern_linkage = None;
    let first = inner.next().unwrap();
    let name;
    if first.as_str() == "extern" {
        // Check if next is string
        let next = inner.peek().unwrap();
        if next.as_rule() == Rule::string {
            let linkage = inner.next().unwrap().as_str().to_string();
            extern_linkage = Some(linkage);
        } else {
            extern_linkage = None;
        }
        name = inner.next().unwrap().as_str().to_string();
    } else {
        name = first.as_str().to_string();
    }
    
    let mut type_annotation = None;
    let mut value_pair = None;
    
    for pair in inner {
        match pair.as_rule() {
            Rule::r#type => {
                if type_annotation.is_none() {
                    type_annotation = Some(parse_type(pair.clone()));
                }
                value_pair = Some(pair);
            }
            Rule::expression => value_pair = Some(pair),
            _ => {}
        }
    }
    
    let value = if let Some(pair) = value_pair {
        match pair.as_rule() {
            Rule::r#type => ConstValue::Type(parse_type(pair)),
            Rule::expression => ConstValue::Expression(parse_expression(pair)),
            _ => panic!("Unexpected const value rule: {:?}", pair.as_rule()),
        }
    } else {
        panic!("Expected const value");
    };
    
    Statement::ConstDecl {
        name,
        type_annotation,
        value,
        extern_linkage,
    }
}

fn parse_assignment(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut inner = pair.into_inner();
    let target = parse_expression(inner.next().unwrap());
    let operator_pair = inner.next().unwrap();
    let operator_str = operator_pair.as_str();
    let operator = match operator_str {
        "=" => AssignmentOp::Assign,
        "+=" => AssignmentOp::AddAssign,
        "-=" => AssignmentOp::SubAssign,
        "*=" => AssignmentOp::MulAssign,
        "/=" => AssignmentOp::DivAssign,
        ".*=" => AssignmentOp::ElementMulAssign,
        "./=" => AssignmentOp::ElementDivAssign,
        "\\=" => AssignmentOp::ModAssign,
        _ => panic!("Unknown assignment operator: {}", operator_str),
    };
    
    let value = parse_expression(inner.next().unwrap());
    
    Statement::Assignment {
        target,
        operator,
        value,
    }
}

fn parse_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let rule = pair.as_rule();
    match rule {
        Rule::or => parse_binary_expression(pair),
        Rule::and => parse_binary_expression(pair),
        Rule::comparison => parse_binary_expression(pair),
        Rule::addition => parse_binary_expression(pair),
        Rule::multiplication => parse_binary_expression(pair),
    Rule::with_range => parse_with_range(pair),
    Rule::unary => parse_unary(pair),
        Rule::post_fix => parse_postfix_expression(pair),
        Rule::primary => parse_primary_expression(pair),
    Rule::function => parse_function(pair),
        Rule::literal => parse_literal(pair),
        Rule::identifier => Expression::Identifier(pair.as_str().to_string()),
        Rule::number => parse_number(pair),
        Rule::integer => Expression::Literal(parse_integer_literal(pair)),
        Rule::float => Expression::Literal(Literal::Float(pair.as_str().parse().unwrap())),
        Rule::string => {
            let content = pair.as_str();
            let unquoted = &content[1..content.len()-1]; // Remove quotes
            Expression::Literal(Literal::String(unquoted.to_string()))
        },
        Rule::char => {
            let content = pair.as_str();
            let ch = parse_char_literal(content);
            Expression::Literal(Literal::Char(ch))
        }
        Rule::boolean => Expression::Literal(Literal::Boolean(pair.as_str() == "true")),
        _ => {
            // For wrapped expressions, unwrap them
            let inner = pair.into_inner().next();
            if let Some(inner_pair) = inner {
                parse_expression(inner_pair)
            } else {
                panic!("Unexpected expression rule: {:?}", rule)
            }
        }
    }
}

fn parse_with_range(pair: pest::iterators::Pair<Rule>) -> Expression {
    // with_range = { unary ~ (":" ~ unary ~ (":" ~ unary)?)? }
    let inners: Vec<_> = pair.into_inner().collect();
    if inners.len() == 1 {
        return parse_expression(inners[0].clone());
    }
    let start = parse_expression(inners[0].clone());
    let end = parse_expression(inners[1].clone());
    let step = if inners.len() >= 3 {
        Some(Box::new(parse_expression(inners[2].clone())))
    } else { None };
    Expression::Range { start: Box::new(start), end: Box::new(end), step }
}

fn parse_unary(pair: pest::iterators::Pair<Rule>) -> Expression {
    // unary = { op_unary? ~ post_fix }
    let mut it = pair.into_inner();
    let first = it.next();
    if let Some(p) = first.clone() {
        match p.as_rule() {
            Rule::post_fix => {
                // no unary operator
                return parse_postfix_expression(p);
            }
            Rule::op_unary => {
                let op_str = p.as_str();
                let operand_pair = it.next().expect("unary missing operand");
                let operand_expr = match operand_pair.as_rule() {
                    Rule::post_fix => parse_postfix_expression(operand_pair),
                    _ => parse_expression(operand_pair),
                };
                let op = match op_str {
                    "-" => UnaryOperator::Negate,
                    "!" => UnaryOperator::Not,
                    "~" => UnaryOperator::BitwiseNot,
                    "*" => UnaryOperator::Deref,
                    s if s.starts_with("&mut") => UnaryOperator::MutAddressOf,
                    "&" => UnaryOperator::AddressOf,
                    _ => UnaryOperator::Not,
                };
                return Expression::UnaryOp { operator: op, operand: Box::new(operand_expr) };
            }
            _ => {}
        }
    }
    // Fallback: just parse as expression
    if let Some(p) = first { parse_expression(p) } else { Expression::Literal(Literal::integer_zero()) }
}

fn parse_binary_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let pairs: Vec<_> = pair.into_inner().collect();
    let mut iter = pairs.into_iter();
    let mut left = match iter.next() {
        Some(first) => parse_expression(first),
        None => panic!("Expected expression in binary expression"),
    };

    while let Some(op_pair) = iter.next() {
        let Some(rhs_pair) = iter.next() else { break };
        let right = parse_expression(rhs_pair);
        let operator = match op_pair.as_str() {
            "+" => BinaryOperator::Add,
            "-" => BinaryOperator::Sub,
            "*" => BinaryOperator::Mul,
            "/" => BinaryOperator::Div,
            "%" => BinaryOperator::Mod,
            "and" => BinaryOperator::And,
            "or" => BinaryOperator::Or,
            "xor" => BinaryOperator::Xor,
            "==" => BinaryOperator::Equal,
            "~=" => BinaryOperator::NotEqual,
            "<" => BinaryOperator::Less,
            ">" => BinaryOperator::Greater,
            "<=" => BinaryOperator::LessEqual,
            ">=" => BinaryOperator::GreaterEqual,
            _ => panic!("Unknown binary operator: {}", op_pair.as_str()),
        };
        
        left = Expression::BinaryOp {
            left: Box::new(left),
            operator,
            right: Box::new(right),
        };
    }
    
    left
}

fn parse_postfix_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let mut inner = pair.into_inner();
    let mut expr = parse_expression(inner.next().unwrap());
    
    for suffix_pair in inner {
        match suffix_pair.as_rule() {
            Rule::call_suffix => {
                let mut arguments = Vec::new();
                for arg_pair in suffix_pair.into_inner() {
                    arguments.push(parse_argument(arg_pair));
                }
                expr = Expression::Call {
                    function: Box::new(expr),
                    arguments,
                };
            }
            Rule::static_path_suffix => {
                // Type::method → Identifier("Type_method")
                let mut it2 = suffix_pair.into_inner();
                let name = it2.next().unwrap().as_str().to_string();
                if let Expression::Identifier(base) = expr {
                    expr = Expression::Identifier(format!("{}_{}", base, name));
                }
            }
            Rule::field_suffix => {
                let mut it2 = suffix_pair.into_inner();
                let name = it2.next().unwrap().as_str().to_string();
                expr = Expression::FieldAccess { object: Box::new(expr), field: name };
            }
            _ if suffix_pair.as_str().starts_with("[") => {
                // Indexing suffix: "[ expr (, expr)* ]" possibly repeated; grammar emits it as part of post_fix
                // We'll parse indices from this suffix explicitly by iterating its inner expressions
                let mut indices: Vec<Expression> = Vec::new();
                for idx in suffix_pair.into_inner() {
                    // inner pairs are expressions
                    indices.push(parse_expression(idx));
                }
                expr = Expression::Index { object: Box::new(expr), indices };
            }
            _ => {}
        }
    }
    
    expr
}

fn parse_primary_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let inner_pair = pair.into_inner().next().unwrap();
    match inner_pair.as_rule() {
        Rule::block => Expression::Block { statements: parse_block(inner_pair) },
        Rule::function => parse_function(inner_pair),
    Rule::conditional => parse_if_expression(inner_pair),
    Rule::r#match => parse_match_expression(inner_pair),
        Rule::matrix => parse_matrix(inner_pair),
        _ => parse_expression(inner_pair),
    }
}

fn parse_matrix(pair: pest::iterators::Pair<Rule>) -> Expression {
    // matrix = "[" ~ (row ~ ";")* ~ row ~ ";"? ~ "]"
    let mut rows: Vec<Vec<Expression>> = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::row {
            let mut row_vals: Vec<Expression> = Vec::new();
            for cell in inner.into_inner() {
                // cells are expressions
                row_vals.push(parse_expression(cell));
            }
            rows.push(row_vals);
        }
    }
    Expression::Matrix { rows }
}

fn parse_match_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let mut it = pair.into_inner();
    // First inner expression is the scrutinee
    let value_pair = it.next().expect("match missing value expression");
    let value = parse_expression(value_pair);
    let mut arms: Vec<MatchArm> = Vec::new();
    // Remaining are pattern/body expression pairs; punctuation tokens are silent in grammar
    loop {
        let pat_pair = match it.next() { Some(p) => p, None => break };
        let body_pair = match it.next() { Some(p) => p, None => break };
        let pattern = parse_expression(pat_pair);
        let body = parse_expression(body_pair);
        arms.push(MatchArm { pattern, body });
    }
    Expression::Match { value: Box::new(value), arms }
}

fn parse_literal(pair: pest::iterators::Pair<Rule>) -> Expression {
    let inner_pair = pair.into_inner().next().unwrap();
    match inner_pair.as_rule() {
        Rule::integer => Expression::Literal(parse_integer_literal(inner_pair)),
        Rule::float => Expression::Literal(Literal::Float(inner_pair.as_str().parse().unwrap())),
        Rule::string => {
            let content = inner_pair.as_str();
            let unquoted = &content[1..content.len() - 1];
            Expression::Literal(Literal::String(unquoted.to_string()))
        }
        Rule::char => {
            let content = inner_pair.as_str();
            let ch = parse_char_literal(content);
            Expression::Literal(Literal::Char(ch))
        }
        Rule::boolean => Expression::Literal(Literal::Boolean(inner_pair.as_str() == "true")),
        Rule::struct_literal => {
            let mut fields: HashMap<String, Expression> = HashMap::new();
            let mut it = inner_pair.into_inner();
            loop {
                let name_pair = match it.next() { Some(p) => p, None => break };
                if name_pair.as_rule() != Rule::identifier { break; }
                let name = name_pair.as_str().to_string();
                // Expect a ':' then an expression; grammar yields the expression directly
                let expr_pair = it.next().expect("struct literal missing field expression");
                let value_expr = parse_expression(expr_pair);
                fields.insert(name, value_expr);
            }
            Expression::StructLiteral { fields }
        }
        _ => parse_expression(inner_pair),
    }
}

fn parse_number(pair: pest::iterators::Pair<Rule>) -> Expression {
    let inner_pair = pair.into_inner().next().unwrap();
    parse_expression(inner_pair)
}

fn parse_argument(pair: pest::iterators::Pair<Rule>) -> Argument {
    let mut inner = pair.into_inner();
    let first_pair = inner.next().unwrap();
    
    if let Some(second_pair) = inner.next() {
        // Named argument
        Argument {
            name: Some(first_pair.as_str().to_string()),
            value: parse_expression(second_pair),
        }
    } else {
        // Positional argument
        Argument {
            name: None,
            value: parse_expression(first_pair),
        }
    }
}

fn parse_type(pair: pest::iterators::Pair<Rule>) -> Type {
    fn parse_struct(p: pest::iterators::Pair<Rule>) -> Type {
        // Note: struct_fields is a silent rule, so we directly see identifier/type pairs here
        let mut fields = HashMap::new();
        let mut it = p.into_inner();
        loop {
            let Some(next) = it.next() else { break };
            if next.as_rule() == Rule::identifier {
                let name = next.as_str().to_string();
                let ty_pair = it.next().expect("struct field missing type");
                let ty = parse_type(ty_pair);
                fields.insert(name, ty);
            } else {
                // ignore any unexpected tokens (commas are not emitted as pairs)
            }
        }
        Type::Struct { fields }
    }

    fn parse_enum(p: pest::iterators::Pair<Rule>) -> Type {
        // enum_variants is silent; we directly see identifier and optional type pairs
        let mut variants: HashMap<String, Option<Type>> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        let mut it = p.into_inner().peekable();
        // Skip "enum" and "{"
        it.next();
        it.next();
        loop {
            if let Some(cur) = it.next() {
                if cur.as_rule() == Rule::identifier {
                    let name = cur.as_str().to_string();
                    let mut ty_opt: Option<Type> = None;
                    // Check if next is ":"
                    if let Some(next_ref) = it.peek() {
                        if next_ref.as_str() == ":" {
                            it.next(); // consume ":"
                            if let Some(type_pair) = it.next() {
                                if type_pair.as_rule() == Rule::r#type {
                                    ty_opt = Some(parse_type(type_pair));
                                }
                            }
                        }
                    }
                    variants.insert(name.clone(), ty_opt);
                    order.push(name);
                    // Skip optional ","
                    if let Some(comma_ref) = it.peek() {
                        if comma_ref.as_str() == "," {
                            it.next();
                        }
                    }
                } else if cur.as_str() == "}" {
                    break;
                }
            } else {
                break;
            }
        }
        Type::Enum { variants, order }
    }

    fn parse_pointer(p: pest::iterators::Pair<Rule>) -> Type {
        let mut it = p.into_inner();
        // pointer = { "&mut " ~ type | "&" ~ type | "*" ~ type }
        let first = it.next().unwrap();
        match first.as_rule() {
            Rule::r#type => {
                // This happens when grammar flattens; fall back
                Type::Pointer { is_mutable: false, pointee: Box::new(parse_type(first)) }
            }
            _ => {
                let text = first.as_str();
                let ty_pair = it.next().unwrap_or_else(|| panic!("pointer missing type"));
                if text.starts_with("&mut") {
                    Type::Pointer { is_mutable: true, pointee: Box::new(parse_type(ty_pair)) }
                } else if text.starts_with('&') {
                    Type::Pointer { is_mutable: false, pointee: Box::new(parse_type(ty_pair)) }
                } else if text.starts_with('*') {
                    Type::RawPointer { pointee: Box::new(parse_type(ty_pair)) }
                } else {
                    Type::Pointer { is_mutable: false, pointee: Box::new(parse_type(ty_pair)) }
                }
            }
        }
    }

    fn parse_optional(p: pest::iterators::Pair<Rule>) -> Type {
        let inner_ty = p.into_inner().next().map(parse_type).unwrap();
        Type::Optional { inner: Box::new(inner_ty) }
    }

    fn parse_result(p: pest::iterators::Pair<Rule>) -> Type {
        let inner_ty = p.into_inner().next().map(parse_type).unwrap();
        Type::Result { inner: Box::new(inner_ty) }
    }

    fn parse_matrix_type(p: pest::iterators::Pair<Rule>) -> Type {
        let mut it = p.into_inner();
        let elem_ty = parse_type(it.next().unwrap());
        let mut dims: Vec<usize> = Vec::new();
        for dim in it {
            match dim.as_rule() {
                Rule::integer => {
                    if let Ok(v) = dim.as_str().parse::<usize>() {
                        dims.push(v);
                    }
                }
                Rule::identifier => {
                    // skip non-constant dims for now
                }
                _ => {}
            }
        }
        Type::Matrix { element_type: Box::new(elem_ty), dimensions: dims }
    }

    fn parse_function_type(p: pest::iterators::Pair<Rule>) -> Type {
        let mut params: Vec<Type> = Vec::new();
        let mut ret: Option<Type> = None;
        for inner in p.into_inner() {
            match inner.as_rule() {
                Rule::r#type => {
                    if ret.is_none() {
                        params.push(parse_type(inner));
                    } else {
                        // unexpected; ignore
                    }
                }
                Rule::return_type => {
                    let ty = inner.into_inner().next().map(parse_type).unwrap();
                    ret = Some(ty);
                }
                _ => {}
            }
        }
        let ret_ty = ret.unwrap_or(Type::None);
        Type::Function { parameters: params, return_type: Box::new(ret_ty) }
    }

    fn parse_trait_type(p: pest::iterators::Pair<Rule>) -> Type {
        let mut associated_types: Vec<String> = Vec::new();
        let mut methods: HashMap<String, Type> = HashMap::new();
        for item in p.into_inner() { // trait_assoc or trait_method directly (trait_items is silent)
            match item.as_rule() {
                Rule::trait_assoc => {
                    let mut it = item.into_inner();
                    let name = it.next().unwrap().as_str().to_string();
                    associated_types.push(name);
                }
                Rule::trait_method => {
                    let mut it = item.into_inner();
                    let name = it.next().unwrap().as_str().to_string();
                    let ty_pair = it.next().unwrap(); // function_type
                    let ty = parse_function_type(ty_pair);
                    methods.insert(name, ty);
                }
                _ => {}
            }
        }
        Type::Trait { associated_types, methods }
    }

    let src_text = pair.as_str().to_string();
    let mut it = pair.into_inner();
    if let Some(inner_pair) = it.next() {
        match inner_pair.as_rule() {
        Rule::identifier => Type::Identifier(inner_pair.as_str().to_string()),
    Rule::r#struct => parse_struct(inner_pair),
    Rule::r#enum => parse_enum(inner_pair),
        Rule::pointer => parse_pointer(inner_pair),
        Rule::optional => parse_optional(inner_pair),
        Rule::result => parse_result(inner_pair),
        Rule::matrix_type => parse_matrix_type(inner_pair),
    Rule::function_type => parse_function_type(inner_pair),
    Rule::trait_type => parse_trait_type(inner_pair),
            _ => panic!("Unexpected type rule: {:?}", inner_pair.as_rule()),
        }
    } else {
        // No inner: check for literal `none`
        if src_text == "none" { Type::None } else { panic!("Unexpected empty type pair: {:?}", src_text) }
    }
}

fn parse_block(pair: pest::iterators::Pair<Rule>) -> Vec<Statement> {
    let mut statements = Vec::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::statement => statements.push(parse_statement(inner)),
            // Allow bare expressions inside blocks to act as expression statements
            Rule::expression => statements.push(Statement::Expression(parse_expression(inner))),
            _ => {}
        }
    }
    statements
}

fn parse_function(pair: pest::iterators::Pair<Rule>) -> Expression {
    let is_async = false;
    let mut params: Vec<Parameter> = Vec::new();
    let mut return_ty: Option<Type> = None;
    let mut body_opt: Option<FunctionBody> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::function_params => {
                for p in inner.into_inner() {
                    match p.as_rule() {
                        Rule::function_param => {
                            let mut it = p.into_inner();
                            let name = it.next().unwrap().as_str().to_string();
                            let mut ty: Option<Type> = None;
                            let mut default_value: Option<Expression> = None;
                            for part in it {
                                match part.as_rule() {
                                    Rule::r#type => ty = Some(parse_type(part)),
                                    Rule::expression => default_value = Some(parse_expression(part)),
                                    _ => {}
                                }
                            }
                            params.push(Parameter { name, param_type: ty, default_value });
                        }
                        _ => { /* self variants ignored for now */ }
                    }
                }
            }
            Rule::return_type => {
                let ty_pair = inner.into_inner().next().unwrap();
                return_ty = Some(parse_type(ty_pair));
            }
            Rule::function_body => {
                if let Some(first) = inner.into_inner().next() {
                    match first.as_rule() {
                        Rule::expression => body_opt = Some(FunctionBody::Expression(Box::new(parse_expression(first)))),
                        Rule::block => body_opt = Some(FunctionBody::Block(parse_block(first))),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Expression::Function { is_async, parameters: params, return_type: return_ty, body: body_opt.unwrap_or(FunctionBody::Block(Vec::new())) }
}

fn parse_if_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    // Minimal: parse condition and ignore branches structure by flattening to Block
    let mut condition: Option<Expression> = None;
    let mut then_stmts: Vec<Statement> = Vec::new();
    let mut else_stmts: Option<Vec<Statement>> = None;
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::expression => condition = Some(parse_expression(inner)),
            Rule::block => {
                if then_stmts.is_empty() {
                    then_stmts = parse_block(inner);
                } else {
                    else_stmts = Some(parse_block(inner));
                }
            }
            _ => {}
        }
    }
    Expression::If {
        condition: Box::new(condition.unwrap_or(Expression::Literal(Literal::Boolean(false)))),
        then_branch: then_stmts,
        else_branch: else_stmts,
    }
}

fn parse_return_statement(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut expr: Option<Expression> = None;
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::expression { expr = Some(parse_expression(inner)); }
    }
    Statement::Return(expr)
}

fn parse_break_statement(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut expr: Option<Expression> = None;
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::expression { expr = Some(parse_expression(inner)); }
    }
    Statement::Break(expr)
}

fn parse_for_loop(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut it = pair.into_inner();
    let var_name = it.next().unwrap().as_str().to_string();
    let mut type_annotation: Option<Type> = None;
    let mut iterable: Option<Expression> = None;
    let mut body: Vec<Statement> = Vec::new();

    while let Some(p) = it.next() {
        match p.as_rule() {
            Rule::r#type => type_annotation = Some(parse_type(p)),
            Rule::expression => iterable = Some(parse_expression(p)),
            Rule::block => body = parse_block(p),
            _ => {}
        }
    }

    Statement::ForLoop {
        variable: var_name,
        type_annotation,
        iterable: iterable.expect("for loop missing iterable expression"),
        body,
    }
}

fn parse_use_statement(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut path: Vec<String> = Vec::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier => path.push(inner.as_str().to_string()),
            _ => {}
        }
    }
    Statement::Use { path }
}

fn parse_mod_decl(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut it = pair.into_inner();
    let name = it.next().unwrap().as_str().to_string();
    // If there is a block, we will see statement items here, otherwise nothing
    let mut items: Vec<Statement> = Vec::new();
    for inner in it {
        if inner.as_rule() == Rule::statement {
            items.push(parse_statement(inner));
        }
    }
    if items.is_empty() {
        Statement::ModuleDecl { name, items: None }
    } else {
        Statement::ModuleDecl { name, items: Some(items) }
    }
}

fn parse_impl_block(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut it = pair.into_inner();
    // In grammar: impl identifier ("for" ~ identifier)? { impl_methods }
    // Pest doesn't yield the raw "for" token, so we see either:
    //  - identifier, impl_methods   (inherent impl)
    //  - identifier, identifier, impl_methods  (trait impl)
    let first_ident = it.next().expect("impl missing identifier");
    let mut trait_name: Option<String> = None;
    let type_name: String;

    // Peek next; if it's an identifier, interpret as trait impl
    let mut rest: Vec<pest::iterators::Pair<Rule>> = it.collect();
    if !rest.is_empty() && rest[0].as_rule() == Rule::identifier {
        trait_name = Some(first_ident.as_str().to_string());
        type_name = rest.remove(0).as_str().to_string();
    } else {
        type_name = first_ident.as_str().to_string();
    }

    let mut methods: Vec<Statement> = Vec::new();
    for inner in rest.into_iter() {
        match inner.as_rule() {
            Rule::const_decl => methods.push(parse_const_decl(inner)),
            // impl_methods is silent, so we don't see it here; const_decl arrives directly
            _ => {}
        }
    }
    Statement::ImplBlock { trait_name, type_name, methods }
}
