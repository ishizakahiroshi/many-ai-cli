//! Apply the same trial GUI boundary to an already-running Hub as startup.
use crate::config::RuntimePaths;
pub fn open_existing_hub(
    paths: &RuntimePaths,
    port: u16,
    token: &str,
    opener: impl FnOnce(&str) -> std::io::Result<()>,
) -> Result<(), url::ParseError> {
    if !paths.automatic_external_actions_allowed() {
        return Ok(());
    }
    let mut url = url::Url::parse(&format!("http://127.0.0.1:{port}/"))?;
    url.query_pairs_mut().append_pair("token", token);
    let _ = opener(url.as_str());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_existing_hub_owner_rejects_trial_gui_and_keeps_production_url_contract() {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let trial = RuntimePaths::trial(root.path(), 49187, installed.path()).unwrap();
        open_existing_hub(&trial, 49187, "synthetic token", |_| {
            panic!("trial must never invoke real browser adapter")
        })
        .unwrap();
        let production = RuntimePaths::production(installed.path()).unwrap();
        let mut invoked = false;
        open_existing_hub(&production, 49187, "synthetic token", |raw| {
            invoked = true;
            let url = url::Url::parse(raw).unwrap();
            assert_eq!(url.host_str(), Some("127.0.0.1"));
            assert_eq!(url.port(), Some(49187));
            assert_eq!(
                url.query_pairs().collect::<Vec<_>>(),
                vec![("token".into(), "synthetic token".into())]
            );
            Err(std::io::Error::other("synthetic unavailable GUI"))
        })
        .unwrap();
        assert!(invoked);
    }
}
