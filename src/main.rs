pub mod ast;
pub mod async_runtime;
pub mod async_scheduler_integration;
pub mod async_table_integration;
pub mod codegen;
pub mod computed_columns;
pub mod error_propagation;
pub mod event_loop_manager;
#[allow(unused_variables, unused_assignments)]
pub mod parser;
pub mod program_loader;
pub mod query;
pub mod query_executor;
pub mod resource_lifecycle;
pub mod scheduler;
pub mod semantic;
pub mod system_executor;
pub mod table_runtime;
#[cfg(feature = "tri-runtime")]
pub mod tri_runtime_bridge;

use crate::ast::*;
use crate::async_runtime::RuntimeConfig;
use crate::async_scheduler_integration::SystemExecutionRequest;
use crate::event_loop_manager::{EventLoopConfig, EventLoopManager, LoadBalancingStrategy};
use crate::program_loader::{parse_file_with_std, StdlibStatus};
use inkwell::context::Context;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("Usage: tricti [--no-std] [--no-run] [source_file]");
        println!("If no source_file is provided, defaults to 'src.tri'.");
        println!("Use --no-run to skip executing the generated binary.");
        return;
    }

    let mut skip_std_flag = false;
    let mut no_run_flag = false;
    let mut source_path: Option<String> = None;

    for arg in args {
        match arg.as_str() {
            "--no-std" => skip_std_flag = true,
            "--no-run" => no_run_flag = true,
            _ if arg.starts_with('-') => {
                eprintln!("Unknown option: {}", arg);
                println!("Use --help to see available options.");
                return;
            }
            _ => {
                if source_path.is_some() {
                    eprintln!("Multiple source files provided. Only one is supported.");
                    return;
                }
                source_path = Some(arg);
            }
        }
    }

    let path = source_path.unwrap_or_else(|| "src.tri".to_string());
    println!("Using source file: {}", path);
    println!("Parsing...");

    let loaded = match parse_file_with_std(Path::new(&path), skip_std_flag) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("Failed to load source file {}: {}", path, error);
            return;
        }
    };

    match loaded.stdlib_status {
        StdlibStatus::Included => println!("Standard library included."),
        StdlibStatus::SkippedEnvironment => {
            println!("Standard library skipped due to SKIP_STDLIB=1.");
        }
        StdlibStatus::SkippedFlag => {
            println!("Standard library skipped due to --no-std flag.");
        }
        StdlibStatus::SkippedAttribute => {
            println!("Standard library skipped due to @no_std attribute.");
        }
    }

    let program = loaded.program;
    println!("AST: {:#?}", program);

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

    let async_systems = extract_async_systems(&program);
    if !async_systems.is_empty() {
        println!(
            "\nDetected {} async systems - initializing async runtime...",
            async_systems.len()
        );

        if let Err(error) = run_async_systems(async_systems, semantic_context.clone()) {
            eprintln!("Async execution error: {:?}", error);
            return;
        }

        println!("Async execution completed successfully!");
    }

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

    println!("\nWriting object file...");
    if let Err(error) = codegen.write_object_file("output.o") {
        eprintln!("Failed to write object file: {}", error);
        return;
    }

    println!("Linking with clang...");
    let output = Command::new("clang")
        .args(["output.o", "-o", "output.out"])
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                println!("Successfully created executable 'output.out'");

                if !no_run_flag {
                    println!("\nRunning the program:");
                    let run_result = Command::new("./output.out").output();
                    match run_result {
                        Ok(run_output) => {
                            println!("Program output:");
                            println!("{}", String::from_utf8_lossy(&run_output.stdout));
                            if !run_output.stderr.is_empty() {
                                eprintln!(
                                    "Stderr: {}",
                                    String::from_utf8_lossy(&run_output.stderr)
                                );
                            }
                        }
                        Err(e) => eprintln!("Failed to run executable: {}", e),
                    }
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

fn run_async_systems(
    async_systems: Vec<SystemDef>,
    semantic_context: crate::semantic::SemanticContext,
) -> Result<(), Box<dyn std::error::Error>> {
    let runtime_config = RuntimeConfig {
        max_concurrent_systems: 100,
        default_task_timeout: Duration::from_secs(30),
        resource_lease_timeout: Duration::from_secs(5),
        scheduling_quantum: Duration::from_millis(10),
        enable_preemption: true,
    };

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

    let event_loop = EventLoopManager::new(runtime_config, semantic_context, event_loop_config);

    let mut execution_requests = Vec::new();
    for system_def in async_systems {
        let request = SystemExecutionRequest {
            system_def: system_def.clone(),
            parameters: HashMap::new(),
            priority: crate::async_runtime::TaskPriority::Normal,
            timeout: Some(Duration::from_secs(30)),
            table_runtimes: HashMap::new(),
        };
        execution_requests.push(request);
    }

    if !execution_requests.is_empty() {
        println!(
            "Starting event loop for {} async systems...",
            execution_requests.len()
        );

        std::thread::spawn(move || {
            if let Err(e) = event_loop.start() {
                eprintln!("Event loop error: {:?}", e);
            }
        });

        std::thread::sleep(Duration::from_millis(100));
        println!("Async systems processing initiated.");
    }

    Ok(())
}
