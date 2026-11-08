//! Apply the same trial GUI boundary to an already-running Hub as startup.
use crate::config::RuntimePaths;
/// Opens the dashboard of the Hub on `port`. The browser's command line gets
/// a one-time launch code, never the token: other OS users can read command
/// lines on Linux and macOS (U04). Without a code the plain address opens and
/// the dashboard uses the token it already keeps.
pub fn open_existing_hub(
    paths: &RuntimePaths,
    port: u16,
    launch_code: Option<&str>,
    opener: impl FnOnce(&str) -> std::io::Result<()>,
) -> Result<(), url::ParseError> {
    if !paths.automatic_external_actions_allowed() {
        return Ok(());
    }
    let url = url::Url::parse(&crate::application::hub_runtime::dashboard_url(
        port,
        launch_code,
    ))?;
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
        open_existing_hub(&trial, 49187, Some("0123abcd"), |_| {
            panic!("trial must never invoke real browser adapter")
        })
        .unwrap();
        let production = RuntimePaths::production(installed.path()).unwrap();
        let mut invoked = false;
        open_existing_hub(&production, 49187, Some("0123abcd"), |raw| {
            invoked = true;
            let url = url::Url::parse(raw).unwrap();
            assert_eq!(url.host_str(), Some("127.0.0.1"));
            assert_eq!(url.port(), Some(49187));
            assert_eq!(
                url.query_pairs().collect::<Vec<_>>(),
                vec![("launch".into(), "0123abcd".into())]
            );
            Err(std::io::Error::other("synthetic unavailable GUI"))
        })
        .unwrap();
        assert!(invoked);
        // Without a code nothing secret goes on the command line.
        open_existing_hub(&production, 49187, None, |raw| {
            assert_eq!(raw, "http://127.0.0.1:49187/");
            Ok(())
        })
        .unwrap();
    }
}
