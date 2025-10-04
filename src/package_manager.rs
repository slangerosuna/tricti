// This module will handle package management for TriCTI when I get the package repo up
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub authors: Vec<String>,
    pub license: Option<String>,
    pub dependencies: HashMap<String, VersionRequirement>,
    pub dev_dependencies: HashMap<String, VersionRequirement>,
    pub lib: Option<LibraryTarget>,
    pub bin: Vec<BinaryTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryTarget {
    pub name: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryTarget {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionRequirement {
    pub version: String,
    pub source: PackageSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PackageSource {
    Registry { url: String },
    Git { url: String, rev: Option<String> },
    Path { path: PathBuf },
}

pub struct PackageManager {
    registry_url: String,
    cache_dir: PathBuf,
}

impl PackageManager {
    pub fn new(registry_url: String, cache_dir: PathBuf) -> Self {
        Self {
            registry_url,
            cache_dir,
        }
    }

    /// Install a package and its dependencies
    pub fn install_package(&self, name: &str, version: &str) -> Result<(), PackageError> {
        println!("Installing package: {}@{}", name, version);

        // Download package
        let package = self.download_package(name, version)?;

        // Install dependencies recursively
        for (dep_name, dep_req) in &package.dependencies {
            self.install_package(dep_name, &dep_req.version)?;
        }

        // Extract package to cache
        self.extract_package(&package)?;

        Ok(())
    }

    /// Resolve all dependencies for a package
    pub fn resolve_dependencies(&self, package: &Package) -> Result<Vec<Package>, PackageError> {
        let mut resolved = Vec::new();
        let mut visited = std::collections::HashSet::new();

        self.resolve_deps_recursive(package, &mut resolved, &mut visited)?;

        Ok(resolved)
    }

    fn resolve_deps_recursive(
        &self,
        package: &Package,
        resolved: &mut Vec<Package>,
        visited: &mut std::collections::HashSet<String>,
    ) -> Result<(), PackageError> {
        if visited.contains(&package.name) {
            return Ok(());
        }

        visited.insert(package.name.clone());

        for (dep_name, dep_req) in &package.dependencies {
            let dep_package = self.get_package(dep_name, &dep_req.version)?;
            self.resolve_deps_recursive(&dep_package, resolved, visited)?;
        }

        resolved.push(package.clone());
        Ok(())
    }

    /// Get a specific package version
    fn get_package(&self, name: &str, version: &str) -> Result<Package, PackageError> {
        // Check local cache first
        let cache_path = self.cache_dir.join(format!("{}@{}", name, version));
        if cache_path.exists() {
            return self.load_cached_package(&cache_path);
        }

        // Download from registry
        self.download_package(name, version)
    }

    fn download_package(&self, name: &str, version: &str) -> Result<Package, PackageError> {
        println!(
            "Downloading {}@{} from registry {}",
            name, version, self.registry_url
        );

        // Simulate downloading from registry
        // In a real implementation, this would make HTTP requests
        let package = Package {
            name: name.to_string(),
            version: version.to_string(),
            description: None,
            authors: vec![],
            license: None,
            dependencies: HashMap::new(),
            dev_dependencies: HashMap::new(),
            lib: None,
            bin: vec![],
        };

        Ok(package)
    }

    fn extract_package(&self, package: &Package) -> Result<(), PackageError> {
        let package_dir = self
            .cache_dir
            .join(format!("{}@{}", package.name, package.version));
        std::fs::create_dir_all(&package_dir).map_err(PackageError::IoError)?;

        println!("Extracted {} to {:?}", package.name, package_dir);
        Ok(())
    }

    fn load_cached_package(&self, path: &PathBuf) -> Result<Package, PackageError> {
        let package_file = path.join("Package.toml");
        let content = std::fs::read_to_string(package_file).map_err(PackageError::IoError)?;

        toml::from_str(&content).map_err(|e| PackageError::ParseError(e.to_string()))
    }

    /// Create a new package manifest
    pub fn init_package(&self, name: &str, path: &PathBuf) -> Result<(), PackageError> {
        let package = Package {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: Some(format!("A TriCTI package")),
            authors: vec!["Your Name <you@example.com>".to_string()],
            license: Some("MIT OR Apache-2.0".to_string()),
            dependencies: HashMap::new(),
            dev_dependencies: HashMap::new(),
            lib: Some(LibraryTarget {
                name: Some(name.to_string()),
                path: PathBuf::from("src/lib.tri"),
            }),
            bin: vec![],
        };

        let toml_content = toml::to_string_pretty(&package)
            .map_err(|e| PackageError::ParseError(e.to_string()))?;

        let manifest_path = path.join("Package.toml");
        std::fs::write(manifest_path, toml_content).map_err(PackageError::IoError)?;

        // Create basic directory structure
        std::fs::create_dir_all(path.join("src")).map_err(PackageError::IoError)?;

        let lib_content = r#"// TriCTI Library
pub main :: () -> i64 => {
    42
}
"#;
        std::fs::write(path.join("src/lib.tri"), lib_content).map_err(PackageError::IoError)?;

        println!("Created new TriCTI package: {}", name);
        Ok(())
    }
}

#[derive(Debug)]
pub enum PackageError {
    IoError(std::io::Error),
    ParseError(String),
    NetworkError(String),
    DependencyConflict(String),
    NotFound(String),
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PackageError::IoError(err) => write!(f, "IO error: {}", err),
            PackageError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            PackageError::NetworkError(msg) => write!(f, "Network error: {}", msg),
            PackageError::DependencyConflict(msg) => write!(f, "Dependency conflict: {}", msg),
            PackageError::NotFound(name) => write!(f, "Package not found: {}", name),
        }
    }
}

impl std::error::Error for PackageError {}
