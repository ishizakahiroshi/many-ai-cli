//! Preview authorization is bound to scrubbed log bytes and consumed once.
use super::{Arc, Cancellation, ConfigStore, CoreFuture, Path, PathBuf, RuntimePaths, io, report};
use crate::{
    files::safe_fs::Dir,
    proto::core::{LiveSessionId, SessionCore},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
    sync::Mutex,
    time::{Duration, Instant},
};
pub trait BugReportIo: Send + Sync {
    fn look_path(&self, name: &str) -> io::Result<String>;
    fn create_secret_gist<'a>(
        &'a self,
        executable: &'a str,
        markdown: &'a str,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<String>>;
}
pub struct BugReportDependencies {
    pub config: Arc<ConfigStore>,
    pub paths: RuntimePaths,
    pub core: Arc<dyn SessionCore>,
    pub version: String,
    pub platform: String,
    pub arch: String,
    pub runtime_version: String,
    pub io: Arc<dyn BugReportIo>,
}
type PreviewTokens = BTreeMap<String, ([u8; 32], Instant)>;
pub struct BugReport {
    deps: BugReportDependencies,
    previews: Mutex<PreviewTokens>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct PreviewRequest {
    pub session_id: Option<i64>,
    pub include_recent_log_lines: i64,
    pub locale: String,
    pub user_agent: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct FinalizeRequest {
    pub symptom: String,
    pub reproduction: String,
    pub environment_markdown: String,
    pub locale: String,
    pub include_session_log: bool,
    pub log_markdown: String,
    pub log_preview_token: String,
}
#[derive(Serialize)]
pub struct PreviewResponse {
    pub markdown: String,
    pub environment_markdown: String,
    pub warnings: Option<Vec<String>>,
    pub gh_available: bool,
    pub session_log_recorded: bool,
    pub log_attachment_available: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub log_markdown: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub log_saved_path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub log_preview_token: String,
}
#[derive(Serialize)]
pub struct FinalizeResponse {
    pub markdown: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub saved_path: String,
    pub warnings: Vec<String>,
}
#[derive(Debug)]
pub struct ReportError {
    pub status: u16,
    pub code: &'static str,
    pub detail: &'static str,
}
fn error(status: u16, code: &'static str, detail: &'static str) -> ReportError {
    ReportError {
        status,
        code,
        detail,
    }
}
impl BugReport {
    pub fn new(deps: BugReportDependencies) -> Arc<Self> {
        Arc::new(Self {
            deps,
            previews: Mutex::new(BTreeMap::new()),
        })
    }
    pub fn preview(&self, request: PreviewRequest) -> Result<PreviewResponse, ReportError> {
        if request.include_recent_log_lines != 0 && request.include_recent_log_lines != 200 {
            return Err(error(
                400,
                "invalid_log_line_count",
                "include_recent_log_lines must be 0 or 200",
            ));
        }
        let cfg = self
            .deps
            .config
            .snapshot()
            .map_err(|_| {
                error(
                    503,
                    "configuration_unavailable",
                    "configuration is unavailable",
                )
            })?
            .config;
        let details = request
            .session_id
            .and_then(|id| self.deps.core.details(LiveSessionId(id)));
        let mut warnings = vec![];
        if request.session_id.is_some() && details.is_none() {
            warnings.push("session_not_found".into())
        }
        let (provider, model, path) = details
            .as_ref()
            .map(|d| {
                (
                    d.snapshot.provider.as_str(),
                    d.snapshot.model.as_str(),
                    d.snapshot.jsonl_path.as_str(),
                )
            })
            .unwrap_or(("", "", ""));
        let gh = self
            .deps
            .io
            .look_path("gh")
            .is_ok_and(|p| !p.trim().is_empty());
        let env = report::collect(
            &cfg,
            report::EnvironmentInput {
                version: &self.deps.version,
                platform: &self.deps.platform,
                arch: &self.deps.arch,
                runtime_version: &self.deps.runtime_version,
                provider,
                model,
                user_agent: &request.user_agent,
            },
        );
        let environment = report::render_environment(&env, &request.locale);
        let mut response = PreviewResponse {
            markdown: report::render_markdown(&request.locale, "", "", &environment),
            environment_markdown: environment,
            warnings: if request.session_id.is_some() && details.is_some() {
                None
            } else {
                Some(warnings)
            },
            gh_available: gh,
            session_log_recorded: !path.is_empty(),
            log_attachment_available: details.is_some() && gh && !path.is_empty(),
            log_markdown: String::new(),
            log_saved_path: String::new(),
            log_preview_token: String::new(),
        };
        if request.include_recent_log_lines == 0 {
            return Ok(response);
        }
        if details.is_none() {
            return Err(error(
                404,
                "active_session_required",
                "active session is required",
            ));
        }
        if !gh {
            return Err(error(409, "gh_not_available", "gh CLI is required"));
        }
        let (text, truncated) = self.tail(&cfg, path).map_err(|_| {
            error(
                404,
                "session_log_unavailable",
                "session log is not available",
            )
        })?;
        let log = canonical(&format!(
            "## many-ai-cli session log (last 200 lines, redacted)\n\n{}\n",
            report::redact(&text)
                .split('\n')
                .map(|line| format!("    {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
        response.log_saved_path = self.save(&log).map_err(|_| {
            error(
                500,
                "report_save_failed",
                "failed to save redacted report log",
            )
        })?;
        response.log_preview_token = self.remember(&log).map_err(|_| {
            error(
                500,
                "log_preview_failed",
                "failed to prepare session log preview",
            )
        })?;
        response.log_markdown = log;
        if truncated {
            response
                .warnings
                .get_or_insert_with(Vec::new)
                .push("session_log_byte_limit".into())
        }
        Ok(response)
    }
    pub async fn finalize(
        &self,
        request: FinalizeRequest,
        cancel: &Cancellation,
    ) -> Result<FinalizeResponse, ReportError> {
        if request.symptom.trim().is_empty() {
            return Err(error(400, "symptom_required", "symptom is required"));
        }
        let mut markdown = report::render_markdown(
            &request.locale,
            &request.symptom,
            &request.reproduction,
            &request.environment_markdown,
        );
        if request.include_session_log {
            let log = canonical(&request.log_markdown);
            if log.is_empty() {
                return Err(error(
                    400,
                    "log_preview_required",
                    "previewed session log is required",
                ));
            }
            if !self.consume(&request.log_preview_token, &log) {
                return Err(error(
                    400,
                    "log_preview_required",
                    "valid session log preview is required",
                ));
            }
            let exe = match self.deps.io.look_path("gh") {
                Ok(p) if !p.trim().is_empty() => p,
                _ => return self.fallback(&request.symptom, &markdown, &log, "gh_not_available"),
            };
            let url = match self.deps.io.create_secret_gist(&exe, &log, cancel).await {
                Ok(url) => url,
                Err(_) => {
                    return self.fallback(&request.symptom, &markdown, &log, "gist_create_failed");
                }
            };
            let Some(url) = validated_gist_url(&url) else {
                return self.fallback(&request.symptom, &markdown, &log, "gist_url_rejected");
            };
            markdown = report::redact(&format!(
                "{}\n\n[log-attachment]({url})\n",
                markdown.trim_end_matches('\n')
            ));
        }
        let title = report::default_title(&request.symptom);
        if let Some(url) = report::issue_url(&title, &markdown) {
            return Ok(FinalizeResponse {
                markdown: report::redact(&markdown),
                url,
                saved_path: String::new(),
                warnings: vec![],
            });
        }
        let saved = self
            .save(&markdown)
            .map_err(|_| error(500, "report_save_failed", "failed to save redacted report"))?;
        Ok(FinalizeResponse {
            markdown: report::redact(&markdown),
            url: report::issue_url(&title, "").unwrap_or_default(),
            saved_path: saved,
            warnings: vec!["issue_url_too_long".into()],
        })
    }
    fn fallback(
        &self,
        symptom: &str,
        markdown: &str,
        log: &str,
        warning: &str,
    ) -> Result<FinalizeResponse, ReportError> {
        let text = report::redact(&format!(
            "{}\n\n## Scrubbed session log (local fallback)\n\n{}\n",
            markdown.trim_end_matches('\n'),
            log.trim()
        ));
        let saved = self
            .save(&text)
            .map_err(|_| error(500, "report_save_failed", "failed to save redacted report"))?;
        let title = report::default_title(symptom);
        Ok(FinalizeResponse {
            markdown: report::redact(markdown),
            url: report::issue_url(&title, markdown)
                .or_else(|| report::issue_url(&title, ""))
                .unwrap_or_default(),
            saved_path: saved,
            warnings: vec![warning.into()],
        })
    }
    fn save(&self, text: &str) -> io::Result<String> {
        let root =
            Dir::open_or_create_private(self.deps.paths.root())?.child_dir("reports", true)?;
        let name = format!(
            "report_{}.md",
            chrono::Local::now().format("%Y%m%d_%H%M%S.%9f")
        );
        root.replace(&name, report::redact(text).as_bytes(), 0o600)?;
        Ok(PathBuf::from("~")
            .join(".many-ai-cli/reports")
            .join(name)
            .to_string_lossy()
            .into_owned())
    }
    fn remember(&self, text: &str) -> io::Result<String> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| io::Error::other("generate preview token"))?;
        let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let now = Instant::now();
        let mut previews = self
            .previews
            .lock()
            .map_err(|_| io::Error::other("preview lock unavailable"))?;
        previews.retain(|_, (_, expiry)| *expiry > now);
        if previews.len() >= 64
            && let Some(key) = previews
                .iter()
                .min_by_key(|(_, (_, expiry))| *expiry)
                .map(|(key, _)| key.clone())
        {
            previews.remove(&key);
        }
        previews.insert(
            token.clone(),
            (
                Sha256::digest(canonical(text).as_bytes()).into(),
                now + Duration::from_secs(900),
            ),
        );
        Ok(token)
    }
    fn consume(&self, token: &str, text: &str) -> bool {
        if token.len() != 64 {
            return false;
        }
        let Some((hash, expiry)) = self.previews.lock().ok().and_then(|mut p| p.remove(token))
        else {
            return false;
        };
        let got: [u8; 32] = Sha256::digest(canonical(text).as_bytes()).into();
        hash.iter().zip(got).fold(0u8, |a, (x, y)| a | (*x ^ y)) == 0 && expiry > Instant::now()
    }
    fn tail(&self, cfg: &crate::config::Config, path: &str) -> io::Result<(String, bool)> {
        if cfg.hub.log_dir.is_empty()
            || !Path::new(path)
                .extension()
                .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("jsonl"))
        {
            return Err(io::Error::other("invalid session log path"));
        }
        let logs = self
            .deps
            .paths
            .clone()
            .with_log_dir(Path::new(&cfg.hub.log_dir))?
            .resource(crate::config::Resource::Logs)
            .join("sessions");
        let root = Dir::open(&logs)?;
        let relative = Path::new(path)
            .strip_prefix(&logs)
            .map_err(|_| io::Error::other("session log outside log directory"))?;
        let mut dir = root;
        let components = relative.components().collect::<Vec<_>>();
        for part in components.iter().take(components.len().saturating_sub(1)) {
            let std::path::Component::Normal(name) = part else {
                return Err(io::Error::other("invalid session log component"));
            };
            dir = dir.child_dir(
                name.to_str()
                    .ok_or_else(|| io::Error::other("invalid session log name"))?,
                false,
            )?;
        }
        let Some(std::path::Component::Normal(name)) = components.last() else {
            return Err(io::Error::other("invalid session log basename"));
        };
        let mut file = dir.open_file(
            name.to_str()
                .ok_or_else(|| io::Error::other("invalid session log name"))?,
            false,
        )?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::other("session log is not regular"));
        }
        let start = metadata.len().saturating_sub(512 * 1024);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = vec![];
        file.take(metadata.len() - start).read_to_end(&mut bytes)?;
        let mut truncated = start > 0;
        if truncated && let Some(newline) = bytes.iter().position(|b| *b == b'\n') {
            bytes.drain(..=newline);
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut lines = text
            .trim_end_matches(['\r', '\n'])
            .split('\n')
            .collect::<Vec<_>>();
        if lines.len() > 200 {
            lines.drain(..lines.len() - 200);
            truncated = true
        }
        Ok((report::redact(&lines.join("\n")), truncated))
    }
}
fn canonical(text: &str) -> String {
    report::redact(text).trim().into()
}
pub fn validated_gist_url(raw: &str) -> Option<String> {
    if raw.is_empty()
        || raw.len() > 2048
        || !raw.starts_with("https://gist.github.com/")
        || raw.contains(['[', ']', '(', ')', '<', '>', ' ', '\t', '\r', '\n'])
    {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("gist.github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() == "/"
    {
        None
    } else {
        Some(url.to_string())
    }
}
#[cfg(test)]
mod tests;
