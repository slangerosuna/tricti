#[test]
fn pest_enum_debug() {
    use pest::Parser;
    use tricti::parser::{PnParser, Rule};

    fn walk(pair: pest::iterators::Pair<Rule>, indent: usize) {
        let padding = "  ".repeat(indent);
        println!("{}{:?} {:?}", padding, pair.as_rule(), pair.as_str());
        for inner in pair.into_inner() {
            walk(inner, indent + 1);
        }
    }

    let src = "Color :: enum { Red, Green: i64, Blue }";
    let pairs = PnParser::parse(Rule::program, src).expect("parse");

    for pair in pairs {
        walk(pair, 0);
    }
}
