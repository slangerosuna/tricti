pub mod ast;
pub mod async_runtime;
pub mod async_scheduler_integration;
pub mod async_table_integration;
pub mod codegen;
pub mod computed_columns;
pub mod error_propagation;
pub mod event_loop_manager;
pub mod parser;
pub mod query;
pub mod query_executor;
pub mod resource_lifecycle;
pub mod scheduler;
pub mod semantic;
pub mod system_executor;
pub mod table_runtime;

use crate::ast::*;
use crate::async_runtime::RuntimeConfig;
use crate::async_scheduler_integration::{AsyncSystemScheduler, SystemExecutionRequest};
use crate::event_loop_manager::{EventLoopConfig, EventLoopManager, LoadBalancingStrategy};
use crate::table_runtime::{ColumnValue, TableRuntime};
use inkwell::context::Context;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        if args[1] == "--help" || args[1] == "-h" {
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

    // Check if we have async systems that need the async runtime
    let async_systems = extract_async_systems(&program);
    if !async_systems.is_empty() {
        println!(
            "\nDetected {} async systems - initializing async runtime...",
            async_systems.len()
        );

        // Run async systems through the event loop
        if let Err(error) = run_async_systems(async_systems, semantic_context.clone()) {
            eprintln!("Async execution error: {:?}", error);
            return;
        }

        println!("Async execution completed successfully!");
    }

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

/// Extract async systems from the program
fn extract_async_systems(program: &Program) -> Vec<SystemDef> {
    let mut async_systems = Vec::new();

    for statement in &program.statements {
        if let Statement::ConstDecl {
            value: ConstValue::SystemDef(system_def),
            ..
        } = statement
        {
            if system_def.is_async {
                async_systems.push(system_def.clone());
            }
        }
    }

    async_systems
}

/// Run async systems through the event loop manager
fn run_async_systems(
    async_systems: Vec<SystemDef>,
    semantic_context: crate::semantic::SemanticContext,
) -> Result<(), Box<dyn std::error::Error>> {
    // Configure the async runtime
    let runtime_config = RuntimeConfig {
        max_concurrent_systems: 100,
        default_task_timeout: Duration::from_secs(30),
        resource_lease_timeout: Duration::from_secs(5),
        scheduling_quantum: Duration::from_millis(10),
        enable_preemption: true,
    };

    // Configure the event loop
    let event_loop_config = EventLoopConfig {
        max_executor_threads: std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(4),
        event_queue_capacity: 1000,
        timer_resolution: Duration::from_millis(1),
        max_execution_time: Duration::from_secs(60),
        preemption_enabled: true,
        load_balancing_strategy: LoadBalancingStrategy::ResourceAware,
    };

    // Create the event loop manager
    let event_loop = EventLoopManager::new(runtime_config, semantic_context, event_loop_config);

    // Create system execution requests for async systems
    let mut execution_requests = Vec::new();
    for system_def in async_systems {
        let request = SystemExecutionRequest {
            system_def: system_def.clone(),
            parameters: HashMap::new(), // Default empty parameters
            priority: crate::async_runtime::TaskPriority::Normal,
            timeout: Some(Duration::from_secs(30)),
            table_runtimes: HashMap::new(), // Will be registered separately if needed
        };
        execution_requests.push(request);
    }

    // Execute the async systems
    if !execution_requests.is_empty() {
        println!(
            "Starting event loop for {} async systems...",
            execution_requests.len()
        );

        // For now, we'll start the event loop and let it run briefly
        // In a full implementation, this would be more sophisticated
        std::thread::spawn(move || {
            if let Err(e) = event_loop.start() {
                eprintln!("Event loop error: {:?}", e);
            }
        });

        // Give the event loop time to process
        std::thread::sleep(Duration::from_millis(100));
        println!("Async systems processing initiated.");
    }

    Ok(())
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
                let content_opt = tried
                    .into_iter()
                    .find_map(|p| std::fs::read_to_string(&p).ok());
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
                    let content_opt = tried
                        .into_iter()
                        .find_map(|p| std::fs::read_to_string(&p).ok());
                    if let Some(content) = content_opt {
                        let mut sub = parser::parse(content);
                        sub = expand_modules(sub, base_dir);
                        expanded.extend(sub.statements);
                    } else {
                        eprintln!("warning: use {:?} not found on disk", path);
                    }
                }
            }
            Statement::ModuleDecl {
                name: _,
                items: Some(items),
            } => {
                // Inline module: just keep items at top-level for now
                for s in items {
                    expanded.push(s.clone());
                }
            }
            _ => expanded.push(stmt.clone()),
        }
    }
    Program {
        statements: expanded,
    }
}
