use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let mut args = env::args().skip(1);
    let input = args
        .next()
        .expect("usage: dump_indent <input> [output]");
    let output = args.next();

    if args.next().is_some() {
        eprintln!("usage: dump_indent <input> [output]");
        std::process::exit(1);
    }

    let src = fs::read_to_string(&input).expect("failed to read input");
    let desugared = tricti::parser::indentation::desugar_indentation(&src);

    match output {
        Some(path) if path != "-" => {
            fs::write(Path::new(&path), desugared).expect("failed to write output");
        }
        _ => {
            print!("{}", desugared);
        }
    }
}
