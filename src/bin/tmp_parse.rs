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
                    Ok(_) => println!(
                        "  statement(parse) succeeded without leading newline"
                    ),
                    Err(se) => println!(
                        "  statement parse failed (no leading newline): {}",
                        se
                    ),
                }
                match DebugParser::parse(Rule::statement, &with_leading_newline) {
                    Ok(_) => println!(
                        "  statement parse succeeded with leading newline"
                    ),
                    Err(se) => println!(
                        "  statement parse failed with leading newline: {}",
                        se
                    ),
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
    let path = env::args().nth(1).unwrap_or_else(|| "tmp/debug_if_expr.tri".to_string());
    let source = std::fs::read_to_string(&path).expect("read source");
    eprintln!("Parsing file: {}", path);
    parse_const_decl_snippet(&source);
    let snippet = "if b >= a { a } else { b }";
    match DebugParser::parse(Rule::conditional, snippet) {
        Ok(_) => eprintln!("conditional snippet parsed successfully"),
        Err(err) => eprintln!("conditional snippet failed: {}", err),
    }
    let block_snippet = "{ if b >= a { a } else { b } }";
    match DebugParser::parse(Rule::block, block_snippet) {
        Ok(mut pairs) => {
            eprintln!("block snippet parsed successfully");
            if let Some(block_pair) = pairs.next() {
                for inner in block_pair.into_inner() {
                    eprintln!("  block inner rule: {:?} -> {:?}", inner.as_rule(), inner.as_str());
                }
            }
        }
        Err(err) => eprintln!("block snippet failed: {}", err),
    }
    let program = tricti::parser::parse(source);
    println!("parsed {} statements", program.statements.len());
    for (idx, stmt) in program.statements.iter().enumerate() {
        println!("statement {}: {:#?}", idx, stmt);
    }
}
