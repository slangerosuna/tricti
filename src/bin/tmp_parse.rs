use tricti::parser;

fn main() {
    let source = std::fs::read_to_string("tmp/tmp_assert_block.tri").unwrap();
    let program = parser::parse(source);
    println!("parsed {} statements", program.statements.len());
}
