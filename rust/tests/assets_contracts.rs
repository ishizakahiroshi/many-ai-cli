use many_ai_cli::{asset_contract::*, assets};
use std::{fs, path::Path};
fn fixture() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("src");
    let dist = t.path().join("dist");
    for d in ["styles", "vendor", "icons", "i18n", "debug"] {
        fs::create_dir_all(source.join(d)).unwrap();
        fs::create_dir_all(dist.join(d)).unwrap();
    }
    for file in [
        "index.html",
        "styles.css",
        "icon.svg",
        "manifest.webmanifest",
        "vendor/xterm.min.js",
        "app-entry.ts",
        "app.ts",
        "sw.ts",
        "whisper-recorder-worklet.js",
        "i18n/en.json",
        "icons/icon.png",
        "styles/base.css",
        "debug/probe.ts",
    ] {
        fs::write(source.join(file), "synthetic").unwrap();
        let mut generated = Path::new(file).to_path_buf();
        if generated.extension().is_some_and(|x| x == "ts") {
            generated.set_extension("js");
        }
        fs::write(dist.join(generated), "synthetic").unwrap();
    }
    fs::write(dist.join("debug/index.js"), "").unwrap();
    fs::write(dist.join(".src-hash"), "012345abcdef").unwrap();
    t
}
#[test]
fn missing_non_entry_asset_fails_closed() {
    let t = fixture();
    let src = t.path().join("src");
    let dist = t.path().join("dist");
    validate_web_assets(&src, &dist).unwrap();
    fs::remove_file(dist.join("i18n/en.json")).unwrap();
    assert!(validate_web_assets(&src, &dist).is_err());
}
#[test]
fn empty_application_and_bad_identity_fail_closed() {
    let t = fixture();
    let src = t.path().join("src");
    let dist = t.path().join("dist");
    fs::write(dist.join("app.js"), "").unwrap();
    assert!(validate_web_assets(&src, &dist).is_err());
    fs::write(dist.join("app.js"), "synthetic").unwrap();
    fs::write(dist.join(".src-hash"), "bad").unwrap();
    assert!(validate_web_assets(&src, &dist).is_err());
}
#[test]
fn actual_embedded_assets_include_boot_and_voice() {
    for path in [
        "/",
        "app-entry.js",
        "app.js",
        "sw.js",
        "whisper-recorder-worklet.js",
        "vendor/xterm.min.js",
        ".src-hash",
    ] {
        assert!(assets::web_asset(path).is_some(), "missing {path}");
    }
    assert!(!assets::LAUNCHER_UI.is_empty());
    assert!(assets::web_asset("../../Cargo.toml").is_none());
}
