use std::path::PathBuf;

use tricti::{program_loader, semantic};

#[test]
fn tripm_main_currently_fails_with_impl_resolution() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let tripm_main = crate_root
        .join("..")
        .join("tripm")
        .join("src")
        .join("main.tri");

    let loaded =
        {
            let previous = std::env::var("SKIP_STDLIB").ok();
            std::env::set_var("SKIP_STDLIB", "1");
            let loaded = program_loader::parse_file_with_std(&tripm_main, false)
                .expect("load tripm main source");
            if let Some(value) = previous {
                std::env::set_var("SKIP_STDLIB", value);
            } else {
                std::env::remove_var("SKIP_STDLIB");
            }
            loaded
        };

    match semantic::analyze_program(&loaded.program) {
        Ok(_) => panic!("tripm main should not compile cleanly yet"),
        Err(semantic::SemanticError::UndefinedVariable(name)) => {
            assert_eq!(name, "impl", "unexpected undefined variable name: {name}");
        }
        Err(other) => panic!("unexpected semantic failure: {other:?}"),
    }
}
