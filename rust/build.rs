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
    prepare_native_windows_test_fixture(&manifest);
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
    println!("cargo:rerun-if-env-changed=MANY_AI_REQUIRE_WINDOWS_RUNTIME");
    let runtime_root = root.join("internal/whisperruntime/files/windows-amd64");
    println!("cargo:rerun-if-changed={}", runtime_root.display());
    output.push_str("pub static WINDOWS_RUNTIME: &[(&str, &[u8])] = &[\n");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64")
    {
        let required = env::var("MANY_AI_REQUIRE_WINDOWS_RUNTIME").as_deref() == Ok("1");
        for (name, path) in asset_contract::windows_runtime_files(&runtime_root, required)
            .expect("validate prepared Windows runtime payload")
        {
            println!("cargo:rerun-if-changed={}", path.display());
            output.push_str(&format!(
                "    ({name:?}, include_bytes!({:?})),\n",
                path.to_str().expect("UTF-8 runtime path")
            ));
        }
    }
    output.push_str("];\n");
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

fn prepare_native_windows_test_fixture(manifest: &Path) {
    let host = env::var("HOST").expect("Cargo host target");
    let target = env::var("TARGET").expect("Cargo compilation target");
    if host != target || !target.contains("windows") {
        return;
    }
    let source = manifest.join("tests/fixtures/application/wrapped_spawn/owned-wrapper-windows.rs");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-env-changed=RUSTC");
    let executable = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo build output directory"))
        .join("owned-wrapper-windows.exe");
    let status = std::process::Command::new(env::var_os("RUSTC").expect("Cargo Rust compiler"))
        .arg("--edition=2024")
        .arg("--crate-name=owned_wrapper_windows")
        .arg("--target")
        .arg(&target)
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .status()
        .expect("start Rust compiler for synthetic Windows test fixture");
    assert!(
        status.success(),
        "synthetic Windows test fixture compilation failed: {status}"
    );
    println!(
        "cargo:rustc-env=MANY_AI_NATIVE_TEST_WRAPPER={}",
        executable.display()
    );
}
