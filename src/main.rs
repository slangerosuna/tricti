pub mod parser;
pub mod ast;
pub mod semantic;
pub mod codegen;

use inkwell::context::Context;
use std::process::Command;
use std::path::PathBuf;
use crate::ast::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        if args[1] == "--help" || args[1] == "-h"{
            println!("Usage: peano [source_file]");
            println!("If no source_file is provided, defaults to 'src.pn'.");
            return;
        }
    }
    let path = if args.len() > 1 { &args[1] } else { "src.pn" };
    println!("Using source file: {}", path);
    let file_content = std::fs::read_to_string(path).expect("Failed to read source file");

    // Parse the program
    println!("Parsing...");
    let mut program = parser::parse(file_content);
    // Expand external modules declared as `mod name;` by reading name.pn
    program = expand_modules(program, &std::env::current_dir().unwrap());
    println!("AST: {:#?}", program);
    
    // Perform semantic analysis
    println!("\nPerforming semantic analysis...");
    let semantic_context = match semantic::analyze_program(&program) {
        Ok(context) => {
            println!("Semantic analysis passed!");
            context
        }
        Err(error) => {
            eprintln!("Semantic error: {:?}", error);
            return;
        }
    };
    
    // Generate LLVM IR
    println!("\nGenerating LLVM IR...");
    let context = Context::create();
    let mut codegen = match codegen::CodeGenerator::new(&context, semantic_context) {
        Ok(generator) => generator,
        Err(error) => {
            eprintln!("Codegen initialization error: {}", error);
            return;
        }
    };
    
    if let Err(error) = codegen.generate_program(&program) {
        eprintln!("Code generation error: {}", error);
        return;
    }
    
    println!("Generated LLVM IR:");
    codegen.print_ir();
    
    // Write object file
    println!("\nWriting object file...");
    if let Err(error) = codegen.write_object_file("output.o") {
        eprintln!("Failed to write object file: {}", error);
        return;
    }
    
    // Link with clang to create executable
    println!("Linking with clang...");
    let output = Command::new("clang")
        .args(&["output.o", "-o", "output.out"])
        .output();
    
    match output {
        Ok(result) => {
            if result.status.success() {
                println!("Successfully created executable 'output.out'");
                
                // Run the executable
                println!("\nRunning the program:");
                let run_result = Command::new("./output.out").output();
                match run_result {
                    Ok(run_output) => {
                        println!("Program output:");
                        println!("{}", String::from_utf8_lossy(&run_output.stdout));
                        if !run_output.stderr.is_empty() {
                            eprintln!("Stderr: {}", String::from_utf8_lossy(&run_output.stderr));
                        }
                    }
                    Err(e) => eprintln!("Failed to run executable: {}", e),
                }
            } else {
                eprintln!("Linking failed:");
                eprintln!("stdout: {}", String::from_utf8_lossy(&result.stdout));
                eprintln!("stderr: {}", String::from_utf8_lossy(&result.stderr));
            }
        }
        Err(e) => eprintln!("Failed to run clang: {}", e),
    }
}

fn expand_modules(program: Program, base_dir: &std::path::Path) -> Program {
    // Collect new statements to append when we inline modules
    let mut expanded: Vec<Statement> = Vec::new();
    for stmt in program.statements.into_iter() {
        match &stmt {
            Statement::ModuleDecl { name, items } if items.is_none() => {
                // Try base_dir/name.pn then base_dir/src/name.pn
                let mut tried: Vec<PathBuf> = Vec::new();
                let p1 = base_dir.join(format!("{}.pn", name));
                tried.push(p1.clone());
                let p2 = base_dir.join("src").join(format!("{}.pn", name));
                tried.push(p2.clone());
                let content_opt = tried.into_iter().find_map(|p| std::fs::read_to_string(&p).ok());
                if let Some(content) = content_opt {
                    let mut sub = parser::parse(content);
                    // Recursively expand submodules relative to same base_dir
                    sub = expand_modules(sub, base_dir);
                    // Splice submodule items at top-level for now
                    expanded.extend(sub.statements);
                } else {
                    eprintln!("warning: module '{}' not found on disk", name);
                }
            }
            Statement::Use { path } => {
                // Simple import: try to load a module file named by the first path segment or joined path
                if !path.is_empty() {
                    let joined = path.join("/");
                    let name = &path[0];
                    let mut tried: Vec<PathBuf> = Vec::new();
                    // Try exact joined path under base, src, stdlib
                    tried.push(base_dir.join(format!("{}.pn", joined)));
                    tried.push(base_dir.join("src").join(format!("{}.pn", joined)));
                    tried.push(base_dir.join("stdlib").join(format!("{}.pn", joined)));
                    // Try single-segment name fallback
                    tried.push(base_dir.join(format!("{}.pn", name)));
                    tried.push(base_dir.join("src").join(format!("{}.pn", name)));
                    tried.push(base_dir.join("stdlib").join(format!("{}.pn", name)));
                    let content_opt = tried.into_iter().find_map(|p| std::fs::read_to_string(&p).ok());
                    if let Some(content) = content_opt {
                        let mut sub = parser::parse(content);
                        sub = expand_modules(sub, base_dir);
                        expanded.extend(sub.statements);
                    } else {
                        eprintln!("warning: use {:?} not found on disk", path);
                    }
                }
            }
            Statement::ModuleDecl { name: _, items: Some(items) } => {
                // Inline module: just keep items at top-level for now
                for s in items { expanded.push(s.clone()); }
            }
            _ => expanded.push(stmt.clone()),
        }
    }
    Program { statements: expanded }
}
