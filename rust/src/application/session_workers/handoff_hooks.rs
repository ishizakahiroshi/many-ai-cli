use super::*;
use crate::hub::handoff_routes::HandoffHttpHooks;
#[cfg(test)]
mod tests;
impl SessionWorkers {
    pub(super) async fn finish_handoff_note(
        &self,
        binding: SessionBinding,
        path: &Path,
    ) -> Result<(), SessionError> {
        let exists = std::fs::metadata(path).is_ok_and(|m| !m.is_dir());
        if exists && self.paths.is_trial() {
            transcript_path::check_trial_path(&self.paths, path)?;
        }
        if exists
            && let Err(error) = self.append_handoff(
                binding,
                Record {
                    kind: crate::orchestration::handoff::KIND_NOTE.into(),
                    note: path.to_string_lossy().into_owned(),
                    ..Default::default()
                },
            )
        {
            (self.warning)("handoff note record append", &error);
        }
        self.apply(self.core()?.broadcast_ui(proto::Message {
            r#type: "handoff_note".into(),
            session_id: binding.session.0,
            note_ok: exists,
            note_path: if exists {
                path.to_string_lossy().into_owned()
            } else {
                String::new()
            },
            ..Default::default()
        }))
        .await
    }
}
impl HandoffHttpHooks for SessionWorkers {
    fn ensure_transcript(&self, id: LiveSessionId, now: Timestamp) -> Result<(), SessionError> {
        if !self.config()?.handoff.enabled_or_default() {
            return Ok(());
        }
        let Some(details) = self.core()?.details(id) else {
            return Ok(());
        };
        let Some(path) = transcript_path::resolve(&self.paths, &details.transcript) else {
            return Ok(());
        };
        if self.paths.is_trial() {
            transcript_path::check_trial_path(&self.paths, &path)?;
        }
        let records = match self.handoff.read_session(id.0) {
            Ok(records) => records,
            Err(_) => return Ok(()),
        };
        let text = path.to_string_lossy().into_owned();
        if records.is_empty() || records.iter().any(|record| record.transcript == text) {
            return Ok(());
        }
        let snapshot = details.snapshot;
        self.handoff
            .append(
                id.0,
                Record {
                    kind: KIND_TRANSCRIPT.into(),
                    transcript: text,
                    provider: snapshot.provider,
                    cwd: snapshot.cwd,
                    branch: snapshot.branch,
                    model: snapshot.model,
                    subscription_id: snapshot.subscription_profile_id,
                    ..Default::default()
                },
                now,
            )
            .map_err(|_| storage_error("handoff transcript record failed"))
    }
    fn request_note<'a>(
        &'a self,
        id: LiveSessionId,
        now: Timestamp,
    ) -> CoreFuture<'a, Result<PathBuf, &'static str>> {
        Box::pin(async move {
            let cfg = self.config().map_err(|_| "handoff_disabled")?;
            if !cfg.handoff.enabled_or_default() {
                return Err("handoff_disabled");
            }
            let path = self
                .handoff
                .note_path_for(id.0)
                .map_err(|_| "note_path_unavailable")?;
            if crate::files::safe_fs::Dir::open_or_create_private(
                path.parent().ok_or("note_path_unavailable")?,
            )
            .is_err()
            {
                (self.warning)(
                    "handoff note dir create failed",
                    &storage_error("private memo directory unavailable"),
                );
            }
            let core = self.core().map_err(|_| "session_not_writable")?;
            let binding = core.details(id).ok_or("session_not_writable")?.binding;
            let timer = self
                .tasks
                .effect_permit()
                .map_err(|_| "session_not_writable")?;
            let seq = core.begin_handoff_note(binding, path.clone())?;
            let weak = self.this.clone();
            let cancellation = timer.cancellation().token().clone();
            drop(timer.start(async move {
                tokio::select! { _=tokio::time::sleep(Duration::from_secs(60))=>{}, _=cancellation.cancelled()=>return }
                if let Some(owner)=weak.upgrade()
                    && let Ok(core)=owner.core()
                    && let Err(error)=owner.apply(core.expire_handoff_note(binding,seq)).await {
                    (owner.warning)("handoff note timeout",&error);
                }
            }));
            let prompt = note_prompt(
                &path,
                cfg.user_prefs.display.lang.trim().is_empty()
                    || cfg.user_prefs.display.lang.eq_ignore_ascii_case("ja"),
            );
            let receipt = core
                .submit(
                    binding,
                    InputRequest {
                        bytes: format!("\x1b[200~{prompt}\x1b[201~\r").into_bytes(),
                        authority: InputAuthority::Internal,
                    },
                    now,
                    &TaskCancellation::default(),
                )
                .await;
            if !matches!(
                receipt.disposition,
                InputDisposition::TransportWritten { .. } | InputDisposition::Deferred { .. }
            ) {
                (self.warning)(
                    "handoff note prompt delivery",
                    &SessionError::Transport("memo prompt delivery failed".into()),
                );
            }
            Ok(path)
        })
    }
    fn warning(&self, operation: &'static str) {
        (self.warning)(
            operation,
            &storage_error("handoff owned storage unavailable"),
        );
    }
}
fn note_prompt(path: &Path, ja: bool) -> String {
    let path = path.to_string_lossy();
    if ja {
        format!(
            "[many-ai-cli] 残量が少ないので引き継ぎメモを書いてください。{path} に markdown で、次の一手・未検証の前提・開いている論点・触っていた md のパスを書き、書き終えたら [MANY-AI-CLI-HANDOFF-NOTE] written [/MANY-AI-CLI-HANDOFF-NOTE] を 1 行で出力してください（コードブロックで囲まない）。"
        )
    } else {
        format!(
            "[many-ai-cli] This session is running low on quota; please write a handoff memo. Write markdown to {path} covering the next step, unverified assumptions, open questions and the path of any plan/bugfix md you had open, then output [MANY-AI-CLI-HANDOFF-NOTE] written [/MANY-AI-CLI-HANDOFF-NOTE] on a single line (not wrapped in a code block)."
        )
    }
}
