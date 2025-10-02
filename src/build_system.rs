// Build System for TriCTI Self-Hosting
use std::process::{Command, Stdio};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use crate::filesystem::FileSystem;

#[derive(Debug, Clone)]
pub struct BuildTarget {
    pub name: String,
    pub source_files: Vec<PathBuf>,
    pub dependencies: Vec<String>,
    pub output_type: OutputType,
    pub output_path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum OutputType {
    Executable,
    Library,
    Object,
}

#[derive(Debug)]
pub struct BuildConfig {
    pub targets: HashMap<String, BuildTarget>,
    pub optimization_level: OptimizationLevel,
    pub debug_info: bool,
    pub target_triple: String,
}

#[derive(Debug, Clone)]
pub enum OptimizationLevel {
    None,
    Speed,
    Size,
    Maximum,
}

pub struct BuildSystem {
    config: BuildConfig,
}

impl BuildSystem {
    pub fn new(config: BuildConfig) -> Self {
        Self { config }
    }

    /// Build a specific target
    pub fn build_target(&self, target_name: &str) -> Result<(), BuildError> {
        let target = self.config.targets.get(target_name)
            .ok_or_else(|| BuildError::TargetNotFound(target_name.to_string()))?;

        println!("Building target: {}", target.name);

        // Compile each source file
        for source_file in &target.source_files {
            self.compile_file(source_file, target)?;
        }

        // Link if necessary
        if matches!(target.output_type, OutputType::Executable) {
            self.link_target(target)?;
        }

        println!("Successfully built target: {}", target.name);
        Ok(())
    }

    /// Build all targets
    pub fn build_all(&self) -> Result<(), BuildError> {
        let mut build_order = self.resolve_dependencies()?;
        
        for target_name in build_order {
            self.build_target(&target_name)?;
        }
        
        Ok(())
    }

    /// Compile a single file
    fn compile_file(&self, source_file: &Path, target: &BuildTarget) -> Result<(), BuildError> {
        let source_content = FileSystem::read_file(source_file.to_str().unwrap())
            .map_err(|e| BuildError::IoError(e))?;

        // Parse and compile with TriCTI compiler
        let program = crate::parser::parse(source_content);
        
        // For now, we'll use a placeholder compilation step
        // In a full self-hosted version, this would use TriCTI's own backend
        println!("Compiling {} with {} statements", 
                 source_file.display(), program.statements.len());
        
        Ok(())
    }

    /// Link target (for executables)
    fn link_target(&self, target: &BuildTarget) -> Result<(), BuildError> {
        println!("Linking target: {}", target.name);
        // Linking logic here
        Ok(())
    }

    /// Resolve build order based on dependencies
    fn resolve_dependencies(&self) -> Result<Vec<String>, BuildError> {
        let mut build_order = Vec::new();
        let mut visited = std::collections::HashSet::new();
        
        for target_name in self.config.targets.keys() {
            if !visited.contains(target_name) {
                self.visit_target(target_name, &mut visited, &mut build_order)?;
            }
        }
        
        Ok(build_order)
    }
    
    fn visit_target(&self, target_name: &str, visited: &mut std::collections::HashSet<String>, 
                   build_order: &mut Vec<String>) -> Result<(), BuildError> {
        if visited.contains(target_name) {
            return Ok(());
        }
        
        let target = self.config.targets.get(target_name)
            .ok_or_else(|| BuildError::TargetNotFound(target_name.to_string()))?;
        
        // Visit dependencies first
        for dep in &target.dependencies {
            self.visit_target(dep, visited, build_order)?;
        }
        
        visited.insert(target_name.to_string());
        build_order.push(target_name.to_string());
        Ok(())
    }
}

#[derive(Debug)]
pub enum BuildError {
    TargetNotFound(String),
    IoError(std::io::Error),
    CompilationError(String),
    LinkError(String),
    DependencyError(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            BuildError::TargetNotFound(name) => write!(f, "Target not found: {}", name),
            BuildError::IoError(err) => write!(f, "IO error: {}", err),
            BuildError::CompilationError(msg) => write!(f, "Compilation error: {}", msg),
            BuildError::LinkError(msg) => write!(f, "Link error: {}", msg),
            BuildError::DependencyError(msg) => write!(f, "Dependency error: {}", msg),
        }
    }
}

impl std::error::Error for BuildError {}