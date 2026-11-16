//! `many-ai-cli provider add|list|remove|test`, and the history commands
//! (`backup restore`, `reset`, `recover`) when a Hub runs.
//!
//! Registry changes go through the running Hub's provider API when there is
//! one (validation, backup, reload and the dashboard notice all happen there).
//! Without a Hub the same validator and `FileStore` write `providers.d`
//! directly and the Hub reads it on its next start. No command here prints the
//! Hub token or any config value.
use super::{
    provider_args::{AddArgs, TestArgs},
    provider_hub::HubApi,
};
use crate::{
    config::{Config, HeadlessDef, RuntimePaths},
    files::safe_fs::Dir,
    orchestration::headless,
    process::{
        Cancellation, ExitOutcome, ProcessPlan,
        execpath::{NativeFs, Platform, Resolver},
    },
    profile::{
        registry::default_adapters,
        store::{ProviderRegistryStore, StoreError, is_builtin, validate_user_id},
        validation::validate_definition,
    },
    proto::provider::{
        CURRENT_SCHEMA_VERSION, Definition, EffectiveDefinition, HeadlessDefinition,
        LaunchDefinition, ORIGIN_LEGACY, ORIGIN_USER, Summary,
    },
    wrapper::launch::{WrapperArgs, prepare_launch_with_definition},
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

fn error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
/// A path segment that is safe inside `/api/providers/<segment>/...`.
fn segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}
fn shown(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| {
            if part.is_empty() || part.contains(char::is_whitespace) {
                format!("{part:?}")
            } else {
                part.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
pub fn origin_label(origin: &str) -> &'static str {
    match origin {
        ORIGIN_USER => "user",
        ORIGIN_LEGACY => "legacy",
        _ => "built-in",
    }
}
fn diagnostics_errors(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["severity"] == "error")
        .map(|d| {
            format!(
                "{}: {}",
                d["field"].as_str().unwrap_or(""),
                d["message"].as_str().unwrap_or("")
            )
        })
        .collect()
}
fn expected_revision(args: &[String], allow_empty: bool) -> io::Result<String> {
    let mut value = None;
    for pair in args.windows(2) {
        if pair[0] == "--expected-revision" {
            value = Some(pair[1].clone());
        }
    }
    value
        .filter(|value| allow_empty || !value.is_empty())
        .ok_or_else(|| error("--expected-revision is required"))
}

pub struct Registry<'a, H: HubApi> {
    pub hub: Option<&'a H>,
    pub store: &'a Arc<ProviderRegistryStore>,
    pub paths: &'a RuntimePaths,
    pub config: &'a Config,
    pub environment: &'a [String],
    pub cwd: &'a Path,
}
impl<H: HubApi> Registry<'_, H> {
    /// Registry revision and every provider, from the Hub when one runs.
    async fn summaries(&self) -> io::Result<(String, Vec<Summary>)> {
        if let Some(hub) = self.hub {
            let reply = hub.call("GET", "/api/providers", None).await?;
            if reply.status != 200 {
                return Err(error(format!("Hub refused the list: {}", reply.reason())));
            }
            let value = reply.json();
            let list = serde_json::from_value(value["providers"].clone())
                .map_err(|_| error("the Hub's provider list could not be read"))?;
            return Ok((value["revision"].as_str().unwrap_or("").to_owned(), list));
        }
        let snapshot = self.store.snapshot().map_err(io::Error::other)?;
        Ok((
            snapshot.registry.revision().to_owned(),
            snapshot.registry.list(),
        ))
    }
    async fn effective(&self, id: &str) -> io::Result<EffectiveDefinition> {
        if let Some(hub) = self.hub {
            let reply = hub
                .call("GET", &format!("/api/providers/{}", segment(id)), None)
                .await?;
            if reply.status == 404 {
                return Err(error(format!("provider {id:?} was not found")));
            }
            if reply.status != 200 {
                return Err(error(format!("Hub refused the lookup: {}", reply.reason())));
            }
            return serde_json::from_value(reply.json()["provider"].clone())
                .map_err(|_| error("the Hub's provider definition could not be read"));
        }
        self.store
            .snapshot()
            .map_err(io::Error::other)?
            .registry
            .lookup(id)
            .ok_or_else(|| error(format!("provider {id:?} was not found")))
    }
    fn command_on_path(&self, command: &str) -> bool {
        Resolver::new(Platform::native(), self.environment, self.cwd, &NativeFs)
            .look_path(command)
            .is_ok()
    }

    pub async fn list(&self) -> io::Result<String> {
        let (_, list) = self.summaries().await?;
        Ok(list
            .iter()
            .map(|p| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\n",
                    p.id,
                    one_line(&p.display_name),
                    origin_label(p.origin.as_str()),
                    if p.enabled { "enabled" } else { "disabled" },
                    if p.capabilities.headless { "yes" } else { "no" },
                )
            })
            .collect())
    }

    fn approval_dir(&self) -> io::Result<Dir> {
        Dir::open(self.paths.root())?.child_dir("approval-patterns", true)
    }
    fn approval_file(&self, id: &str) -> PathBuf {
        self.paths
            .root()
            .join("approval-patterns")
            .join(format!("{id}.md"))
    }
    fn write_approval_phrases(&self, id: &str, label: &str, phrases: &[String]) -> io::Result<()> {
        let mut body = format!(
            "# {} Approval Patterns\n\n> Written by `many-ai-cli provider add`. One backtick-quoted phrase per bullet; the Hub reads only those.\n\n",
            one_line(label)
        );
        for phrase in phrases {
            body.push_str(&format!("- `{}`\n", phrase.trim()));
        }
        self.approval_dir()?
            .replace(&format!("{id}.md"), body.as_bytes(), 0o600)
    }
    fn remove_approval_phrases(&self, id: &str) {
        if let Ok(dir) = self.approval_dir() {
            let _ = dir.remove_file(&format!("{id}.md"));
        }
    }

    pub async fn add(&self, add: &AddArgs) -> io::Result<String> {
        if is_builtin(&add.id) || add.id == "shell" {
            return Err(error(format!(
                "provider id {:?} is reserved for a built-in; choose another id",
                add.id
            )));
        }
        // The same id rule the registry applies; it also keeps the id safe to use
        // as a file name for the approval phrases below.
        validate_user_id(&add.id).map_err(|e| error(format!("provider id {:?}: {e}", add.id)))?;
        if !self.command_on_path(&add.command) {
            return Err(error(format!(
                "executable {:?} was not found on PATH; install it, or register a small wrapper script that starts it (a CLI that needs its own HOME or config directory belongs in such a wrapper)",
                add.command
            )));
        }
        let (_, existing) = self.summaries().await?;
        if let Some(found) = existing.iter().find(|p| p.id == add.id) {
            return Err(error(format!(
                "provider id {:?} already exists ({}); remove it first or choose another id",
                add.id,
                origin_label(found.origin.as_str())
            )));
        }
        let source = if add.approval_options.is_empty() {
            String::new()
        } else {
            self.approval_file(&add.id).to_string_lossy().into_owned()
        };
        let definition = Definition {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: add.id.clone(),
            display_name: add.label.clone(),
            description: add.description.clone(),
            launch: Some(LaunchDefinition {
                executable: add.command.clone(),
                args: add.args.clone(),
                headless: add
                    .headless
                    .as_ref()
                    .map(|(format, via)| HeadlessDefinition {
                        args: add.headless_args.clone(),
                        format: format.clone(),
                        prompt_via: via.clone(),
                    }),
                ..Default::default()
            }),
            approval_pattern_source: source,
            ..Default::default()
        };
        let raw = serde_json::to_vec(&definition).map_err(io::Error::other)?;
        // Nothing is written until the definition is accepted.
        if let Some(hub) = self.hub {
            let reply = hub
                .call("POST", "/api/providers/validate", Some(raw.clone()))
                .await?;
            let body = reply.json();
            if reply.status != 200 {
                return Err(error(format!(
                    "the Hub refused the definition: {}",
                    reply.reason()
                )));
            }
            if body["valid"] != true {
                return Err(error(format!(
                    "the Hub refused the definition:\n  {}",
                    diagnostics_errors(&body["diagnostics"]).join("\n  ")
                )));
            }
        } else {
            let (_, diagnostics) = validate_definition(&raw, &default_adapters())
                .map_err(|e| error(format!("the definition is not valid: {e}")))?;
            let errors: Vec<_> = diagnostics
                .iter()
                .filter(|d| d.is_error())
                .map(|d| format!("{}: {}", d.field, d.message))
                .collect();
            if !errors.is_empty() {
                return Err(error(format!(
                    "the definition is not valid:\n  {}",
                    errors.join("\n  ")
                )));
            }
        }
        if !add.approval_options.is_empty() {
            self.write_approval_phrases(&add.id, &add.label, &add.approval_options)
                .map_err(|e| error(format!("could not write the approval phrases: {e}")))?;
        }
        let created = self.create(&definition, raw).await;
        let warnings = match created {
            Ok(warnings) => warnings,
            Err(failure) => {
                if !add.approval_options.is_empty() {
                    self.remove_approval_phrases(&add.id);
                }
                return Err(failure);
            }
        };
        Ok(self.added_report(add, &definition, &warnings))
    }
    /// Returns the Hub's warning lines (for example a command it cannot find).
    async fn create(&self, definition: &Definition, raw: Vec<u8>) -> io::Result<Vec<String>> {
        if let Some(hub) = self.hub {
            let reply = hub.call("POST", "/api/providers", Some(raw)).await?;
            if reply.status != 201 {
                return Err(error(format!("the Hub did not add it: {}", reply.reason())));
            }
            return Ok(reply.json()["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|d| {
                    format!(
                        "{}: {}",
                        d["field"].as_str().unwrap_or(""),
                        d["message"].as_str().unwrap_or("")
                    )
                })
                .collect());
        }
        let _guard = self
            .store
            .mutation_gate
            .lock()
            .map_err(|_| error("provider mutation lock unavailable"))?;
        match self.store.definitions.create_new(definition) {
            Ok(()) => {}
            Err(StoreError::AlreadyExists) => {
                return Err(error(format!(
                    "provider id {:?} already exists",
                    definition.id
                )));
            }
            Err(e) => return Err(error(format!("could not save the provider: {e}"))),
        }
        if let Err(e) = self
            .store
            .history
            .backup_snapshot(&definition.id, definition, "create")
        {
            let _ = self.store.definitions.delete(&definition.id);
            return Err(error(format!("could not back up the new provider: {e}")));
        }
        self.store.reload().map_err(|e| {
            error(format!(
                "saved, but the registry could not be reloaded: {e}"
            ))
        })?;
        Ok(vec![])
    }
    fn added_report(&self, add: &AddArgs, definition: &Definition, warnings: &[String]) -> String {
        let mut out = format!("added provider {} ({})\n", add.id, one_line(&add.label));
        let mut interactive = vec![add.command.clone()];
        interactive.extend(add.args.clone());
        out.push_str(&format!("interactive: {}\n", shown(&interactive)));
        match &add.headless {
            Some((format, via)) => {
                let def = HeadlessDef {
                    args: add.headless_args.clone(),
                    format: format.clone(),
                    prompt_via: via.clone(),
                };
                let mut argv = vec![add.command.clone()];
                argv.extend(headless::build_argv(&def, &add.args, "<prompt>"));
                out.push_str(&format!(
                    "headless: yes: {}{} (output format {format})\n",
                    shown(&argv),
                    if via == "stdin" { "  (prompt on stdin)" } else { "" },
                ));
            }
            None => out.push_str(
                "headless: no (add --headless-arg / --prompt-via to let orchestrate run it as a child)\n",
            ),
        }
        if !definition.approval_pattern_source.is_empty() {
            out.push_str(&format!(
                "approval phrases: {} written to {}\n",
                add.approval_options.len(),
                definition.approval_pattern_source
            ));
        }
        for warning in warnings {
            out.push_str(&format!("warning: {warning}\n"));
        }
        if self.hub.is_some() {
            out.push_str("The running Hub reloaded the registry; open dashboards show a notice with an undo.\n");
        } else {
            out.push_str("No Hub is running; the Hub will read it on its next start.\n");
        }
        out.push_str(&format!("next: many-ai-cli provider test {}\n", add.id));
        out
    }

    pub async fn remove(&self, id: &str) -> io::Result<String> {
        let (revision, list) = self.summaries().await?;
        let Some(found) = list.iter().find(|p| p.id == id) else {
            return Err(error(format!("provider {id:?} was not found")));
        };
        match found.origin.as_str() {
            ORIGIN_USER => {}
            ORIGIN_LEGACY => {
                return Err(error(format!(
                    "provider {id:?} is defined in config.yaml (custom_providers); remove it there. `provider remove` only removes providers registered with `provider add` or the dashboard"
                )));
            }
            _ => {
                return Err(error(format!(
                    "provider {id:?} is built in and cannot be removed; disable it from Settings instead"
                )));
            }
        }
        if let Some(hub) = self.hub {
            let reply = hub
                .call(
                    "DELETE",
                    &format!(
                        "/api/providers/{}?expected_revision={}",
                        segment(id),
                        segment(&revision)
                    ),
                    None,
                )
                .await?;
            if reply.status != 200 {
                return Err(error(format!(
                    "the Hub did not remove it: {}",
                    reply.reason()
                )));
            }
        } else {
            let _guard = self
                .store
                .mutation_gate
                .lock()
                .map_err(|_| error("provider mutation lock unavailable"))?;
            let current = self
                .store
                .snapshot()
                .map_err(io::Error::other)?
                .registry
                .lookup(id)
                .ok_or_else(|| error(format!("provider {id:?} was not found")))?;
            self.store
                .history
                .backup_snapshot(id, &current.definition, "before-delete")
                .map_err(|e| error(format!("could not back up the provider first: {e}")))?;
            self.store
                .definitions
                .delete(id)
                .map_err(|e| error(format!("could not remove the provider: {e}")))?;
            self.store.reload().map_err(|e| {
                error(format!(
                    "removed, but the registry could not be reloaded: {e}"
                ))
            })?;
        }
        Ok(format!(
            "removed provider {id}\nA backup (reason before-delete) is kept: many-ai-cli provider backup list {id}\n"
        ))
    }

    pub async fn test(&self, args: &TestArgs) -> io::Result<String> {
        let effective = self.effective(&args.id).await?;
        let launch = prepare_launch_with_definition(
            self.config,
            &args.id,
            WrapperArgs {
                headless: true,
                ..Default::default()
            },
            Some(&effective),
        )
        .map_err(|e| error(e.to_string()))?;
        let def = launch.headless_definition.as_ref().ok_or_else(|| {
            error(format!(
                "provider {:?} has no headless definition; register it with --headless-arg or --prompt-via (see `provider add --help`)",
                args.id
            ))
        })?;
        let argument_prompt =
            crate::config::normalize_headless_prompt_via(&def.prompt_via) == "arg";
        let argv = headless::build_argv(def, &launch.provider_args, &args.prompt);
        let resolver = Resolver::new(Platform::native(), self.environment, self.cwd, &NativeFs);
        let command = resolver
            .resolve_provider(&launch.provider, launch.custom_argv.as_deref(), &argv)
            .map_err(|e| error(format!("could not start {:?}: {e}", args.id)))?;
        if argument_prompt && command.shell_shim.is_some() {
            return Err(error(
                "a headless argument prompt cannot run through a Windows cmd shim; use a native executable, or register with --prompt-via stdin",
            ));
        }
        let mut env = std::collections::BTreeMap::new();
        // The tested CLI is not a Hub session: it gets no Hub credentials.
        for name in [
            "MANY_AI_CLI_HUB_TOKEN",
            "MANY_AI_CLI_HUB_PORT",
            "MANY_AI_CLI_SESSION_ID",
        ] {
            env.insert(name.into(), None);
        }
        let process = ProcessPlan {
            executable: command.executable.into(),
            args: command.args.into_iter().map(Into::into).collect(),
            cwd: self.cwd.to_path_buf(),
            env,
            stdin: vec![],
            timeout: Duration::from_secs(args.timeout_secs),
            output_cap: 4 * 1024 * 1024,
            pipe_drain_timeout: Duration::from_secs(2),
        };
        let mut text = String::new();
        let started = Instant::now();
        let result = headless::run(
            headless::Spec {
                process,
                prompt: args.prompt.clone(),
                prompt_via: def.prompt_via.clone(),
                format: def.format.clone(),
                raw_logs: None,
                event_capacity: 1024,
            },
            &Cancellation::default(),
            |bytes| text.push_str(&String::from_utf8_lossy(bytes)),
        )
        .await
        .map_err(|e| error(format!("could not run {:?}: {e}", args.id)))?;
        let elapsed = started.elapsed().as_secs_f64();
        let exit = match &result.process_output.outcome {
            ExitOutcome::TimedOut => {
                format!("none (killed after the {} s timeout)", args.timeout_secs)
            }
            _ => result.exit_code.to_string(),
        };
        let mut report = format!("exit code: {exit}\nelapsed: {elapsed:.1} s\n");
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .collect();
        report.push_str("output (first 10 lines):\n");
        for line in lines.iter().take(10) {
            report.push_str(&format!("  {line}\n"));
        }
        if lines.is_empty() {
            report.push_str("  (no output)\n");
        }
        if result.state == "completed" {
            Ok(report)
        } else {
            Err(error(format!("provider test failed\n{report}")))
        }
    }

    /// `backup restore`, `reset --distributed` and non-list `recover` through the
    /// running Hub, so it reloads and tells open dashboards. `None` when the
    /// command is not one of those or no Hub runs: the in-process path applies.
    pub async fn history_via_hub(&self, args: &[String]) -> io::Result<Option<String>> {
        let Some(hub) = self.hub else {
            return Ok(None);
        };
        let (path, body, word) = match args.first().map(String::as_str) {
            Some("backup")
                if args.get(1).map(String::as_str) == Some("restore") && args.len() >= 4 =>
            {
                let (id, backup) = (&args[2], &args[3]);
                (
                    format!(
                        "/api/providers/{}/backups/{}/restore",
                        segment(id),
                        segment(backup)
                    ),
                    serde_json::json!({"expected_revision": expected_revision(&args[4..], false)?}),
                    "restored",
                )
            }
            Some("reset")
                if args.get(1).map(String::as_str) == Some("--distributed") && args.len() >= 3 =>
            {
                let id = &args[2];
                if !is_builtin(id) {
                    return Err(error(format!("built-in provider {id:?} was not found")));
                }
                (
                    format!("/api/providers/{}/reset", segment(id)),
                    serde_json::json!({"expected_revision": expected_revision(&args[3..], true)?}),
                    "reset",
                )
            }
            Some("recover")
                if args.len() >= 2 && args.get(2).map(String::as_str) != Some("--list") =>
            {
                (
                    format!("/api/providers/{}/recovery", segment(&args[1])),
                    serde_json::json!({"revision": args.get(2).map(String::as_str).unwrap_or("")}),
                    "recovered",
                )
            }
            _ => return Ok(None),
        };
        let reply = hub
            .call(
                "POST",
                &path,
                Some(serde_json::to_vec(&body).map_err(io::Error::other)?),
            )
            .await?;
        if reply.status != 200 {
            return Err(error(format!("the Hub refused it: {}", reply.reason())));
        }
        Ok(Some(format!(
            "{word}\t{}\n",
            reply.json()["revision"].as_str().unwrap_or("")
        )))
    }
}

#[cfg(test)]
#[path = "provider_registry_tests.rs"]
mod tests;
