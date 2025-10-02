use std::fs;
use tricti::parser;

fn main() {
    let source = fs::read_to_string("test_module_system.tri").unwrap();
    match std::panic::catch_unwind(|| {
        let program = parser::parse(source);
        println!("Parsed {} statements successfully", program.statements.len());
        for (i, stmt) in program.statements.iter().enumerate() {
            println!("Statement {}: {:?}", i, stmt);
        }
    }) {
        Ok(_) => println!("Parsing successful!"),
        Err(e) => println!("Parsing failed: {:?}", e),
    }
}
