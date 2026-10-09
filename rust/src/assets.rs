//! Build-time embedded assets, independent of the process working directory.
include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));

/// Prepared VS Redist bytes for Windows x64; empty on other targets or an
/// unprepared development build, matching Go's placeholder-only embed behavior.
pub fn windows_runtime_payload() -> Vec<(String, Vec<u8>)> {
    WINDOWS_RUNTIME
        .iter()
        .map(|(name, bytes)| ((*name).into(), bytes.to_vec()))
        .collect()
}

pub fn web_asset(path: &str) -> Option<&'static [u8]> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    WEB_ASSETS
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, data)| *data)
}
