use pest::Parser;
use pest_derive::Parser;

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
            Err(se) => println!("chunk {}: statement failed with leading newline: {}", idx, se),
        }
    }
}

fn main() {
    let source = std::fs::read_to_string("tmp/tmp_assert_block.tri").unwrap();
    parse_const_decl_snippet(&source);
    let program = tricti::parser::parse(source);
    println!("parsed {} statements", program.statements.len());
}
