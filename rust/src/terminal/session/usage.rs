//! Numeric usage lives only with the canonical session incarnation, never disk.
use super::*;
impl SessionEngine {
    pub fn session_usage(
        &self,
        binding: SessionBinding,
    ) -> Result<Option<proto::Message>, SessionError> {
        Ok(lock(&self.state).session(binding)?.usage.clone())
    }
    pub fn record_session_usage(
        &self,
        binding: SessionBinding,
        message: proto::Message,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        let session = state.session(binding)?;
        if message.session_id != binding.session.0 || message.r#type != "usage_stat" {
            return Err(SessionError::InvalidRequest(
                "invalid usage identity".into(),
            ));
        }
        session.usage = Some(message.clone());
        Ok(state.route(CoreEffects(vec![CoreEffect::Broadcast(message)])))
    }
    /// Codex Stop hook path admission is lexical and does not read transcript bytes.
    pub fn set_usage_transcript(
        &self,
        binding: SessionBinding,
        raw: &str,
    ) -> Result<(), SessionError> {
        let path = crate::files::scope::clean(std::path::Path::new(raw.trim()));
        if !path.is_absolute()
            || !path
                .extension()
                .and_then(|v| v.to_str())
                .is_some_and(|v| v.eq_ignore_ascii_case("jsonl"))
        {
            return Ok(());
        }
        let mut state = lock(&self.state);
        let session = state.session(binding)?;
        if session.snapshot.provider != "codex" {
            return Ok(());
        }
        let identity = &session.transcript;
        let root = if !identity.codex_home.trim().is_empty() {
            PathBuf::from(identity.codex_home.trim())
        } else if !identity.home_dir.trim().is_empty() {
            PathBuf::from(identity.home_dir.trim()).join(".codex")
        } else {
            return Ok(());
        }
        .join("sessions");
        let root = crate::files::scope::clean(&root);
        let convert = |p: &std::path::Path| {
            let s = p.to_string_lossy().replace('\\', "/");
            if cfg!(windows) {
                crate::proto::unicode::simple_lower(&s)
            } else {
                s
            }
        };
        let child = convert(&path);
        let root = convert(&root);
        let root = root.trim_end_matches('/');
        if child == root
            || child
                .strip_prefix(root)
                .is_some_and(|tail| tail.starts_with('/'))
        {
            session.transcript.native_log_path = path.to_string_lossy().into_owned();
        }
        Ok(())
    }
}
