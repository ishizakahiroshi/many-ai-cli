#[path = "src/asset_contract.rs"]
mod asset_contract;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

fn walk(root: &Path, dir: &Path, entries: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(io::Error::other("asset symlinks are not allowed"));
        }
        if kind.is_dir() {
            walk(root, &entry.path(), entries)?;
        } else if kind.is_file() {
            let name = entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            entries.push((name, entry.path()));
        }
    }
    Ok(())
}

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.parent().unwrap();
    let web = root.join("web/dist");
    if !web.join("index.html").is_file() {
        panic!(
            "missing generated web/dist/index.html; run the locked Bun frontend build in the isolated checkout before Cargo"
        );
    }
    asset_contract::validate_web_assets(&root.join("web/src"), &web)
        .expect("generated Web asset validation failed");
    println!("cargo:rerun-if-changed={}", root.join("web/src").display());
    println!("cargo:rerun-if-changed=src/asset_contract.rs");
    let mut files = Vec::new();
    walk(&web, &web, &mut files).expect("read generated Web assets");
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut output = String::from("pub static WEB_ASSETS: &[(&str, &[u8])] = &[\n");
    for (name, path) in files {
        println!("cargo:rerun-if-changed={}", path.display());
        output.push_str(&format!(
            "    ({name:?}, include_bytes!({:?})),\n",
            path.to_str().expect("UTF-8 asset path")
        ));
    }
    output.push_str("];\n");
    let launcher = root.join("internal/launcher/ui/index.html");
    println!("cargo:rerun-if-changed={}", launcher.display());
    output.push_str(&format!(
        "pub static LAUNCHER_UI: &[u8] = include_bytes!({:?});\n",
        launcher.to_str().unwrap()
    ));
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("embedded_assets.rs"),
        output,
    )
    .expect("write asset source");
    println!("cargo:rerun-if-changed={}", web.display());
    for name in [
        "MANY_AI_BUILD_VERSION",
        "MANY_AI_BUILD_COMMIT",
        "MANY_AI_BUILD_TIME",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
        let default = if name == "MANY_AI_BUILD_VERSION" {
            "dev-rust-candidate"
        } else {
            ""
        };
        println!(
            "cargo:rustc-env={name}={}",
            env::var(name).unwrap_or_else(|_| default.into())
        );
    }
}
