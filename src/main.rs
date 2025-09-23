pub mod parser;
pub mod ast;
pub mod semantic;
pub mod codegen;

use inkwell::context::Context;
use std::process::Command;

fn main() {
    let file_content = std::fs::read_to_string("src.bp").expect("Failed to read source file");
    
    // Parse the program
    println!("Parsing...");
    let program = parser::parse(file_content);
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
