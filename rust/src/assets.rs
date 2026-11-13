//! Build-time embedded assets, independent of the process working directory.
include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));

pub fn web_asset(path: &str) -> Option<&'static [u8]> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    WEB_ASSETS
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, data)| *data)
}
