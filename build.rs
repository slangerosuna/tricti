use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=stdlib/");
    println!("cargo:rerun-if-changed=build.rs");

    let base_target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| ".".to_string());
    let build_target_dir =
        std::env::var("CARGO_BUILD_TARGET_DIR").unwrap_or_else(|_| "target".to_string());
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());

    let target_dir = PathBuf::from(base_target_dir)
        .join(build_target_dir)
        .join(profile)
        .join("stdlib");

    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir).unwrap();
    }

    std::fs::create_dir_all(&target_dir).unwrap();

    let resource_dir = Path::new("stdlib");
    copy_files(resource_dir, &target_dir);
}

fn copy_files(from: &Path, to: &Path) {
    let read_dir = std::fs::read_dir(from).unwrap();
    for entry in read_dir {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().unwrap();
            let new_dir = to.join(dir_name);
            std::fs::create_dir_all(&new_dir).unwrap();
            copy_files(&path, &new_dir);
        } else {
            let file_name = path.file_name().unwrap();
            let new_file = to.join(file_name);
            std::fs::copy(&path, &new_file).unwrap();
        }
    }
}
