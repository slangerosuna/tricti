use pest::Parser;
use pest_derive::Parser;
use std::env;

#[derive(Parser)]
#[grammar = "src/parser/grammar.pest"]
struct DebugParser;

fn parse_const_decl_snippet(source: &str) {
    for (idx, snippet) in source.split("\n\n").enumerate() {
        let trimmed = snippet.trim();
        if trimmed.is_empty() {
            continue;
        }
        let with_leading_newline = format!("\n{}", trimmed);
        match DebugParser::parse(Rule::const_decl, trimmed) {
            Ok(_) => println!("chunk {}: const_decl parsed", idx),
            Err(e) => {
                println!("chunk {}: const_decl parse failed: {}", idx, e);
                match DebugParser::parse(Rule::statement, trimmed) {
                    Ok(_) => println!("  statement(parse) succeeded without leading newline"),
                    Err(se) => println!("  statement parse failed (no leading newline): {}", se),
                }
                match DebugParser::parse(Rule::statement, &with_leading_newline) {
                    Ok(_) => println!("  statement parse succeeded with leading newline"),
                    Err(se) => println!("  statement parse failed with leading newline: {}", se),
                }
            }
        }
        match DebugParser::parse(Rule::statement, &with_leading_newline) {
            Ok(_) => println!("chunk {}: statement parsed with leading newline", idx),
            Err(se) => println!(
                "chunk {}: statement failed with leading newline: {}",
                idx, se
            ),
        }
    }
}

fn main() {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "tmp/debug_if_expr.tri".to_string());
    let source = std::fs::read_to_string(&path).expect("read source");
    let standalone = "assert :: () -> none => {}\n";
    match DebugParser::parse(Rule::statement, standalone) {
        Ok(_) => eprintln!("standalone assert statement parsed"),
        Err(err) => eprintln!("standalone assert statement failed: {}", err),
    }
    let with_leading_newline = "\nassert :: () -> none => {}\n";
    match DebugParser::parse(Rule::statement, with_leading_newline) {
        Ok(_) => eprintln!("newline-leading assert statement parsed"),
        Err(err) => eprintln!("newline-leading assert statement failed: {}", err),
    }
    eprintln!("Parsing file: {}", path);
    match DebugParser::parse(Rule::statement, &source) {
        Ok(mut pairs) => {
            eprintln!("entire source parsed as statement");
            if let Some(pair) = pairs.next() {
                eprintln!("statement span: {:?}", pair.as_str());
            }
        }
        Err(err) => eprintln!("entire source failed as statement: {}", err),
    }
    match DebugParser::parse(Rule::const_statement, &source) {
        Ok(mut pairs) => {
            eprintln!("entire source parsed as const_statement");
            if let Some(pair) = pairs.next() {
                let span = pair.as_str().replace('\n', "\\n");
                eprintln!("const_statement span: {}", span);
                for inner in pair.clone().into_inner() {
                    let inner_span = inner.as_str().replace('\n', "\\n");
                    eprintln!("  inner {:?} -> {}", inner.as_rule(), inner_span);
                    for nested in inner.clone().into_inner() {
                        let nested_span = nested.as_str().replace('\n', "\\n");
                        eprintln!("    nested {:?} -> {}", nested.as_rule(), nested_span);
                        for deeper in nested.clone().into_inner() {
                            let deeper_span = deeper.as_str().replace('\n', "\\n");
                            eprintln!("      deeper {:?} -> {}", deeper.as_rule(), deeper_span);
                        }
                    }
                }
            }
        }
        Err(err) => eprintln!("entire source failed as const_statement: {}", err),
    }
    match DebugParser::parse(Rule::const_decl, &source) {
        Ok(mut pairs) => {
            eprintln!("entire source parsed as const_decl");
            if let Some(pair) = pairs.next() {
                let span = pair.as_str().replace('\n', "\\n");
                eprintln!("const_decl span: {}", span);
                for inner in pair.clone().into_inner() {
                    let inner_span = inner.as_str().replace('\n', "\\n");
                    eprintln!("  inner {:?} -> {}", inner.as_rule(), inner_span);
                }
            }
        }
        Err(err) => eprintln!("entire source failed as const_decl: {}", err),
    }
    if let Some(assert_idx) = source.find("assert ::") {
        let tail = &source[assert_idx..];
        match DebugParser::parse(Rule::statement, tail) {
            Ok(_) => eprintln!("tail statement parsed"),
            Err(err) => eprintln!("tail statement failed: {}", err),
        }
        match DebugParser::parse(Rule::program, tail) {
            Ok(_) => eprintln!("tail program parsed"),
            Err(err) => eprintln!("tail program failed: {}", err),
        }
    }
    parse_const_decl_snippet(&source);
    let snippet = "if b >= a { a } else { b }";
    match DebugParser::parse(Rule::keyword_if, "if ") {
        Ok(_) => eprintln!("keyword_if parsed"),
        Err(err) => eprintln!("keyword_if failed: {}", err),
    }
    match DebugParser::parse(Rule::conditional, snippet) {
        Ok(_) => eprintln!("conditional snippet parsed successfully"),
        Err(err) => eprintln!("conditional snippet failed: {}", err),
    }
    let no_else = "if b >= a { a }";
    match DebugParser::parse(Rule::conditional, no_else) {
        Ok(_) => eprintln!("conditional without else parsed successfully"),
        Err(err) => eprintln!("conditional without else failed: {}", err),
    }
    let cmp_snippet = "b >= a";
    match DebugParser::parse(Rule::comparison, cmp_snippet) {
        Ok(mut pairs) => {
            eprintln!("comparison snippet parsed successfully");
            if let Some(pair) = pairs.next() {
                eprintln!(
                    "  comparison top rule: {:?} -> {:?}",
                    pair.as_rule(),
                    pair.as_str()
                );
                for inner in pair.clone().into_inner() {
                    eprintln!("    inner: {:?} -> {:?}", inner.as_rule(), inner.as_str());
                }
            }
        }
        Err(err) => eprintln!("comparison snippet failed: {}", err),
    }
    let cmp_simple = "a > b";
    match DebugParser::parse(Rule::comparison, cmp_simple) {
        Ok(mut pairs) => {
            eprintln!("simple comparison snippet parsed successfully");
            if let Some(pair) = pairs.next() {
                eprintln!(
                    "  simple comparison top rule: {:?} -> {:?}",
                    pair.as_rule(),
                    pair.as_str()
                );
                for inner in pair.clone().into_inner() {
                    eprintln!("    inner: {:?} -> {:?}", inner.as_rule(), inner.as_str());
                }
            }
        }
        Err(err) => eprintln!("simple comparison snippet failed: {}", err),
    }
    let cond_expr = "b >= a";
    match DebugParser::parse(Rule::expression_no_block, cond_expr) {
        Ok(_) => eprintln!("condition expression parsed successfully"),
        Err(err) => eprintln!("condition expression failed: {}", err),
    }
    let cond_expr_with_block = "b >= a { a }";
    eprintln!("cond_expr_with_block raw: {:?}", cond_expr_with_block);
    match DebugParser::parse(Rule::expression_no_block, cond_expr_with_block) {
        Ok(mut pairs) => {
            eprintln!("condition expression with block parsed successfully");
            if let Some(pair) = pairs.next() {
                eprintln!(
                    "  expr_no_block top rule: {:?} -> {:?} (len {} vs input {})",
                    pair.as_rule(),
                    pair.as_str(),
                    pair.as_str().len(),
                    cond_expr_with_block.len()
                );
                for inner in pair.clone().into_inner() {
                    eprintln!("    inner: {:?} -> {:?}", inner.as_rule(), inner.as_str());
                }
            }
        }
        Err(err) => eprintln!("condition expression with block failed: {}", err),
    }
    let simple_block = "{ b }";
    match DebugParser::parse(Rule::block, simple_block) {
        Ok(_) => eprintln!("simple block parsed successfully"),
        Err(err) => eprintln!("simple block failed: {}", err),
    }
    let block_snippet = "{ if b >= a { a } else { b } }";
    match DebugParser::parse(Rule::block, block_snippet) {
        Ok(mut pairs) => {
            eprintln!("block snippet parsed successfully");
            if let Some(block_pair) = pairs.next() {
                for inner in block_pair.into_inner() {
                    eprintln!(
                        "  block inner rule: {:?} -> {:?}",
                        inner.as_rule(),
                        inner.as_str()
                    );
                }
            }
        }
        Err(err) => eprintln!("block snippet failed: {}", err),
    }
    let expr_snippet = "if a > b { a } else { b }";
    match DebugParser::parse(Rule::expression, expr_snippet) {
        Ok(_) => eprintln!("expression snippet parsed successfully"),
        Err(err) => eprintln!("expression snippet failed: {}", err),
    }
    let expr_eq_snippet = "if zero == 0 { 1 } else { zero * 2 }";
    match DebugParser::parse(Rule::expression, expr_eq_snippet) {
        Ok(_) => eprintln!("expression eq snippet parsed successfully"),
        Err(err) => eprintln!("expression eq snippet failed: {}", err),
    }
    match DebugParser::parse(Rule::r#type, "?*T") {
        Ok(mut pairs) => {
            eprintln!("type '?*T' parsed successfully");
            while let Some(pair) = pairs.next() {
                eprintln!("  type pair: {:?} -> {:?}", pair.as_rule(), pair.as_str());
                for inner in pair.clone().into_inner() {
                    eprintln!("    inner: {:?} -> {:?}", inner.as_rule(), inner.as_str());
                }
            }
        }
        Err(err) => eprintln!("type '?*T' failed: {:?}", err),
    }
    match DebugParser::parse(Rule::cast_tail, "as ?*T") {
        Ok(_) => eprintln!("cast_tail 'as ?*T' parsed successfully"),
        Err(err) => eprintln!("cast_tail 'as ?*T' failed: {:?}", err),
    }
    match DebugParser::parse(Rule::cast_tail, "as i32") {
        Ok(_) => eprintln!("cast_tail 'as i32' parsed successfully"),
        Err(err) => eprintln!("cast_tail 'as i32' failed: {:?}", err),
    }
    match DebugParser::parse(Rule::cast_expr, "x as i32") {
        Ok(_) => eprintln!("cast_expr 'x as i32' parsed successfully"),
        Err(err) => eprintln!("cast_expr 'x as i32' failed: {:?}", err),
    }
    match DebugParser::parse(Rule::cast_expr, "none as ?*T") {
        Ok(_) => eprintln!("cast_expr 'none as ?*T' parsed successfully"),
        Err(err) => eprintln!("cast_expr 'none as ?*T' failed: {:?}", err),
    }
    match DebugParser::parse(Rule::cast_expr, "none as ?*raw T") {
        Ok(_) => eprintln!("cast_expr 'none as ?*raw T' parsed successfully"),
        Err(err) => eprintln!("cast_expr 'none as ?*raw T' failed: {:?}", err),
    }
    match DebugParser::parse(Rule::statement, "x := none as ?*raw T;") {
        Ok(_) => eprintln!("statement 'x := none as ?*raw T;' parsed successfully"),
        Err(err) => eprintln!("statement 'x := none as ?*raw T;' failed: {:?}", err),
    }
    match DebugParser::parse(Rule::r#type, "?*raw T") {
        Ok(mut pairs) => {
            eprintln!("type '?*raw T' parsed successfully");
            while let Some(pair) = pairs.next() {
                eprintln!("  type pair: {:?} -> {:?}", pair.as_rule(), pair.as_str());
                for inner in pair.clone().into_inner() {
                    eprintln!("    inner: {:?} -> {:?}", inner.as_rule(), inner.as_str());
                }
            }
        }
        Err(err) => eprintln!("type '?*raw T' failed: {:?}", err),
    }
    match DebugParser::parse(Rule::cast_tail, "as ?*raw T") {
        Ok(_) => eprintln!("cast_tail 'as ?*raw T' parsed successfully"),
        Err(err) => eprintln!("cast_tail 'as ?*raw T' failed: {:?}", err),
    }
    let pointer_input = "*raw T";
    eprintln!(
        "pointer input {:?} len {} bytes {:?}",
        pointer_input,
        pointer_input.len(),
        pointer_input.as_bytes()
    );
    match DebugParser::parse(Rule::pointer, pointer_input) {
        Ok(mut pairs) => {
            eprintln!("pointer '*raw T' parsed successfully");
            while let Some(pair) = pairs.next() {
                eprintln!(
                    "  pointer pair: {:?} -> {:?}",
                    pair.as_rule(),
                    pair.as_str()
                );
                let bytes: Vec<u8> = pair.as_str().as_bytes().to_vec();
                eprintln!("    bytes: {:?}", bytes);
                for inner in pair.clone().into_inner() {
                    eprintln!("    inner: {:?} -> {:?}", inner.as_rule(), inner.as_str());
                    let inner_bytes: Vec<u8> = inner.as_str().as_bytes().to_vec();
                    eprintln!("      bytes: {:?}", inner_bytes);
                }
            }
        }
        Err(err) => eprintln!("pointer '*raw T' failed: {:?}", err),
    }
    match DebugParser::parse(Rule::program, "x := none as ?*raw T;") {
        Ok(_) => eprintln!("program snippet parsed successfully"),
        Err(err) => eprintln!("program snippet failed: {:?}", err),
    }
    let program = tricti::parser::parse(source);
    println!("parsed {} statements", program.statements.len());
    for (idx, stmt) in program.statements.iter().enumerate() {
        println!("statement {}: {:#?}", idx, stmt);
    }
}
