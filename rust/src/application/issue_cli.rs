//! Fixed CLI issue preview/confirmation with explicit external action ownership.
#[path = "maintenance_cli/flags.rs"]
pub mod flags;
use crate::{
    application::diagnostics::{DiagnosticIo, NativeDiagnosticIo, report},
    config::{ConfigStore, RuntimePaths},
    files::safe_fs::Dir,
    process::Cancellation,
    proto::{core::CoreFuture, provider::BUILTIN_PROVIDER_IDS},
};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
pub trait IssueIo: Send + Sync {
    fn look_path(&self, name: &str) -> io::Result<String>;
    fn open_web<'a>(
        &'a self,
        executable: &'a str,
        args: Vec<String>,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<Vec<u8>>>;
}
pub struct NativeIssueIo {
    pub actor: NativeDiagnosticIo,
}
impl IssueIo for NativeIssueIo {
    fn look_path(&self, name: &str) -> io::Result<String> {
        self.actor.look_path(name)
    }
    fn open_web<'a>(
        &'a self,
        executable: &'a str,
        args: Vec<String>,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            let output = self
                .actor
                .command(executable, args, &self.actor.cwd, Duration::ZERO, cancel)
                .await?;
            if !output.success {
                return Err(io::Error::other("gh could not open issue preview"));
            }
            Ok(output.bytes)
        })
    }
}
pub struct IssueDependencies {
    pub config: Arc<ConfigStore>,
    pub paths: RuntimePaths,
    pub environment: Vec<String>,
    pub version: String,
    pub platform: String,
    pub arch: String,
    pub runtime_version: String,
    pub io: Arc<dyn IssueIo>,
}
pub struct IssueCli {
    deps: IssueDependencies,
}
impl IssueCli {
    pub fn new(deps: IssueDependencies) -> Self {
        Self { deps }
    }
    pub async fn run(
        &self,
        args: &[String],
        input: &mut impl BufRead,
        output: &mut impl Write,
        prompts: &mut impl Write,
        cancel: &Cancellation,
    ) -> io::Result<()> {
        let flags = flags::parse(
            args,
            &[
                ("title", flags::Kind::String),
                ("provider", flags::Kind::String),
                ("dry-run", flags::Kind::Bool),
            ],
        )
        .map_err(io::Error::other)?;
        if flags.help {
            return Ok(());
        }
        if flags.positional.len() > 1 {
            return Err(io::Error::other(
                "issue accepts at most one positional title",
            ));
        }
        if !flags.positional.is_empty() && !flags.value("title").trim().is_empty() {
            return Err(io::Error::other(
                "issue title must be provided either positionally or with --title, not both",
            ));
        }
        let provider = flags.value("provider").trim();
        if !provider.is_empty() && !BUILTIN_PROVIDER_IDS.contains(&provider) {
            return Err(io::Error::other(format!(
                "unsupported provider {provider:?}"
            )));
        }
        let auto = self.deps.environment.iter().rev().find_map(|entry| {
            entry
                .split_once('=')
                .filter(|(name, _)| {
                    if cfg!(windows) {
                        name.eq_ignore_ascii_case("MANY_AI_CLI_AUTO")
                    } else {
                        *name == "MANY_AI_CLI_AUTO"
                    }
                })
                .map(|(_, value)| value)
        });
        if auto == Some("1") && !flags.boolean("dry-run") {
            return Err(io::Error::other(
                "issue is disabled when MANY_AI_CLI_AUTO=1; use --dry-run to inspect the report",
            ));
        }
        let requested = flags
            .positional
            .first()
            .map(String::as_str)
            .unwrap_or(flags.value("title"))
            .trim();
        let mut symptom = requested.to_owned();
        if symptom.is_empty() {
            write!(prompts, "症状を1行で入力してください: ")?;
            prompts.flush()?;
            input.read_line(&mut symptom)?;
            symptom = symptom.trim().into();
            if symptom.is_empty() {
                return Err(io::Error::other("symptom is required"));
            }
        }
        let cfg = self
            .deps
            .config
            .snapshot()
            .map_err(io::Error::other)?
            .config;
        let environment = report::collect(
            &cfg,
            report::EnvironmentInput {
                version: &self.deps.version,
                platform: &self.deps.platform,
                arch: &self.deps.arch,
                runtime_version: &self.deps.runtime_version,
                provider,
                model: "",
                user_agent: "",
            },
        );
        let markdown = report::render_markdown(
            "ja",
            &symptom,
            "",
            &report::render_environment(&environment, "ja"),
        );
        let title = if requested.is_empty() {
            report::default_title(&symptom)
        } else {
            report::redact(requested)
        };
        if flags.boolean("dry-run") {
            writeln!(output, "{markdown}")?;
            return Ok(());
        }
        write!(
            output,
            "GitHub Issue preview\n\nTitle: {title}\n\n{markdown}\n"
        )?;
        write!(
            prompts,
            "この内容で GitHub の Issue 作成画面を開きますか? [y/N]: "
        )?;
        prompts.flush()?;
        let mut answer = String::new();
        input.read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            writeln!(output, "Issue creation cancelled.")?;
            return Ok(());
        }
        if cancel.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "issue cancelled",
            ));
        }
        let title = report::redact(&title);
        let markdown = report::redact(&markdown);
        let Some(url) = report::issue_url(&title, &markdown) else {
            return Err(self.fallback(&markdown, "report is too long for a GitHub issue URL")?);
        };
        let executable = match self.deps.io.look_path("gh") {
            Ok(executable) => executable,
            Err(_) => {
                writeln!(output, "{url}")?;
                return Ok(());
            }
        };
        let args = vec![
            "issue".into(),
            "create".into(),
            "--repo".into(),
            "ishizakahiroshi/many-ai-cli".into(),
            "--web".into(),
            "--title".into(),
            title,
            "--body".into(),
            markdown.clone(),
        ];
        match self.deps.io.open_web(&executable, args, cancel).await {
            Ok(bytes) => {
                output.write_all(report::redact(&String::from_utf8_lossy(&bytes)).as_bytes())
            }
            Err(_) => Err(self.fallback(&markdown, "gh could not open the issue preview")?),
        }
    }
    fn fallback(&self, markdown: &str, reason: &str) -> io::Result<io::Error> {
        let dir = Dir::open_or_create_private_components(self.deps.paths.root())?
            .child_dir("reports", true)?;
        let name = format!(
            "report_{}.md",
            chrono::Local::now().format("%Y%m%d_%H%M%S.%9f")
        );
        dir.replace(&name, report::redact(markdown).as_bytes(), 0o600)?;
        let display = PathBuf::from("~/.many-ai-cli/reports").join(name);
        Ok(io::Error::other(format!(
            "{reason}; redacted report saved to {}",
            display.display()
        )))
    }
}
#[cfg(test)]
mod tests;
