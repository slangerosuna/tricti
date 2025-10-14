use crate::ast::{ConstValue, Expression, IntegerLiteral, Literal, Program, Statement};
use crate::parser;
use std::collections::HashSet;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdlibStatus {
    Included,
    SkippedEnvironment,
    SkippedFlag,
    SkippedAttribute,
}

pub struct LoadOptions {
    pub skip_std_env: bool,
    pub skip_std_flag: bool,
    pub stdlib_path: PathBuf,
    pub base_dir: PathBuf,
}

pub struct LoadedProgram {
    pub program: Program,
    pub stdlib_status: StdlibStatus,
}

fn fallback_std_program() -> Program {
    Program {
        statements: vec![Statement::ConstDecl {
            attributes: Vec::new(),
            name: "__tricti_std_placeholder".to_string(),
            type_params: Vec::new(),
            type_annotation: None,
            value: ConstValue::Expression(Expression::Literal(Literal::Integer(IntegerLiteral {
                raw: "0".to_string(),
                value: 0,
                suffix: None,
            }))),
            extern_linkage: None,
        }],
    }
}

pub fn parse_file_with_std(path: &Path, skip_std_flag: bool) -> std::io::Result<LoadedProgram> {
    let source = fs::read_to_string(path)?;
    let cwd = std::env::current_dir().expect("failed to get current dir");
    let base_dir_buf = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| cwd.clone());
    let stdlib_path_buf = resolve_stdlib_path(&cwd);

    let options = LoadOptions {
        skip_std_env: std::env::var("SKIP_STDLIB").unwrap_or_default() == "1",
        skip_std_flag,
        stdlib_path: stdlib_path_buf,
        base_dir: base_dir_buf,
    };

    Ok(parse_source_with_std(source, Some(path), options))
}

pub fn parse_source_with_std(
    source: String,
    primary_path: Option<&Path>,
    options: LoadOptions,
) -> LoadedProgram {
    let mut visited_modules: HashSet<PathBuf> = HashSet::new();

    if let Some(path) = primary_path {
        if let Ok(canonical) = fs::canonicalize(path) {
            visited_modules.insert(canonical);
        }
    }

    let parse_main = panic::catch_unwind(AssertUnwindSafe(|| parser::parse(source)));
    let mut program = match parse_main {
        Ok(program) => program,
        Err(err) => {
            eprintln!("warning: failed to parse primary source: {:?}", err);
            Program {
                statements: Vec::new(),
            }
        }
    };
    let has_no_std_attr = has_program_no_std_attribute(&program);

    program = expand_modules(
        program,
        options.base_dir.as_path(),
        options.base_dir.as_path(),
        None,
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
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(""));
        let mut std_program = load_and_expand_stdlib(
            options.stdlib_path.as_path(),
            std_root.as_path(),
            &mut visited_modules,
        );
        statements.append(&mut std_program.statements);
    }

    statements.append(&mut program.statements);

    LoadedProgram {
        program: Program { statements },
        stdlib_status,
    }
}

fn resolve_stdlib_path_from_env() -> Option<PathBuf> {
    if let Ok(explicit_file) = std::env::var("TRICTI_STDLIB_PATH") {
        let path = PathBuf::from(explicit_file);
        if path.exists() {
            return Some(path);
        }
    }

    if let Ok(explicit_dir) = std::env::var("TRICTI_STDLIB_DIR") {
        let candidate = PathBuf::from(explicit_dir).join("std.tri");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

fn resolve_stdlib_path_from_executable() -> Option<PathBuf> {
    let mut current = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))?;

    loop {
        let candidate = current.join("stdlib").join("std.tri");
        if candidate.exists() {
            return Some(candidate);
        }

        if !current.pop() {
            break;
        }
    }

    None
}

pub fn resolve_stdlib_root() -> PathBuf {
    let cwd = std::env::current_dir().expect("failed to get current dir");
    resolve_stdlib_path(&cwd)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| cwd.join("stdlib"))
}

fn resolve_stdlib_path(cwd: &Path) -> PathBuf {
    if let Some(path) = resolve_stdlib_path_from_env() {
        return path;
    }

    if let Some(path) = resolve_stdlib_path_from_executable() {
        return path;
    }

    let mut current = cwd.to_path_buf();
    loop {
        let direct = current.join("stdlib").join("std.tri");
        if direct.exists() {
            return direct;
        }

        let compiler_relative = current
            .join("tricti-compiler")
            .join("stdlib")
            .join("std.tri");
        if compiler_relative.exists() {
            return compiler_relative;
        }

        if !current.pop() {
            break;
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest_candidate = manifest_dir.join("stdlib").join("std.tri");
    if manifest_candidate.exists() {
        manifest_candidate
    } else {
        cwd.join("stdlib").join("std.tri")
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

    let parse_result = panic::catch_unwind(AssertUnwindSafe(|| parser::parse(stdlib_content)));
    let std_program = match parse_result {
        Ok(program) => program,
        Err(err) => {
            eprintln!(
                "warning: failed to parse stdlib {}: {:?}",
                stdlib_path.display(),
                err
            );
            return fallback_std_program();
        }
    };
    let base_dir = stdlib_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current dir"));

    expand_modules(
        std_program,
        &base_dir,
        root_dir,
        Some(stdlib_path),
        visited_modules,
    )
}

fn expand_modules(
    program: Program,
    base_dir: &Path,
    root_dir: &Path,
    current_file: Option<&Path>,
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
                let module_file = format!("{}.tri", name);
                let mut roots: Vec<PathBuf> = vec![
                    base_dir.to_path_buf(),
                    base_dir.join("src"),
                    base_dir.join("stdlib"),
                    root_dir.to_path_buf(),
                    root_dir.join("src"),
                    root_dir.join("stdlib"),
                ];

                if let Some(parent) = base_dir.parent() {
                    roots.push(parent.to_path_buf());
                    roots.push(parent.join("src"));
                    roots.push(parent.join("stdlib"));
                }

                if let Some(current_path) = current_file {
                    if let Some(parent) = current_path.parent() {
                        roots.push(parent.to_path_buf());
                        if let Some(stem) = current_path.file_stem().and_then(|s| s.to_str()) {
                            roots.push(parent.join(stem));
                        }
                    }
                }

                let mut tried: Vec<PathBuf> = Vec::new();
                for root in roots {
                    tried.push(root.join(Path::new(&module_file)));
                    tried.push(root.join(Path::new(name)).join("mod.tri"));
                    tried.push(root.join(Path::new(name)).join(Path::new(&module_file)));
                }

                let mut loaded: Option<(String, PathBuf)> = None;
                let mut already_loaded = false;
                let mut unique: HashSet<PathBuf> = HashSet::new();
                for candidate in tried {
                    if !unique.insert(candidate.clone()) {
                        continue;
                    }
                    if let Ok(canonical) = fs::canonicalize(&candidate) {
                        if visited.contains(&canonical) {
                            already_loaded = true;
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
                    let parse_result =
                        panic::catch_unwind(AssertUnwindSafe(move || parser::parse(content)));
                    let mut sub = match parse_result {
                        Ok(program) => program,
                        Err(err) => {
                            eprintln!(
                                "warning: failed to parse module {}: {:?}",
                                canonical.display(),
                                err
                            );
                            continue;
                        }
                    };
                    let base_for_sub = canonical
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| base_dir.to_path_buf());
                    sub = expand_modules(sub, &base_for_sub, root_dir, Some(&canonical), visited);
                    expanded.extend(sub.statements);
                } else if already_loaded {
                    continue;
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
                    let joined_file = format!("{}.tri", joined);
                    let module_file = format!("{}.tri", name);
                    let joined_path = Path::new(&joined);

                    let mut roots: Vec<PathBuf> = vec![
                        base_dir.to_path_buf(),
                        base_dir.join("src"),
                        base_dir.join("stdlib"),
                        root_dir.to_path_buf(),
                        root_dir.join("src"),
                        root_dir.join("stdlib"),
                    ];

                    if let Some(parent) = base_dir.parent() {
                        roots.push(parent.to_path_buf());
                        roots.push(parent.join("src"));
                        roots.push(parent.join("stdlib"));
                    }

                    if let Some(current_path) = current_file {
                        if let Some(parent) = current_path.parent() {
                            roots.push(parent.to_path_buf());
                            if let Some(stem) = current_path.file_stem().and_then(|s| s.to_str()) {
                                roots.push(parent.join(stem));
                            }
                        }
                    }

                    let mut tried: Vec<PathBuf> = Vec::new();
                    for root in roots {
                        tried.push(root.join(Path::new(&joined_file)));
                        tried.push(root.join(joined_path).join("mod.tri"));
                        tried.push(root.join(joined_path).join(Path::new(&module_file)));
                        tried.push(root.join(Path::new(&module_file)));
                        tried.push(root.join(Path::new(name)).join("mod.tri"));
                        tried.push(root.join(Path::new(name)).join(Path::new(&module_file)));
                    }

                    let mut loaded: Option<(String, PathBuf)> = None;
                    let mut already_loaded = false;
                    let mut unique: HashSet<PathBuf> = HashSet::new();
                    for candidate in tried {
                        if !unique.insert(candidate.clone()) {
                            continue;
                        }
                        if let Ok(canonical) = fs::canonicalize(&candidate) {
                            if visited.contains(&canonical) {
                                already_loaded = true;
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
                        let parse_result =
                            panic::catch_unwind(AssertUnwindSafe(move || parser::parse(content)));
                        let mut sub = match parse_result {
                            Ok(program) => program,
                            Err(err) => {
                                eprintln!(
                                    "warning: failed to parse use target {}: {:?}",
                                    canonical.display(),
                                    err
                                );
                                continue;
                            }
                        };
                        let base_for_sub = canonical
                            .parent()
                            .map(Path::to_path_buf)
                            .unwrap_or_else(|| base_dir.to_path_buf());
                        sub =
                            expand_modules(sub, &base_for_sub, root_dir, Some(&canonical), visited);
                        expanded.extend(sub.statements);
                    } else if already_loaded {
                        continue;
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
            stdlib_path,
            base_dir: current_dir.clone(),
        };
        let loaded = parse_source_with_std("main :: () => do {}".to_string(), None, options);
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
            stdlib_path,
            base_dir: current_dir.clone(),
        };
        let loaded = parse_source_with_std("@no_std\nmain :: () => {}".to_string(), None, options);
        assert_eq!(loaded.stdlib_status, StdlibStatus::SkippedAttribute);
    }
}
