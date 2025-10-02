use std::fs;
use tricti::parser::parse;

#[test]
fn test_module_system_parsing() {
    let source = fs::read_to_string("tests/test_module_system.tri")
        .expect("Failed to read test_module_system.tri");

    let program = parse(source);

    // Verify we parsed the expected number of statements
    assert_eq!(program.statements.len(), 3);

    // Check that we have a module declaration
    match &program.statements[0] {
        tricti::ast::Statement::ModuleDecl {
            is_public,
            name,
            items,
        } => {
            assert!(!is_public, "Module should not be public");
            assert_eq!(name, "test_module");
            assert!(items.is_some(), "Module should have items");
        }
        _ => panic!("First statement should be a ModuleDecl"),
    }

    // Check that we have a use statement
    match &program.statements[1] {
        tricti::ast::Statement::Use {
            is_public,
            path,
            alias,
        } => {
            assert!(!is_public, "Use statement should not be public");
            assert_eq!(path, &vec!["test_module", "test_public_function"]);
            assert!(alias.is_none(), "Use statement should not have an alias");
        }
        _ => panic!("Second statement should be a Use statement"),
    }

    // Check that we have a main function declaration
    match &program.statements[2] {
        tricti::ast::Statement::ConstDecl { name, .. } => {
            assert_eq!(name, "main");
        }
        _ => panic!("Third statement should be a main function ConstDecl"),
    }

    println!("Module system parsing test passed!");
}

#[test]
fn test_public_module_parsing() {
    let source = r#"
        pub mod public_module {
            public_function :: () -> i64 => 100
        }
        
        pub use public_module::public_function as pf;
    "#;

    let program = parse(source.to_string());

    // Check public module declaration
    match &program.statements[0] {
        tricti::ast::Statement::ModuleDecl {
            is_public, name, ..
        } => {
            println!("Module '{}' is_public: {}", name, is_public);
            assert!(is_public, "Module should be public");
            assert_eq!(name, "public_module");
        }
        _ => panic!("First statement should be a public ModuleDecl"),
    }

    // Check public use with alias
    match &program.statements[1] {
        tricti::ast::Statement::Use {
            is_public,
            path,
            alias,
        } => {
            assert!(is_public, "Use statement should be public");
            assert_eq!(path, &vec!["public_module", "public_function"]);
            assert_eq!(alias.as_ref().unwrap(), "pf");
        }
        _ => panic!("Second statement should be a public Use statement with alias"),
    }

    println!("Public module parsing test passed!");
}

#[test]
fn test_nested_module_paths() {
    let source = r#"
        mod outer {
            pub mod inner {
                nested_function :: () -> i64 => 200
            }
        }
        
        use outer::inner::nested_function;
    "#;

    let program = parse(source.to_string());

    // Check use statement with nested path
    match &program.statements[1] {
        tricti::ast::Statement::Use { path, .. } => {
            assert_eq!(path, &vec!["outer", "inner", "nested_function"]);
        }
        _ => panic!("Second statement should be a Use statement with nested path"),
    }

    println!("Nested module paths test passed!");
}
