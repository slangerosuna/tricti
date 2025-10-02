use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub struct FileSystem;

impl FileSystem {
    /// Read a file and return its contents as a string
    pub fn read_file(path: &str) -> Result<String, io::Error> {
        fs::read_to_string(path)
    }

    /// Write content to a file
    pub fn write_file(path: &str, content: &str) -> Result<(), io::Error> {
        fs::write(path, content)
    }

    /// Check if a file exists
    pub fn file_exists(path: &str) -> bool {
        Path::new(path).exists()
    }

    /// Get the current working directory
    pub fn current_dir() -> Result<PathBuf, io::Error> {
        std::env::current_dir()
    }

    /// Create a directory and all parent directories if they don't exist
    pub fn create_dir_all(path: &str) -> Result<(), io::Error> {
        fs::create_dir_all(path)
    }

    /// List files in a directory
    pub fn list_dir(path: &str) -> Result<Vec<PathBuf>, io::Error> {
        let mut files = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            files.push(entry.path());
        }
        Ok(files)
    }

    /// Get file extension
    pub fn get_extension(path: &str) -> Option<&str> {
        Path::new(path).extension()?.to_str()
    }

    /// Join path components
    pub fn join_path(base: &str, component: &str) -> PathBuf {
        Path::new(base).join(component)
    }
}
