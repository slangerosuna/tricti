use crate::ast::{Program, Statement};
use crate::parser;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdlibStatus {
    Included,
    SkippedEnvironment,
    SkippedFlag,
    SkippedAttribute,
}

pub struct LoadOptions<'a> {
    pub skip_std_env: bool,
    pub skip_std_flag: bool,
    pub stdlib_path: &'a Path,
    pub base_dir: &'a Path,
}

pub struct LoadedProgram {
    pub program: Program,
    pub stdlib_status: StdlibStatus,
}

pub fn parse_file_with_std(path: &Path, skip_std_flag: bool) -> std::io::Result<LoadedProgram> {
    let source = fs::read_to_string(path)?;
    let cwd = std::env::current_dir().expect("failed to get current dir");
    let base_dir_buf = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| cwd.clone());
    let stdlib_path_buf = cwd.join("stdlib").join("std.tri");

    let options = LoadOptions {
        skip_std_env: std::env::var("SKIP_STDLIB").unwrap_or_default() == "1",
        skip_std_flag,
        stdlib_path: &stdlib_path_buf,
        base_dir: &base_dir_buf,
    };

    Ok(parse_source_with_std(source, Some(path), options))
}

pub fn parse_source_with_std(
    source: String,
    primary_path: Option<&Path>,
    options: LoadOptions<'_>,
) -> LoadedProgram {
    let mut visited_modules: HashSet<PathBuf> = HashSet::new();

    if let Some(path) = primary_path {
        if let Ok(canonical) = fs::canonicalize(path) {
            visited_modules.insert(canonical);
        }
    }

    let mut program = parser::parse(source);
    let has_no_std_attr = has_program_no_std_attribute(&program);

    program = expand_modules(
        program,
        options.base_dir,
        options.base_dir,
        &mut visited_modules,
    );

    let stdlib_status = if options.skip_std_env {
        StdlibStatus::SkippedEnvironment
    } else if options.skip_std_flag {
        StdlibStatus::SkippedFlag
    } else if has_no_std_attr {
        StdlibStatus::SkippedAttribute
    } else {
        StdlibStatus::Included
    };

    let mut statements = Vec::new();

    if matches!(stdlib_status, StdlibStatus::Included) {
        let std_root = options
            .stdlib_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""));
        let mut std_program =
            load_and_expand_stdlib(options.stdlib_path, std_root, &mut visited_modules);
        statements.append(&mut std_program.statements);
    }

    statements.append(&mut program.statements);

    LoadedProgram {
        program: Program { statements },
        stdlib_status,
    }
}

fn has_program_no_std_attribute(program: &Program) -> bool {
    program
        .statements
        .first()
        .map(|stmt| match stmt {
            Statement::ConstDecl { attributes, .. } => {
                attributes.iter().any(|attr| attr.name == "no_std")
            }
            _ => false,
        })
        .unwrap_or(false)
}

fn load_and_expand_stdlib(
    stdlib_path: &Path,
    root_dir: &Path,
    visited_modules: &mut HashSet<PathBuf>,
) -> Program {
    if let Ok(canonical) = fs::canonicalize(stdlib_path) {
        visited_modules.insert(canonical);
    }

    let stdlib_content = fs::read_to_string(stdlib_path).unwrap_or_else(|err| {
        panic!(
            "Failed to read stdlib file {}: {}",
            stdlib_path.display(),
            err
        )
    });

    let std_program = parser::parse(stdlib_content);
    let base_dir = stdlib_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));

    expand_modules(std_program, &base_dir, root_dir, visited_modules)
}

fn expand_modules(
    program: Program,
    base_dir: &Path,
    root_dir: &Path,
    visited: &mut HashSet<PathBuf>,
) -> Program {
    let mut expanded: Vec<Statement> = Vec::new();
    for stmt in program.statements.into_iter() {
        match &stmt {
            Statement::ModuleDecl {
                name,
                items,
                is_public: _,
            } if items.is_none() => {
                let mut tried: Vec<PathBuf> = Vec::new();
                tried.push(base_dir.join(format!("{}.tri", name)));
                tried.push(base_dir.join("src").join(format!("{}.tri", name)));
                tried.push(root_dir.join(format!("{}.tri", name)));
                tried.push(root_dir.join("src").join(format!("{}.tri", name)));
                tried.push(root_dir.join("stdlib").join(format!("{}.tri", name)));

                let mut loaded: Option<(String, PathBuf)> = None;
                if let Some(parent) = base_dir.parent() {
                    let parent = parent.to_path_buf();
                    tried.push(parent.join(format!("{}.tri", name)));
                    tried.push(parent.join("src").join(format!("{}.tri", name)));
                    tried.push(parent.join("stdlib").join(format!("{}.tri", name)));
                }
                for candidate in tried {
                    if let Ok(canonical) = fs::canonicalize(&candidate) {
                        if visited.contains(&canonical) {
                            continue;
                        }
                        if let Ok(content) = fs::read_to_string(&canonical) {
                            visited.insert(canonical.clone());
                            loaded = Some((content, canonical));
                            break;
                        }
                    }
                }

                if let Some((content, canonical)) = loaded {
                    let mut sub = parser::parse(content);
                    let base_for_sub = canonical
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| base_dir.to_path_buf());
                    sub = expand_modules(sub, &base_for_sub, root_dir, visited);
                    expanded.extend(sub.statements);
                } else {
                    eprintln!("warning: module '{}' not found on disk", name);
                }
            }
            Statement::Use {
                path,
                is_public: _,
                alias: _,
            } => {
                if !path.is_empty() {
                    let joined = path.join("/");
                    let name = &path[0];
                    let mut tried: Vec<PathBuf> = Vec::new();
                    tried.push(base_dir.join(format!("{}.tri", joined)));
                    tried.push(base_dir.join("src").join(format!("{}.tri", joined)));
                    tried.push(base_dir.join("stdlib").join(format!("{}.tri", joined)));
                    tried.push(base_dir.join(format!("{}.tri", name)));
                    tried.push(base_dir.join("src").join(format!("{}.tri", name)));
                    tried.push(base_dir.join("stdlib").join(format!("{}.tri", name)));
                    tried.push(root_dir.join(format!("{}.tri", joined)));
                    tried.push(root_dir.join("src").join(format!("{}.tri", joined)));
                    tried.push(root_dir.join("stdlib").join(format!("{}.tri", joined)));
                    tried.push(root_dir.join(format!("{}.tri", name)));
                    tried.push(root_dir.join("src").join(format!("{}.tri", name)));
                    tried.push(root_dir.join("stdlib").join(format!("{}.tri", name)));
                    let mut loaded: Option<(String, PathBuf)> = None;
                    if let Some(parent) = base_dir.parent() {
                        let parent = parent.to_path_buf();
                        tried.push(parent.join(format!("{}.tri", joined)));
                        tried.push(parent.join("src").join(format!("{}.tri", joined)));
                        tried.push(parent.join("stdlib").join(format!("{}.tri", joined)));
                        tried.push(parent.join(format!("{}.tri", name)));
                        tried.push(parent.join("src").join(format!("{}.tri", name)));
                        tried.push(parent.join("stdlib").join(format!("{}.tri", name)));
                    }
                    for candidate in tried {
                        if let Ok(canonical) = fs::canonicalize(&candidate) {
                            if visited.contains(&canonical) {
                                continue;
                            }
                            if let Ok(content) = fs::read_to_string(&canonical) {
                                visited.insert(canonical.clone());
                                loaded = Some((content, canonical));
                                break;
                            }
                        }
                    }
                    if let Some((content, canonical)) = loaded {
                        let mut sub = parser::parse(content);
                        let base_for_sub = canonical
                            .parent()
                            .map(Path::to_path_buf)
                            .unwrap_or_else(|| base_dir.to_path_buf());
                        sub = expand_modules(sub, &base_for_sub, root_dir, visited);
                        expanded.extend(sub.statements);
                    } else {
                        eprintln!("warning: use {:?} not found on disk", path);
                    }
                }
            }
            Statement::ModuleDecl {
                name: _,
                items: Some(items),
                is_public: _,
            } => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_std_by_default() {
        let current_dir = std::env::current_dir().expect("cwd");
        let stdlib_path = current_dir.join("stdlib").join("std.tri");
        let options = LoadOptions {
            skip_std_env: false,
            skip_std_flag: false,
            stdlib_path: &stdlib_path,
            base_dir: &current_dir,
        };
        let loaded = parse_source_with_std("main :: () => {}".to_string(), None, options);
        assert_eq!(loaded.stdlib_status, StdlibStatus::Included);
        assert!(loaded.program.statements.len() > 1);
    }

    #[test]
    fn skips_std_with_attribute() {
        let current_dir = std::env::current_dir().expect("cwd");
        let stdlib_path = current_dir.join("stdlib").join("std.tri");
        let options = LoadOptions {
            skip_std_env: false,
            skip_std_flag: false,
            stdlib_path: &stdlib_path,
            base_dir: &current_dir,
        };
        let loaded = parse_source_with_std("@no_std\nmain :: () => {}".to_string(), None, options);
        assert_eq!(loaded.stdlib_status, StdlibStatus::SkippedAttribute);
    }
}
