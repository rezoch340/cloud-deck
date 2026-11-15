use std::{env, fs, path::Path};

fn collect_files(root: &Path, directory: &Path, entries: &mut String) {
    let mut children: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    children.sort();
    for child in children {
        assert!(
            !child.is_symlink(),
            "Application bundles cannot contain symbolic links"
        );
        if child.is_dir() {
            collect_files(root, &child, entries);
        } else {
            let relative = child
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            let absolute = child.canonicalize().unwrap();
            entries.push_str(&format!(
                "({relative:?}, include_str!({:?})),\n",
                absolute.to_str().unwrap()
            ));
        }
    }
}

fn main() {
    let root = Path::new("../applications");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut entries = String::from("pub const FILES: &[(&str, &str)] = &[\n");
    for entry in fs::read_dir(root).unwrap() {
        let directory = entry.unwrap().path();
        if directory.is_dir() && directory.join("manifest.json").is_file() {
            collect_files(root, &directory, &mut entries);
        }
    }
    entries.push_str("];\n");
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("applications.rs"),
        entries,
    )
    .unwrap();
    tauri_build::build()
}
