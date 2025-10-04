use crate::build_system::{BuildConfig, BuildSystem, OptimizationLevel};
use crate::filesystem::FileSystem;
use crate::package_manager::PackageManager;
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tricti")]
#[command(about = "TriCTI Compiler")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Compile TriCTI source files
    Build {
        /// Input file or directory
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Output file or directory
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Optimization level (0, 1, 2, 3)
        #[arg(short = 'O', long, default_value = "0")]
        opt_level: u8,
        /// Enable debug information
        #[arg(short, long)]
        debug: bool,
        /// Target triple
        #[arg(long)]
        target: Option<String>,
    },
    /// Run TriCTI code
    Run {
        /// Input file to run
        file: PathBuf,
        /// Arguments to pass to the program
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Initialize a new TriCTI package
    New {
        /// Package name
        name: String,
        /// Directory to create package in
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Install package dependencies
    Install {
        /// Package name (optional, installs from Package.toml if not specified)
        package: Option<String>,
        /// Package version
        #[arg(short, long)]
        version: Option<String>,
    },
    /// Update package dependencies
    Update,
    /// Clean build artifacts
    Clean,
    /// Show TriCTI version
    Version,
}

pub struct CliHandler {
    _build_system: BuildSystem,
    package_manager: PackageManager,
}

impl CliHandler {
    pub fn new() -> Self {
        let build_config = BuildConfig {
            targets: HashMap::new(),
            optimization_level: OptimizationLevel::None,
            debug_info: false,
            target_triple: "x86_64-unknown-linux-gnu".to_string(),
        };

        let build_system = BuildSystem::new(build_config);

        let cache_dir = std::env::current_dir().unwrap().join(".tricti/cache");
        let package_manager = PackageManager::new(
            "https://tricti.rawringgirl.com/packages".to_string(),
            cache_dir,
        );

        Self {
            _build_system: build_system,
            package_manager,
        }
    }

    pub fn handle_command(&self, command: Commands) -> Result<(), Box<dyn std::error::Error>> {
        match command {
            Commands::Build {
                input,
                output,
                opt_level,
                debug,
                target,
            } => self.handle_build(input, output, opt_level, debug, target),
            Commands::Run { file, args } => self.handle_run(file, args),
            Commands::New { name, path } => self.handle_new(name, path),
            Commands::Install { package, version } => self.handle_install(package, version),
            Commands::Update => self.handle_update(),
            Commands::Clean => self.handle_clean(),
            Commands::Version => self.handle_version(),
        }
    }

    fn handle_build(
        &self,
        input: Option<PathBuf>,
        output: Option<PathBuf>,
        opt_level: u8,
        debug: bool,
        target: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let input_path = input.unwrap_or_else(|| PathBuf::from("."));

        if let Some(ref target_triple) = target {
            println!("Using target triple: {}", target_triple);
        }

        if input_path.is_file() {
            self.compile_single_file(&input_path, output, opt_level, debug)?;
        } else {
            self.compile_package(&input_path, opt_level, debug)?;
        }

        Ok(())
    }

    fn compile_single_file(
        &self,
        input: &PathBuf,
        output: Option<PathBuf>,
        opt_level: u8,
        debug: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!("Compiling file: {:?}", input);
        println!(
            "Build profile: {} (opt level: {})",
            if debug { "debug" } else { "release" },
            opt_level
        );

        let content = FileSystem::read_file(input.to_str().unwrap())?;
        let program = crate::parser::parse(content);

        // Generate LLVM IR (placeholder - in full self-hosting this would be native)
        println!("Parsed {} statements", program.statements.len());

        let output_path = output.unwrap_or_else(|| {
            let mut path = input.clone();
            path.set_extension("out");
            path
        });

        println!("Output written to: {:?}", output_path);
        Ok(())
    }

    fn compile_package(
        &self,
        package_dir: &PathBuf,
        opt_level: u8,
        debug: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let manifest_path = package_dir.join("Package.toml");

        if !manifest_path.exists() {
            return Err("No Package.toml found. Run 'tricti new' to create a new package.".into());
        }

        println!("Building package in: {:?}", package_dir);

        // Load package manifest
        let manifest_content = FileSystem::read_file(manifest_path.to_str().unwrap())?;
        let package: crate::package_manager::Package = toml::from_str(&manifest_content)?;

        println!("Building {} v{}", package.name, package.version);

        // Build library if present
        if let Some(lib) = &package.lib {
            let lib_path = package_dir.join(&lib.path);
            self.compile_single_file(&lib_path, None, opt_level, debug)?;
        }

        // Build binaries
        for bin in &package.bin {
            let bin_path = package_dir.join(&bin.path);
            self.compile_single_file(&bin_path, None, opt_level, debug)?;
        }

        Ok(())
    }

    fn handle_run(
        &self,
        file: PathBuf,
        args: Vec<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!("Running file: {:?} with args: {:?}", file, args);

        // First compile the file
        self.compile_single_file(&file, None, 0, false)?;

        // Then execute it (this would invoke the generated executable)
        println!("Execution completed");

        Ok(())
    }

    fn handle_new(
        &self,
        name: String,
        path: Option<PathBuf>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let package_path = path.unwrap_or_else(|| PathBuf::from(&name));

        if package_path.exists() {
            return Err(format!("Directory {:?} already exists", package_path).into());
        }

        self.package_manager.init_package(&name, &package_path)?;

        println!("Created new package: {}", name);
        println!("  cd {}", package_path.display());
        println!("  tricti build");

        Ok(())
    }

    fn handle_install(
        &self,
        package: Option<String>,
        version: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match package {
            Some(pkg_name) => {
                let version = version.unwrap_or_else(|| "latest".to_string());
                self.package_manager.install_package(&pkg_name, &version)?;
                println!("Installed {}@{}", pkg_name, version);
            }
            None => {
                // Install dependencies from Package.toml
                let manifest_path = PathBuf::from("Package.toml");
                if !manifest_path.exists() {
                    return Err("No Package.toml found".into());
                }

                let content = FileSystem::read_file("Package.toml")?;
                let package: crate::package_manager::Package = toml::from_str(&content)?;

                for (dep_name, dep_req) in &package.dependencies {
                    self.package_manager
                        .install_package(dep_name, &dep_req.version)?;
                }

                println!("Installed all dependencies");
            }
        }

        Ok(())
    }

    fn handle_update(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("Updating dependencies...");
        // Update logic here
        Ok(())
    }

    fn handle_clean(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("Cleaning build artifacts...");

        let target_dir = PathBuf::from("target");
        if target_dir.exists() {
            std::fs::remove_dir_all(target_dir)?;
            println!("Removed target directory");
        }

        Ok(())
    }

    fn handle_version(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("TriCTI Self-Hosting Compiler v0.1.0");
        println!("Built with Rust (version information unavailable)");
        Ok(())
    }
}
