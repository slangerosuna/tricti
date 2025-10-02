use crate::ast::*;
use pest::*;
use pest_derive::*;
use std::collections::HashMap;

#[derive(Parser)]
#[grammar = "src/parser/grammar.pest"]
struct PnParser;

fn type_name_str(ty: &Type) -> String {
    match ty {
        Type::Identifier { name, .. } => name.clone(),
        _ => panic!("Expected identifier type for impl"),
    }
}

fn parse_type_params(pair: pest::iterators::Pair<Rule>) -> Vec<TypeParam> {
    let mut type_params = Vec::new();
    for tp in pair.into_inner() {
        if tp.as_rule() == Rule::type_param {
            let mut inner = tp.into_inner();
            let name = inner.next().unwrap().as_str().to_string();
            let mut bounds = Vec::new();

            if let Some(bounds_pair) = inner.next() {
                if bounds_pair.as_rule() == Rule::trait_bounds {
                    for bound_type in bounds_pair.into_inner() {
                        if bound_type.as_rule() == Rule::r#type {
                            bounds.push(parse_type(bound_type));
                        }
                    }
                }
            }

            type_params.push(TypeParam { name, bounds });
        }
    }
    type_params
}

fn parse_attribute(pair: pest::iterators::Pair<Rule>) -> Attribute {
    let mut name = String::new();
    let mut arguments = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier => {
                if name.is_empty() {
                    name = inner.as_str().to_string();
                } else {
                    arguments.push(Expression::Identifier(inner.as_str().to_string()));
                }
            }
            Rule::expression => {
                arguments.push(parse_expression(inner));
            }
            _ => {}
        }
    }

    Attribute { name, arguments }
}

fn parse_attributes(pair: pest::iterators::Pair<Rule>) -> Vec<Attribute> {
    let mut attributes = Vec::new();
    for attr_pair in pair.into_inner() {
        if attr_pair.as_rule() == Rule::attribute {
            attributes.push(parse_attribute(attr_pair));
        }
    }
    attributes
}

pub fn parse(file: String) -> Program {
    let successful_parse = match PnParser::parse(Rule::program, &file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Pest parse error: {}", e);
            panic!("Pest parse error (debug): {:?}", e);
        }
    };

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

    let normalized_digits: String = digits.chars().filter(|c| *c != '_').collect();
    let value = normalized_digits
        .parse::<u128>()
        .unwrap_or_else(|_| panic!("invalid integer literal: {}", text));

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
        Rule::const_statement => {
            let mut nested = inner_pair.into_inner();
            let const_decl_pair = nested
                .next()
                .expect("const_statement must contain a const_decl");
            parse_const_decl(const_decl_pair)
        }
        Rule::assignment => parse_assignment(inner_pair),
        Rule::return_statement => parse_return_statement(inner_pair),
        Rule::break_statement => parse_break_statement(inner_pair),
        Rule::for_loop => parse_for_loop(inner_pair),
        Rule::use_statement => parse_use_statement(inner_pair),
        Rule::mod_decl => parse_mod_decl(inner_pair),
        Rule::impl_block => parse_impl_block(inner_pair),
        Rule::ifdef_statement => parse_ifdef_statement(inner_pair),
        Rule::expression => Statement::Expression(parse_expression(inner_pair)),
        _ => panic!("Unexpected statement rule: {:?}", inner_pair.as_rule()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_const_declaration() {
        let src = "assert :: (cond: bool, msg: string) -> none => { ret none }";
        let mut pairs = PnParser::parse(Rule::statement, src).expect("failed to parse const statement");
        let statement_pair = pairs.next().expect("expected statement pair");
        assert_eq!(statement_pair.as_rule(), Rule::statement);
    }

    #[test]
    fn parses_stdlib_prelude_file() {
        let src = std::fs::read_to_string("stdlib/prelude.tri").expect("failed to read prelude");
        PnParser::parse(Rule::program, &src).expect("failed to parse stdlib prelude");
    }

    #[test]
    fn parses_assert_block() {
    let function_src = "(cond: bool, msg: string) -> none => { if ~cond { panic(msg) } }";
    PnParser::parse(Rule::function, function_src).expect("failed to parse function snippet");

    let statement_src = "assert :: (cond: bool, msg: string) -> none => { if ~cond { panic(msg) } }";
    PnParser::parse(Rule::statement, statement_src).expect("failed to parse assert statement snippet");

    let single_program_src = "# Assert a condition holds; otherwise panic with message\nassert :: (cond: bool, msg: string) -> none => { if ~cond { panic(msg) } }";
    PnParser::parse(Rule::program, single_program_src).expect("failed to parse single assert program");

    let two_simple_consts = "foo :: () -> none => {}\nbar :: () -> none => {}";
    PnParser::parse(Rule::program, two_simple_consts).expect("failed to parse two simple const declarations");

    let blank_line_between_consts = "foo :: () -> none => {}\n\nbar :: () -> none => {}";
    PnParser::parse(Rule::program, blank_line_between_consts).expect("failed to parse const declarations separated by blank line");

    let comment_between_consts = r"foo :: () -> none => {}
# a helpful message
bar :: () -> none => {}
";
    PnParser::parse(Rule::program, comment_between_consts).expect("failed to parse const declarations separated by comment");

    let panic_only_const = r#"panic :: (msg: string) -> none => {
    println("Assertion failed:", msg)
    exit(1)
}
"#;
    PnParser::parse(Rule::program, panic_only_const).expect("failed to parse panic-only const declaration");

    let two_nontrivial_consts = r#"foo :: () -> none => {
    bar()
}
bar :: () -> none => {
    ret none
}
"#;
    PnParser::parse(Rule::program, two_nontrivial_consts).expect("failed to parse consecutive nontrivial const declarations");

        let src = r#"
panic :: (msg: string) -> none => {
    println("Assertion failed:", msg)
    exit(1)
}
assert :: (cond: bool, msg: string) -> none => {
    if ~cond { panic(msg) }
}
"#;

        let mut statement_pairs = PnParser::parse(Rule::statement, src)
            .expect("failed to parse statement with consecutive const declarations");
        let statement_pair = statement_pairs
            .next()
            .expect("expected at least one statement pair");
        assert_ne!(
            statement_pair.as_str().trim(),
            src.trim(),
            "two const declarations should not be parsed as a single statement",
        );
        PnParser::parse(Rule::program, src).expect("failed to parse assert block snippet");
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
    let mut attributes: Vec<Attribute> = Vec::new();
    let mut first = inner
        .next()
        .expect("const declaration must have a name or attribute");
    while first.as_rule() == Rule::attributes {
        attributes.extend(parse_attributes(first));
        first = inner
            .next()
            .expect("const declaration must have a name after attributes");
    }
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

    let mut type_params = Vec::new();
    let mut type_annotation = None;
    let mut value: Option<ConstValue> = None;

    for pair in inner {
        match pair.as_rule() {
            Rule::type_params => {
                type_params = parse_type_params(pair);
            }
            Rule::r#type => {
                if type_annotation.is_none() && value.is_none() {
                    type_annotation = Some(parse_type(pair));
                } else {
                    value = Some(ConstValue::Type(parse_type(pair)));
                }
            }
            Rule::const_body => {
                let mut body_inner = pair.into_inner();
                let mut body_pair = body_inner
                    .next()
                    .expect("const body should contain a value");

                if body_pair.as_rule() == Rule::attributes {
                    attributes.extend(parse_attributes(body_pair));
                    body_pair = body_inner
                        .next()
                        .expect("const body attribute should be followed by a value");
                }

                let body_pair = if body_pair.as_rule() == Rule::const_body_inner {
                    let mut inner = body_pair.into_inner();
                    inner
                        .next()
                        .expect("const body inner should contain a value")
                } else {
                    body_pair
                };
                let const_value = match body_pair.as_rule() {
                    Rule::r#type => ConstValue::Type(parse_type(body_pair)),
                    Rule::expression => ConstValue::Expression(parse_expression(body_pair)),
                    Rule::function => ConstValue::Expression(parse_function(body_pair)),
                    Rule::table_def => ConstValue::TableDef(parse_table_def(body_pair, &name)),
                    Rule::sys_def => ConstValue::SystemDef(parse_sys_def(body_pair, &name)),
                    Rule::compose_def => ConstValue::ComposeDef(parse_compose_def(body_pair)),
                    Rule::db_def => ConstValue::DatabaseDef(parse_db_def(body_pair)),
                    Rule::type_assign => {
                        let mut assign_inner = body_pair.into_inner();
                        let ty_pair = assign_inner
                            .next()
                            .expect("type assignment requires a type");
                        let expr_pair = assign_inner
                            .next()
                            .expect("type assignment requires an expression");
                        type_annotation = Some(parse_type(ty_pair));
                        ConstValue::Expression(parse_expression(expr_pair))
                    }
                    _ => panic!("Unexpected const body rule: {:?}", body_pair.as_rule()),
                };
                value = Some(const_value);
            }
            Rule::expression => {
                value = Some(ConstValue::Expression(parse_expression(pair)));
            }
            Rule::table_def => {
                value = Some(ConstValue::TableDef(parse_table_def(pair, &name)));
            }
            Rule::sys_def => {
                value = Some(ConstValue::SystemDef(parse_sys_def(pair, &name)));
            }
            Rule::compose_def => {
                value = Some(ConstValue::ComposeDef(parse_compose_def(pair)));
            }
            Rule::db_def => {
                value = Some(ConstValue::DatabaseDef(parse_db_def(pair)));
            }
            _ => {}
        }
    }

    let value = value.expect("Expected const value");

    Statement::ConstDecl {
        attributes,
        name,
        type_params,
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
        Rule::shift => parse_binary_expression(pair),
        Rule::addition => parse_binary_expression(pair),
        Rule::multiplication => parse_binary_expression(pair),
        Rule::with_range => parse_with_range(pair),
        Rule::unary => parse_unary(pair),
        Rule::cast_expr => parse_cast_expression(pair),
        Rule::post_fix => parse_postfix_expression(pair),
        Rule::primary => parse_primary_expression(pair),
        Rule::array_new => parse_array_new(pair),
        Rule::function => parse_function(pair),
        Rule::literal => parse_literal(pair),
        Rule::identifier => Expression::Identifier(pair.as_str().to_string()),
        Rule::number => parse_number(pair),
        Rule::integer => Expression::Literal(parse_integer_literal(pair)),
        Rule::float => Expression::Literal(Literal::Float(pair.as_str().parse().unwrap())),
        Rule::string => {
            let content = pair.as_str();
            let unquoted = &content[1..content.len() - 1]; // Remove quotes
            Expression::Literal(Literal::String(unquoted.to_string()))
        }
        Rule::char => {
            let content = pair.as_str();
            let ch = parse_char_literal(content);
            Expression::Literal(Literal::Char(ch))
        }
        Rule::boolean => Expression::Literal(Literal::Boolean(pair.as_str() == "true")),
        Rule::query_spec => Expression::Query(parse_query_spec(pair)),
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
    } else {
        None
    };
    Expression::Range {
        start: Box::new(start),
        end: Box::new(end),
        step,
    }
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
            Rule::cast_expr => {
                return parse_cast_expression(p);
            }
            Rule::op_unary => {
                let op_str = p.as_str();
                let operand_pair = it.next().expect("unary missing operand");
                let operand_expr = match operand_pair.as_rule() {
                    Rule::post_fix => parse_postfix_expression(operand_pair),
                    Rule::cast_expr => parse_cast_expression(operand_pair),
                    _ => parse_expression(operand_pair),
                };
                if op_str == "some" {
                    return Expression::Call {
                        function: Box::new(Expression::Identifier("some".to_string())),
                        type_args: vec![],
                        arguments: vec![Argument {
                            name: None,
                            value: operand_expr,
                        }],
                    };
                }
                let op = match op_str {
                    "-" => UnaryOperator::Negate,
                    "!" => UnaryOperator::Not,
                    "~" => UnaryOperator::BitwiseNot,
                    "*" => UnaryOperator::Deref,
                    s if s.starts_with("&mut") => UnaryOperator::MutAddressOf,
                    "&" => UnaryOperator::AddressOf,
                    _ => UnaryOperator::Not,
                };
                return Expression::UnaryOp {
                    operator: op,
                    operand: Box::new(operand_expr),
                };
            }
            _ => {}
        }
    }
    // Fallback: just parse as expression
    if let Some(p) = first {
        parse_expression(p)
    } else {
        Expression::Literal(Literal::integer_zero())
    }
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
            "<<" => BinaryOperator::ShiftLeft,
            ">>" => BinaryOperator::ShiftRight,
            "and" => BinaryOperator::And,
            "or" => BinaryOperator::Or,
            "xor" => BinaryOperator::Xor,
            "&&" => BinaryOperator::LogicalAnd,
            "||" => BinaryOperator::LogicalOr,
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
                let mut type_args = Vec::new();
                let mut arguments = Vec::new();
                for arg_pair in suffix_pair.into_inner() {
                    match arg_pair.as_rule() {
                        Rule::type_args => {
                            for type_pair in arg_pair.into_inner() {
                                type_args.push(parse_type(type_pair));
                            }
                        }
                        Rule::argument => {
                            arguments.push(parse_argument(arg_pair));
                        }
                        _ => {}
                    }
                }
                expr = Expression::Call {
                    function: Box::new(expr),
                    type_args,
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
                expr = Expression::FieldAccess {
                    object: Box::new(expr),
                    field: name,
                };
            }
            Rule::struct_literal => {
                let fields = parse_struct_literal_fields(suffix_pair);
                let type_name = if let Expression::Identifier(id) = &expr {
                    Some(id.clone())
                } else {
                    None // shouldn't happen
                };
                expr = Expression::StructLiteral { type_name, fields };
            }
            _ if suffix_pair.as_str().starts_with("[") => {
                // Indexing suffix: "[ expr (, expr)* ]" possibly repeated; grammar emits it as part of post_fix
                // We'll parse indices from this suffix explicitly by iterating its inner expressions
                let mut indices: Vec<Expression> = Vec::new();
                for idx in suffix_pair.into_inner() {
                    // inner pairs are expressions
                    indices.push(parse_expression(idx));
                }
                expr = Expression::Index {
                    object: Box::new(expr),
                    indices,
                };
            }
            _ => {}
        }
    }

    expr
}

fn parse_cast_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let mut inner = pair.into_inner();
    let first = inner.next().expect("cast expression missing base");
    let mut expr = match first.as_rule() {
        Rule::post_fix => parse_postfix_expression(first),
        _ => parse_expression(first),
    };

    for tail in inner {
        if tail.as_rule() == Rule::cast_tail {
            let mut tail_inner = tail.into_inner();
            let to_type = parse_type(tail_inner.next().unwrap());
            expr = Expression::Cast {
                value: Box::new(expr),
                to_type,
            };
        }
    }

    expr
}

fn parse_primary_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let inner_pair = pair.into_inner().next().unwrap();
    match inner_pair.as_rule() {
        Rule::block => Expression::Block {
            statements: parse_block(inner_pair),
        },
        Rule::unsafe_block => Expression::UnsafeBlock {
            statements: parse_block(inner_pair.into_inner().next().unwrap()),
        },
        Rule::function => parse_function(inner_pair),
        Rule::conditional => parse_if_expression(inner_pair),
        Rule::inline_conditional => parse_inline_if_expression(inner_pair),
        Rule::r#match => parse_match_expression(inner_pair),
        Rule::matrix => parse_matrix(inner_pair),
        Rule::tuple_expr => parse_tuple_expression(inner_pair),
        Rule::path_struct => parse_path_struct_expression(inner_pair),
        Rule::static_path => parse_static_path_expression(inner_pair),
        Rule::primary_struct => {
            let mut it = inner_pair.into_inner();
            let id = it.next().unwrap().as_str().to_string();
            // optional type_args consumed here but ignored for now
            let mut maybe_type_args = None;
            if let Some(next) = it.next() {
                if next.as_rule() == Rule::type_args {
                    maybe_type_args = Some(next);
                } else {
                    // it's the struct_literal
                    let fields = parse_struct_literal_fields(next);
                    return Expression::StructLiteral {
                        type_name: Some(id),
                        fields,
                    };
                }
            }
            // if we got here, last inner is struct_literal
            let struct_pair = it.next().unwrap();
            let fields = parse_struct_literal_fields(struct_pair);
            Expression::StructLiteral {
                type_name: Some(id),
                fields,
            }
        }
        _ => parse_expression(inner_pair),
    }
}

fn parse_path_struct_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let mut inner = pair.into_inner();
    let mut path_parts = Vec::new();

    while let Some(part) = inner.next() {
        match part.as_rule() {
            Rule::identifier => path_parts.push(part.as_str().to_string()),
            Rule::struct_literal => {
                let fields = parse_struct_literal_fields(part);
                let mangled_name = path_parts.join("_");
                return Expression::StructLiteral {
                    type_name: Some(mangled_name),
                    fields,
                };
            }
            _ => panic!("Unexpected path struct part: {:?}", part.as_rule()),
        }
    }
    panic!("Path struct missing struct literal");
}

fn parse_static_path_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let mut inner = pair.into_inner();
    let mut segments = Vec::new();
    let mut type_args = Vec::new();

    while let Some(part) = inner.next() {
        match part.as_rule() {
            Rule::identifier => segments.push(part.as_str().to_string()),
            Rule::type_args => {
                for type_pair in part.into_inner() {
                    type_args.push(parse_type(type_pair));
                }
            }
            _ => {}
        }
    }

    Expression::StaticPath {
        segments,
        type_args,
    }
}

fn parse_array_new(pair: pest::iterators::Pair<Rule>) -> Expression {
    let mut inner = pair.into_inner();
    let element_type_pair = inner
        .next()
        .unwrap_or_else(|| panic!("array new missing element type"));
    let element_type = parse_type(element_type_pair);
    let mut dimensions: Vec<Expression> = Vec::new();
    for dim in inner {
        if dim.as_rule() == Rule::expression {
            dimensions.push(parse_expression(dim));
        }
    }
    Expression::ArrayNew {
        element_type,
        dimensions,
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

fn parse_tuple_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let elements: Vec<Expression> = pair
        .into_inner()
        .filter(|inner| inner.as_rule() == Rule::expression)
        .map(parse_expression)
        .collect();
    Expression::Tuple(elements)
}

fn parse_match_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let mut it = pair.into_inner();
    // First inner expression is the scrutinee
    let value_pair = it.next().expect("match missing value expression");
    let value = parse_expression(value_pair);
    let mut arms: Vec<MatchArm> = Vec::new();
    for arm_pair in it {
        if arm_pair.as_rule() != Rule::match_arm {
            continue;
        }
        let mut arm_inner = arm_pair.into_inner();
        let pattern_pair = arm_inner
            .next()
            .expect("match arm missing pattern expression");
        let body_pair = arm_inner.next().expect("match arm missing body expression");
        let pattern = parse_pattern(pattern_pair);
        let body = parse_match_arm_body(body_pair);
        arms.push(MatchArm { pattern, body });
    }
    Expression::Match {
        value: Box::new(value),
        arms,
    }
}

fn parse_pattern(pair: pest::iterators::Pair<Rule>) -> Expression {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::path_struct => parse_path_struct_expression(inner),
        Rule::static_path => parse_static_path_expression(inner),
        Rule::identifier => Expression::Identifier(inner.as_str().to_string()),
        Rule::option_pattern => parse_option_pattern(inner),
        _ => panic!("Unexpected pattern rule: {:?}", inner.as_rule()),
    }
}

fn parse_option_pattern(pair: pest::iterators::Pair<Rule>) -> Expression {
    let mut inner = pair.into_inner();
    let variant = inner
        .next()
        .expect("option pattern missing variant");

    match variant.as_rule() {
        Rule::some_option_pattern => {
            let mut some_inner = variant.into_inner();
            let value_pair = some_inner
                .next()
                .expect("some pattern must have an inner pattern");
            let argument_pattern = parse_pattern(value_pair);
            Expression::Call {
                function: Box::new(Expression::Identifier("some".to_string())),
                type_args: vec![],
                arguments: vec![Argument {
                    name: None,
                    value: argument_pattern,
                }],
            }
        }
        Rule::none_option_pattern => Expression::Identifier("none".to_string()),
        other => panic!("unexpected option pattern variant: {:?}", other),
    }
}

fn parse_match_arm_body(pair: pest::iterators::Pair<Rule>) -> Expression {
    let mut inner = pair.into_inner();
    let body_pair = inner
        .next()
        .expect("match arm body missing expression or block");
    match body_pair.as_rule() {
        Rule::block => Expression::Block {
            statements: parse_block(body_pair),
        },
        Rule::return_statement => {
            let stmt = parse_return_statement(body_pair);
            Expression::Block {
                statements: vec![stmt],
            }
        }
        Rule::break_statement => {
            let stmt = parse_break_statement(body_pair);
            Expression::Block {
                statements: vec![stmt],
            }
        }
        Rule::expression => parse_expression(body_pair),
        _ => parse_expression(body_pair),
    }
}

fn parse_struct_literal_fields(pair: pest::iterators::Pair<Rule>) -> HashMap<String, Expression> {
    let mut fields: HashMap<String, Expression> = HashMap::new();
    for field_pair in pair.into_inner() {
        match field_pair.as_rule() {
            Rule::shorthand_field => {
                let name = field_pair.into_inner().next().unwrap().as_str().to_string();
                fields.insert(name.clone(), Expression::Identifier(name));
            }
            Rule::full_field => {
                let mut inner = field_pair.into_inner();
                let name = inner.next().unwrap().as_str().to_string();
                let expr_pair = inner.next().expect("full_field missing expression");
                let expr = parse_expression(expr_pair);
                fields.insert(name, expr);
            }
            _ => {
                // skip unknown tokens such as the ".." wildcard or commas
            }
        }
    }
    fields
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
                let name_pair = match it.next() {
                    Some(p) => p,
                    None => break,
                };
                if name_pair.as_rule() != Rule::identifier {
                    break;
                }
                let name = name_pair.as_str().to_string();
                // Expect a ':' then an expression; grammar yields the expression directly
                let expr_pair = it.next().expect("struct literal missing field expression");
                let value_expr = parse_expression(expr_pair);
                fields.insert(name, value_expr);
            }
            Expression::StructLiteral {
                type_name: None,
                fields,
            }
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

fn parse_tuple_type(pair: pest::iterators::Pair<Rule>) -> Type {
    let elements: Vec<Type> = pair
        .into_inner()
        .filter(|inner| inner.as_rule() == Rule::r#type)
        .map(parse_type)
        .collect();
    Type::Tuple(elements)
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
                Type::Pointer {
                    is_mutable: false,
                    pointee: Box::new(parse_type(first)),
                }
            }
            _ => {
                let text = first.as_str();
                let ty_pair = it.next().unwrap_or_else(|| panic!("pointer missing type"));
                if text.starts_with("&mut") {
                    Type::Pointer {
                        is_mutable: true,
                        pointee: Box::new(parse_type(ty_pair)),
                    }
                } else if text.starts_with('&') {
                    Type::Pointer {
                        is_mutable: false,
                        pointee: Box::new(parse_type(ty_pair)),
                    }
                } else if text.starts_with('*') {
                    Type::RawPointer {
                        pointee: Box::new(parse_type(ty_pair)),
                    }
                } else {
                    Type::Pointer {
                        is_mutable: false,
                        pointee: Box::new(parse_type(ty_pair)),
                    }
                }
            }
        }
    }

    fn parse_optional(p: pest::iterators::Pair<Rule>) -> Type {
        let inner_ty = p.into_inner().next().map(parse_type).unwrap();
        Type::Optional {
            inner: Box::new(inner_ty),
        }
    }

    fn parse_result(p: pest::iterators::Pair<Rule>) -> Type {
        let inner_ty = p.into_inner().next().map(parse_type).unwrap();
        Type::Result {
            inner: Box::new(inner_ty),
        }
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
        Type::Matrix {
            element_type: Box::new(elem_ty),
            dimensions: dims,
        }
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
        Type::Function {
            parameters: params,
            return_type: Box::new(ret_ty),
        }
    }

    fn parse_trait_type(p: pest::iterators::Pair<Rule>) -> Type {
        let mut associated_types: Vec<String> = Vec::new();
        let mut methods: HashMap<String, Type> = HashMap::new();
        for item in p.into_inner() {
            // trait_assoc or trait_method directly (trait_items is silent)
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
        Type::Trait {
            associated_types,
            methods,
        }
    }

    let src_text = pair.as_str().to_string();
    let mut it = pair.into_inner();
    if let Some(inner_pair) = it.next() {
        match inner_pair.as_rule() {
            Rule::identifier => {
                let name = inner_pair.as_str().to_string();
                let mut type_args = vec![];
                // Check if next is type_args
                if let Some(next) = it.next() {
                    if next.as_rule() == Rule::type_args {
                        for ta in next.into_inner() {
                            if ta.as_rule() == Rule::r#type {
                                type_args.push(parse_type(ta));
                            }
                        }
                    }
                }
                Type::Identifier { name, type_args }
            }
            Rule::r#struct => parse_struct(inner_pair),
            Rule::r#enum => parse_enum(inner_pair),
            Rule::pointer => parse_pointer(inner_pair),
            Rule::reference_type => parse_reference_type(inner_pair),
            Rule::optional => parse_optional(inner_pair),
            Rule::result => parse_result(inner_pair),
            Rule::matrix_type => parse_matrix_type(inner_pair),
            Rule::function_type => parse_function_type(inner_pair),
            Rule::trait_type => parse_trait_type(inner_pair),
            Rule::tuple_type => parse_tuple_type(inner_pair),
            _ => panic!("Unexpected type rule: {:?}", inner_pair.as_rule()),
        }
    } else {
        // No inner: check for literal `none`
        if src_text == "none" {
            Type::None
        } else {
            panic!("Unexpected empty type pair: {:?}", src_text)
        }
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
    let mut is_async = false;
    let mut params: Vec<Parameter> = Vec::new();
    let mut return_ty: Option<Type> = None;
    let mut body_opt: Option<FunctionBody> = None;
    let mut type_params: Vec<TypeParam> = Vec::new();
    let mut attributes: Vec<Attribute> = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::attributes => {
                attributes.extend(parse_attributes(inner));
            }
            Rule::type_params => {
                type_params = parse_type_params(inner);
            }
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
                                    Rule::expression => {
                                        default_value = Some(parse_expression(part))
                                    }
                                    _ => {}
                                }
                            }
                            params.push(Parameter {
                                name,
                                param_type: ty,
                                default_value,
                            });
                        }
                        _ => { /* self variants ignored for now */ }
                    }
                }
            }
            Rule::return_type => {
                let ty_pair = inner.into_inner().next().unwrap();
                return_ty = Some(parse_type(ty_pair));
            }
            Rule::identifier if inner.as_str() == "async" => {
                is_async = true;
            }
            Rule::function_body => {
                if let Some(first) = inner.into_inner().next() {
                    match first.as_rule() {
                        Rule::expression => {
                            body_opt =
                                Some(FunctionBody::Expression(Box::new(parse_expression(first))))
                        }
                        Rule::block => body_opt = Some(FunctionBody::Block(parse_block(first))),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Expression::Function {
        is_async,
        type_params,
        parameters: params,
        return_type: return_ty,
        body: body_opt.unwrap_or(FunctionBody::Block(Vec::new())),
        attributes,
    }
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

fn parse_inline_if_expression(pair: pest::iterators::Pair<Rule>) -> Expression {
    let mut inner = pair.into_inner();
    let condition_pair = inner
        .next()
        .expect("inline if expression missing condition");
    let then_pair = inner
        .next()
        .expect("inline if expression missing then branch");

    let condition = parse_expression(condition_pair);
    let then_expr = parse_expression(then_pair);
    let else_expr = inner.next().map(parse_expression);

    Expression::IfExpr {
        condition: Box::new(condition),
        then_expr: Box::new(then_expr),
        else_expr: else_expr.map(Box::new),
    }
}

fn parse_return_statement(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut expr: Option<Expression> = None;
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::expression {
            expr = Some(parse_expression(inner));
        }
    }
    Statement::Return(expr)
}

fn parse_break_statement(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut expr: Option<Expression> = None;
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::expression {
            expr = Some(parse_expression(inner));
        }
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
    // Check if the source string starts with "pub"
    let source = pair.as_str();
    let is_public = source.starts_with("pub ");

    let mut path: Vec<String> = Vec::new();
    let mut alias: Option<String> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier => {
                let identifier = inner.as_str().to_string();
                if identifier == "pub" {
                    // Skip the "pub" keyword
                } else if identifier == "use" {
                    // Skip the "use" keyword
                } else {
                    path.push(identifier);
                }
            }
            Rule::use_alias => {
                // Parse the use_alias rule to extract the alias identifier
                for alias_inner in inner.into_inner() {
                    if alias_inner.as_rule() == Rule::identifier {
                        alias = Some(alias_inner.as_str().to_string());
                    }
                }
            }
            _ => {
                // Skip other rules
            }
        }
    }
    Statement::Use {
        is_public,
        path,
        alias,
    }
}

fn parse_mod_decl(pair: pest::iterators::Pair<Rule>) -> Statement {
    // Check if the source string starts with "pub"
    let source = pair.as_str();
    let is_public = source.trim_start().starts_with("pub");

    let mut it = pair.into_inner();
    let mut name = String::new();
    let mut items: Vec<Statement> = Vec::new();

    for inner in it {
        match inner.as_rule() {
            Rule::identifier => {
                if name.is_empty() {
                    name = inner.as_str().to_string();
                }
            }
            Rule::statement => {
                items.push(parse_statement(inner));
            }
            _ => {}
        }
    }

    if items.is_empty() {
        Statement::ModuleDecl {
            is_public,
            name,
            items: None,
        }
    } else {
        Statement::ModuleDecl {
            is_public,
            name,
            items: Some(items),
        }
    }
}

fn parse_impl_block(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut it = pair.into_inner();
    // In grammar: impl type_params? type ("for" ~ type)? { impl_methods }
    // Parse optional type_params first
    let mut type_params: Vec<TypeParam> = Vec::new();
    let first = it.next().expect("impl missing content");

    let first_type = if first.as_rule() == Rule::type_params {
        type_params = parse_type_params(first);
        it.next().expect("impl missing type after type_params")
    } else {
        first
    };

    let mut trait_name: Option<String> = None;
    let type_name: String;

    // Collect rest
    let mut rest: Vec<pest::iterators::Pair<Rule>> = it.collect();
    if !rest.is_empty() && rest[0].as_rule() == Rule::r#type {
        trait_name = Some(type_name_str(&parse_type(first_type)));
        type_name = type_name_str(&parse_type(rest.remove(0)));
    } else {
        type_name = type_name_str(&parse_type(first_type));
    }

    let mut methods: Vec<Statement> = Vec::new();
    for inner in rest.into_iter() {
        match inner.as_rule() {
            Rule::const_decl => methods.push(parse_const_decl(inner)),
            Rule::impl_method => methods.push(parse_impl_method(inner)),
            _ => {}
        }
    }
    Statement::ImplBlock {
        type_params,
        trait_name,
        type_name,
        methods,
    }
}

fn parse_impl_method(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut params: Vec<Parameter> = Vec::new();
    let mut return_ty: Option<Type> = None;
    let mut body_opt: Option<FunctionBody> = None;
    let mut type_params: Vec<TypeParam> = Vec::new();
    let mut name = String::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier => {
                name = inner.as_str().to_string();
            }
            Rule::type_params => {
                type_params = parse_type_params(inner);
            }
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
                                    Rule::expression => {
                                        default_value = Some(parse_expression(part))
                                    }
                                    _ => {}
                                }
                            }
                            params.push(Parameter {
                                name,
                                param_type: ty,
                                default_value,
                            });
                        }
                        _ => { /* self variants ignored */ }
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
                        Rule::expression => {
                            body_opt =
                                Some(FunctionBody::Expression(Box::new(parse_expression(first))))
                        }
                        Rule::block => body_opt = Some(FunctionBody::Block(parse_block(first))),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Statement::ImplMethod {
        name,
        type_params,
        parameters: params,
        return_type: return_ty,
        body: body_opt.unwrap_or(FunctionBody::Block(Vec::new())),
    }
}

fn parse_table_def(pair: pest::iterators::Pair<Rule>, table_name: &str) -> TableDef {
    let mut columns = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::table_fields => {
                for field_pair in inner_pair.into_inner() {
                    if field_pair.as_rule() == Rule::table_field {
                        columns.push(parse_table_field(field_pair));
                    }
                }
            }
            _ => {}
        }
    }

    TableDef {
        name: table_name.to_string(),
        columns,
    }
}

fn parse_table_field(pair: pest::iterators::Pair<Rule>) -> TableColumn {
    let mut annotations = Vec::new();
    let mut field_name = String::new();
    let mut field_type = Type::None;
    let mut default_value = None;
    let mut is_computed = false;
    let mut computed_expression = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::attribute => {
                annotations.push(parse_table_annotation(inner_pair));
            }
            Rule::identifier => {
                if field_name.is_empty() {
                    field_name = inner_pair.as_str().to_string();
                }
            }
            Rule::r#type => {
                field_type = parse_type(inner_pair);
            }
            Rule::expression => {
                default_value = Some(parse_expression(inner_pair));
            }
            Rule::computed_field => {
                is_computed = true;
                // Extract the expression from computed(expression)
                for computed_inner in inner_pair.into_inner() {
                    match computed_inner.as_rule() {
                        Rule::expression => {
                            computed_expression = Some(parse_expression(computed_inner));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    TableColumn {
        name: field_name,
        column_type: field_type,
        annotations,
        default_value,
        is_computed,
        computed_expression,
    }
}

fn parse_table_annotation(pair: pest::iterators::Pair<Rule>) -> TableAnnotation {
    let mut name = String::new();
    let mut args = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::identifier => {
                name = inner_pair.as_str().to_string();
            }
            Rule::expression => {
                args.push(parse_expression(inner_pair));
            }
            _ => {}
        }
    }

    TableAnnotation { name, args }
}

// System Execution Model parsing functions
fn parse_sys_def(pair: pest::iterators::Pair<Rule>, name: &str) -> SystemDef {
    let mut is_async = pair.as_str().trim_start().starts_with("async");
    let mut parameters = Vec::new();
    let mut return_type = None;
    let mut body = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::sys_params => {
                for param_pair in inner_pair.into_inner() {
                    if param_pair.as_rule() == Rule::sys_param {
                        parameters.push(parse_system_parameter(param_pair));
                    }
                }
            }
            Rule::return_type => {
                for type_pair in inner_pair.into_inner() {
                    if type_pair.as_rule() == Rule::r#type {
                        return_type = Some(parse_type(type_pair));
                    }
                }
            }
            Rule::block => {
                for stmt_pair in inner_pair.into_inner() {
                    if stmt_pair.as_rule() == Rule::statement {
                        body.push(parse_statement(stmt_pair));
                    }
                }
            }
            Rule::identifier if inner_pair.as_str() == "async" => {
                is_async = true;
            }
            _ => {}
        }
    }

    SystemDef {
        name: name.to_string(),
        parameters,
        return_type,
        body,
        is_async,
    }
}

fn collect_attribute_names(pair: pest::iterators::Pair<Rule>) -> Vec<String> {
    let mut names = Vec::new();
    for attr_pair in pair.into_inner() {
        if attr_pair.as_rule() == Rule::attribute {
            let mut attr_inner = attr_pair.into_inner();
            if let Some(name_pair) = attr_inner.next() {
                names.push(name_pair.as_str().to_string());
            }
        }
    }
    names
}

fn parse_system_parameter(pair: pest::iterators::Pair<Rule>) -> SystemParameter {
    let mut attributes = Vec::new();
    let mut param_value: Option<pest::iterators::Pair<Rule>> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::attributes => {
                attributes.extend(collect_attribute_names(inner));
            }
            Rule::query_param
            | Rule::resource_param
            | Rule::regular_param
            | Rule::function_param => {
                param_value = Some(inner);
            }
            _ => {}
        }
    }

    let value_pair = param_value.expect("system parameter should contain a value");

    let mut parameter = match value_pair.as_rule() {
        Rule::query_param => parse_query_parameter(value_pair),
        Rule::resource_param => parse_resource_parameter(value_pair),
        Rule::regular_param => parse_regular_parameter(value_pair),
        Rule::function_param => {
            let param = parse_function_param(value_pair);
            SystemParameter::Regular {
                param_type: "value".to_string(),
                name: param.name,
                value_type: param.param_type.unwrap_or(Type::None),
                default_value: param.default_value,
            }
        }
        _ => SystemParameter::Regular {
            param_type: "value".to_string(),
            name: value_pair.as_str().to_string(),
            value_type: Type::None,
            default_value: None,
        },
    };

    if !attributes.is_empty() {
        let attr = attributes.last().cloned().unwrap_or_default();
        parameter = match parameter {
            SystemParameter::Regular {
                param_type: _,
                name,
                value_type,
                default_value,
            } => SystemParameter::Regular {
                param_type: attr,
                name,
                value_type,
                default_value,
            },
            other => other,
        };
    }

    parameter
}

fn parse_query_parameter(pair: pest::iterators::Pair<Rule>) -> SystemParameter {
    let mut name: Option<String> = None;
    let mut query_spec: Option<QuerySpec> = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::identifier => {
                if name.is_none() {
                    name = Some(inner_pair.as_str().to_string());
                }
            }
            Rule::query_spec => {
                query_spec = Some(parse_query_spec(inner_pair));
            }
            _ => {}
        }
    }

    SystemParameter::Query {
        name: name.unwrap_or_else(|| "query".to_string()),
        query_spec: query_spec.expect("query parameter requires a query spec"),
    }
}

fn parse_resource_parameter(pair: pest::iterators::Pair<Rule>) -> SystemParameter {
    let mut name = String::new();
    let mut access = ResourceAccess::Owned;
    let mut resource_type: Option<Type> = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::identifier => {
                if name.is_empty() {
                    name = inner_pair.as_str().to_string();
                }
            }
            Rule::resource_ref => {
                access = parse_resource_access(inner_pair);
            }
            Rule::r#type => {
                resource_type = Some(parse_type(inner_pair));
            }
            _ => {}
        }
    }

    SystemParameter::Resource {
        param_type: "resource".to_string(),
        name,
        resource_type: resource_type.expect("resource parameter requires a type"),
        access,
    }
}

fn parse_regular_parameter(pair: pest::iterators::Pair<Rule>) -> SystemParameter {
    let mut name = String::new();
    let mut value_type: Option<Type> = None;
    let mut default_value: Option<Expression> = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::identifier => {
                if name.is_empty() {
                    name = inner_pair.as_str().to_string();
                }
            }
            Rule::r#type => {
                value_type = Some(parse_type(inner_pair));
            }
            Rule::expression => {
                default_value = Some(parse_expression(inner_pair));
            }
            _ => {}
        }
    }

    SystemParameter::Regular {
        param_type: "value".to_string(),
        name,
        value_type: value_type.unwrap_or(Type::None),
        default_value,
    }
}

fn parse_query_spec(pair: pest::iterators::Pair<Rule>) -> QuerySpec {
    let mut projections = Vec::new();
    let mut from_table = String::new();
    let mut where_clause = None;
    let mut joins = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::field_projections => {
                projections.extend(parse_field_projection_container(inner_pair));
            }
            Rule::identifier => {
                from_table = inner_pair.as_str().to_string();
            }
            Rule::join_clauses => {
                for join_pair in inner_pair.into_inner() {
                    if join_pair.as_rule() == Rule::join_clause {
                        joins.push(parse_join_clause(join_pair));
                    }
                }
            }
            Rule::expression => {
                where_clause = Some(Box::new(parse_expression(inner_pair)));
            }
            _ => {}
        }
    }

    QuerySpec {
        projections,
        from_table,
        where_clause,
        joins,
    }
}

fn parse_field_projection(pair: pest::iterators::Pair<Rule>) -> FieldProjection {
    let mut name = String::new();
    let mut field_type = None;
    let mut access = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::identifier => {
                name = inner_pair.as_str().to_string();
            }
            Rule::resource_ref => {
                access = Some(parse_resource_access(inner_pair));
            }
            Rule::r#type => {
                field_type = Some(parse_type(inner_pair));
            }
            _ => {}
        }
    }

    if let (Some(access_mode), Some(inner_type)) = (access.clone(), field_type.take()) {
        let wrapped_type = match access_mode {
            ResourceAccess::Mutable => Type::Reference {
                is_mutable: true,
                inner: Box::new(inner_type),
            },
            ResourceAccess::Immutable => Type::Reference {
                is_mutable: false,
                inner: Box::new(inner_type),
            },
            ResourceAccess::Owned => inner_type,
        };
        field_type = Some(wrapped_type);
    }

    FieldProjection {
        name,
        field_type,
        access,
    }
}

fn parse_field_projection_container(pair: pest::iterators::Pair<Rule>) -> Vec<FieldProjection> {
    match pair.as_rule() {
        Rule::field_projections | Rule::field_projection_group | Rule::field_projection_list => {
            let mut projections = Vec::new();
            for inner in pair.into_inner() {
                projections.extend(parse_field_projection_container(inner));
            }
            projections
        }
        Rule::field_projection => vec![parse_field_projection(pair)],
        _ => Vec::new(),
    }
}

fn parse_resource_access(pair: pest::iterators::Pair<Rule>) -> ResourceAccess {
    match pair.as_str() {
        "&mut" => ResourceAccess::Mutable,
        "&" => ResourceAccess::Immutable,
        _ => ResourceAccess::Owned,
    }
}

fn parse_join_clause(pair: pest::iterators::Pair<Rule>) -> JoinClause {
    let mut join_type = JoinType::Inner;
    let mut table = String::new();
    let mut condition = Box::new(Expression::Literal(Literal::Boolean(true)));

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::join_type => {
                join_type = parse_join_type(inner_pair);
            }
            Rule::identifier => {
                table = inner_pair.as_str().to_string();
            }
            Rule::expression => {
                condition = Box::new(parse_expression(inner_pair));
            }
            _ => {}
        }
    }

    JoinClause {
        join_type,
        table,
        condition,
    }
}

fn parse_join_type(pair: pest::iterators::Pair<Rule>) -> JoinType {
    match pair.as_str() {
        "inner" => JoinType::Inner,
        "left" => JoinType::Left,
        "right" => JoinType::Right,
        "full" => JoinType::Full,
        _ => JoinType::Inner,
    }
}

fn parse_compose_def(pair: pest::iterators::Pair<Rule>) -> ComposeDef {
    let mut entries = Vec::new();

    for inner_pair in pair.into_inner() {
        if inner_pair.as_rule() == Rule::compose_entry {
            entries.push(parse_compose_entry(inner_pair));
        }
    }

    ComposeDef { entries }
}

fn parse_compose_entry(pair: pest::iterators::Pair<Rule>) -> ComposeEntry {
    let mut source = ComposeNode::Single(String::new());
    let mut targets = Vec::new();
    let mut is_first = true;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::identifier => {
                let node = ComposeNode::Single(inner_pair.as_str().to_string());
                if is_first {
                    source = node;
                    is_first = false;
                } else {
                    targets.push(node);
                }
            }
            Rule::tuple_chain => {
                let node = parse_tuple_chain(inner_pair);
                if is_first {
                    source = node;
                    is_first = false;
                } else {
                    targets.push(node);
                }
            }
            _ => {}
        }
    }

    ComposeEntry { source, targets }
}

fn parse_tuple_chain(pair: pest::iterators::Pair<Rule>) -> ComposeNode {
    let mut names = Vec::new();

    for inner_pair in pair.into_inner() {
        if inner_pair.as_rule() == Rule::identifier {
            names.push(inner_pair.as_str().to_string());
        }
    }

    ComposeNode::Tuple(names)
}

fn parse_db_def(pair: pest::iterators::Pair<Rule>) -> DatabaseDef {
    let mut entries = Vec::new();

    for inner_pair in pair.into_inner() {
        if inner_pair.as_rule() == Rule::db_entries {
            for entry_pair in inner_pair.into_inner() {
                if entry_pair.as_rule() == Rule::db_entry {
                    entries.push(parse_db_entry(entry_pair));
                }
            }
        }
    }

    DatabaseDef { entries }
}

fn parse_db_entry(pair: pest::iterators::Pair<Rule>) -> DatabaseEntry {
    let mut name = String::new();
    let mut table_type = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::identifier => {
                if name.is_empty() {
                    name = inner_pair.as_str().to_string();
                } else {
                    table_type = Some(inner_pair.as_str().to_string());
                }
            }
            _ => {}
        }
    }

    DatabaseEntry { name, table_type }
}

// Helper functions for type parsing
fn parse_reference_type(pair: pest::iterators::Pair<Rule>) -> Type {
    let mut it = pair.into_inner();
    // reference_type = { ("&mut" | "&") ~ type }
    let first = it.next().unwrap();
    let ty_pair = it
        .next()
        .unwrap_or_else(|| panic!("reference missing type"));

    match first.as_str() {
        "&mut" => Type::Reference {
            is_mutable: true,
            inner: Box::new(parse_type(ty_pair)),
        },
        "&" => Type::Reference {
            is_mutable: false,
            inner: Box::new(parse_type(ty_pair)),
        },
        _ => panic!("Unexpected reference type: {}", first.as_str()),
    }
}

fn parse_function_param(pair: pest::iterators::Pair<Rule>) -> Parameter {
    let mut it = pair.into_inner();
    let name = it.next().unwrap().as_str().to_string();
    let mut param_type: Option<Type> = None;
    let mut default_value: Option<Expression> = None;

    for part in it {
        match part.as_rule() {
            Rule::r#type => {
                param_type = Some(parse_type(part));
            }
            Rule::expression => {
                default_value = Some(parse_expression(part));
            }
            _ => {}
        }
    }

    Parameter {
        name,
        param_type,
        default_value,
    }
}

fn parse_ifdef_statement(pair: pest::iterators::Pair<Rule>) -> Statement {
    let mut inner = pair.into_inner();
    let condition_pair = inner.next().unwrap();
    let condition = condition_pair.as_str().to_string();
    let then_branch = parse_block(inner.next().unwrap());
    let else_branch = inner.next().map(parse_block);

    Statement::IfDef {
        condition,
        then_branch,
        else_branch,
    }
}
