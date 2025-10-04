use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "src/parser/grammar.pest"]
struct DebugParser;

fn main() {
    let src = "enum { Red, Green: i64, Blue }";
    let mut pairs = DebugParser::parse(Rule::r#enum, src).expect("parse enum");
    let enum_pair = pairs.next().expect("enum pair");
    println!("top: {:?} -> {:?}", enum_pair.as_rule(), enum_pair.as_str());
    for inner in enum_pair.clone().into_inner() {
        println!("inner: {:?} -> {:?}", inner.as_rule(), inner.as_str());
        for sub in inner.clone().into_inner() {
            println!("  sub: {:?} -> {:?}", sub.as_rule(), sub.as_str());
        }
    }

    let program_src = r#"
        Color :: enum { Red, Green: i64, Blue }
    "#;
    let program = tricti::parser::parse(program_src.to_string());
    for stmt in program.statements {
        if let tricti::ast::Statement::ConstDecl { name, value, .. } = stmt {
            println!("const {} => {:?}", name, value);
            if let tricti::ast::ConstValue::Type(tricti::ast::Type::Enum { variants, order }) =
                value
            {
                println!("variants count {} order {:?}", variants.len(), order);
                for key in order {
                    println!("  variant {} -> {:?}", key, variants.get(&key));
                }
            }
        }
    }
}
