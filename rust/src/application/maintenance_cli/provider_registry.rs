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
    provider_checks,
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
        capability_report, declared,
        registry::default_adapters,
        store::{ProviderRegistryStore, StoreError, is_builtin, validate_user_id},
        validation::validate_definition,
    },
    proto::provider::{
        CURRENT_SCHEMA_VERSION, Definition, EffectiveDefinition, HeadlessDefinition,
        LinksDefinition, ModelDefinition, ORIGIN_LEGACY, ORIGIN_USER, Summary,
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
/// A line of another program's output made safe to print: ANSI/OSC sequences
/// removed, any other control character shown as a space.
fn printable_line(text: &str) -> String {
    one_line(&crate::approval::identity::strip_ansi(text))
}
const SESSION_VARIABLE_PREFIX: &str = "MANY_AI_CLI_";
/// Every variable of a Hub session this process inherited, to be removed for a
/// provider under test: the Hub token, port, session id and local proof, and
/// anything else with the `MANY_AI_CLI_` prefix (case-insensitive on Windows).
pub(super) fn hub_session_variables(
    environment: &[String],
    inherited: impl IntoIterator<Item = std::ffi::OsString>,
) -> std::collections::BTreeMap<std::ffi::OsString, Option<std::ffi::OsString>> {
    let mut env = std::collections::BTreeMap::new();
    for name in crate::wrapper::launch::SESSION_ENV
        .iter()
        .copied()
        .chain(["MANY_AI_CLI_HUB_PORT"])
    {
        env.insert(name.into(), None);
    }
    let from_entries = environment
        .iter()
        .filter_map(|entry| entry.split_once('=').map(|(name, _)| name.into()));
    for name in inherited.into_iter().chain(from_entries) {
        let Some(text) = name.to_str() else { continue };
        let matches = if cfg!(windows) {
            text.to_ascii_uppercase()
                .starts_with(SESSION_VARIABLE_PREFIX)
        } else {
            text.starts_with(SESSION_VARIABLE_PREFIX)
        };
        if matches
            || crate::wrapper::launch::SESSION_ENV
                .iter()
                .any(|known| text.eq_ignore_ascii_case(known))
        {
            env.insert(name, None);
        }
    }
    env
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
    /// Creates the phrase file, never replacing one that exists: whatever is
    /// there belongs to another registration (or an earlier run), and a failed
    /// `add` must not delete or overwrite a file it did not create.
    fn write_approval_phrases(&self, id: &str, label: &str, phrases: &[String]) -> io::Result<()> {
        let mut body = format!(
            "# {} Approval Patterns\n\n> Written by `many-ai-cli provider add`. One backtick-quoted phrase per bullet; the Hub reads only those.\n\n",
            one_line(label)
        );
        for phrase in phrases {
            body.push_str(&format!("- `{}`\n", phrase.trim()));
        }
        match self
            .approval_dir()?
            .create_new(&format!("{id}.md"), body.as_bytes(), 0o600)
        {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(error(format!(
                "{} already exists (left by an earlier registration, or another one running now); it was not changed. Delete it, or choose another id",
                self.approval_file(id).display()
            ))),
            other => other,
        }
    }
    fn remove_approval_phrases(&self, id: &str) {
        if id.is_empty() || id.contains(['/', '\\']) || id == ".." {
            return;
        }
        if let Ok(dir) = self.approval_dir() {
            let _ = dir.remove_file(&format!("{id}.md"));
        }
    }
    /// The Hub refused a source (or never could read it) because this CLI's data
    /// folder is not the Hub's: say so plainly.
    fn explain_source_refusal(&self, reason: String) -> String {
        if reason.contains("approval_pattern_source") {
            format!(
                "{reason}\n  hint: this command wrote the phrase file under {}, which the running Hub does not read. The CLI and the Hub use different data folders (HOME / USERPROFILE of this shell differs from the Hub's). Run it with the Hub's HOME, or register without --approval-option",
                self.paths.root().display()
            )
        } else {
            reason
        }
    }

    /// The definition to register: the `--definition` file when given, else an
    /// empty one, with every other option written over it.
    fn build_definition(&self, add: &AddArgs) -> io::Result<Definition> {
        let mut definition = match &add.definition_file {
            Some(file) => {
                let path = self.cwd.join(file);
                let meta = std::fs::metadata(&path).map_err(|e| {
                    error(format!("could not read the definition file {file:?}: {e}"))
                })?;
                if !meta.is_file() || meta.len() > 1024 * 1024 {
                    return Err(error(format!(
                        "the definition file {file:?} must be a regular file of at most 1 MiB"
                    )));
                }
                let bytes = std::fs::read(&path).map_err(|e| {
                    error(format!("could not read the definition file {file:?}: {e}"))
                })?;
                crate::proto::decode_wire::<Definition>(&bytes)
                    .map_err(|e| error(format!("the definition file {file:?} is not valid: {e}")))?
            }
            None => Definition::default(),
        };
        if definition.schema_version == 0 {
            definition.schema_version = CURRENT_SCHEMA_VERSION;
        }
        if !add.id.is_empty() {
            definition.id = add.id.clone();
        }
        if !add.label.is_empty() {
            definition.display_name = add.label.clone();
        } else if definition.display_name.is_empty() {
            definition.display_name = definition.id.clone();
        }
        if !add.description.is_empty() {
            definition.description = add.description.clone();
        }
        if !add.command.is_empty() || !add.args.is_empty() || add.headless.is_some() {
            let launch = definition.launch.get_or_insert_with(Default::default);
            if !add.command.is_empty() {
                launch.executable = add.command.clone();
            }
            if !add.args.is_empty() {
                launch.args = add.args.clone();
            }
            if let Some((format, via)) = &add.headless {
                launch.headless = Some(HeadlessDefinition {
                    args: add.headless_args.clone(),
                    format: format.clone(),
                    prompt_via: via.clone(),
                });
            }
        }
        if !add.models.is_empty() {
            let models = definition.models.get_or_insert_with(Default::default);
            models.items = add
                .models
                .iter()
                .map(|id| ModelDefinition {
                    id: id.trim().to_owned(),
                    label: id.trim().to_owned(),
                })
                .collect();
        }
        if !add.letter.is_empty() || !add.color.is_empty() {
            let presentation = definition.presentation.get_or_insert_with(Default::default);
            if !add.letter.is_empty() {
                presentation.icon_text = add.letter.clone();
            }
            if !add.color.is_empty() {
                presentation.color = add.color.clone();
            }
        }
        if !add.usage_url.is_empty() {
            definition.links = Some(LinksDefinition {
                usage_url: add.usage_url.clone(),
            });
        }
        if !add.slash_source.is_empty() {
            let slash = definition
                .slash_commands
                .get_or_insert_with(Default::default);
            slash.source = add.slash_source.clone();
        }
        if !add.permission_modes.is_empty() {
            let permissions = definition.permissions.get_or_insert_with(Default::default);
            for (name, flags) in &add.permission_modes {
                permissions.modes.insert(name.clone(), flags.clone());
                if !permissions.choices.contains(name) {
                    permissions.choices.push(name.clone());
                }
            }
        }
        if !add.approval_options.is_empty() {
            definition.approval_pattern_source = self
                .approval_file(&definition.id)
                .to_string_lossy()
                .into_owned();
        }
        Ok(definition)
    }

    pub async fn add(&self, add: &AddArgs) -> io::Result<String> {
        let definition = self.build_definition(add)?;
        let id = definition.id.clone();
        if id.is_empty() {
            return Err(error(
                "the provider id is missing: give --id, or put \"id\" in the definition file",
            ));
        }
        if is_builtin(&id) || id == "shell" {
            return Err(error(format!(
                "provider id {id:?} is reserved for a built-in; choose another id"
            )));
        }
        // The same id rule the registry applies; it also keeps the id safe to use
        // as a file name for the approval phrases below.
        validate_user_id(&id).map_err(|e| error(format!("provider id {id:?}: {e}")))?;
        if let Some(command) = definition
            .launch
            .as_ref()
            .map(|l| l.executable.clone())
            .filter(|c| !c.is_empty())
            && !self.command_on_path(&command)
        {
            return Err(error(format!(
                "executable {command:?} was not found on PATH; install it, or register a small wrapper script that starts it (a CLI that needs its own HOME or config directory belongs in such a wrapper)"
            )));
        }
        let (_, existing) = self.summaries().await?;
        if let Some(found) = existing.iter().find(|p| p.id == id) {
            return Err(error(format!(
                "provider id {id:?} already exists ({}); remove it first or choose another id",
                origin_label(found.origin.as_str())
            )));
        }
        let label = definition.display_name.clone();
        let has_phrases = !add.approval_options.is_empty();
        let raw = serde_json::to_vec(&definition).map_err(io::Error::other)?;
        // Nothing is written until the definition is accepted.
        if let Some(hub) = self.hub {
            let reply = hub
                .call("POST", "/api/providers/validate", Some(raw.clone()))
                .await?;
            let body = reply.json();
            if reply.status != 200 {
                return Err(error(self.explain_source_refusal(format!(
                    "the Hub refused the definition: {}",
                    reply.reason()
                ))));
            }
            if body["valid"] != true {
                return Err(error(self.explain_source_refusal(format!(
                    "the Hub refused the definition:\n  {}",
                    diagnostics_errors(&body["diagnostics"]).join("\n  ")
                ))));
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
        if has_phrases {
            self.write_approval_phrases(&id, &label, &add.approval_options)
                .map_err(|e| error(format!("could not write the approval phrases: {e}")))?;
        }
        let created = self.create(&definition, raw).await;
        let warnings = match created {
            Ok(warnings) => warnings,
            Err(failure) => {
                if has_phrases {
                    self.remove_approval_phrases(&id);
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
                return Err(error(self.explain_source_refusal(format!(
                    "the Hub did not add it: {}",
                    reply.reason()
                ))));
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
        let mut out = format!(
            "added provider {} ({})
",
            definition.id,
            one_line(&definition.display_name)
        );
        let launch = definition.launch.clone().unwrap_or_default();
        let mut interactive = vec![launch.executable.clone()];
        interactive.extend(launch.args.clone());
        out.push_str(&format!(
            "interactive: {}
",
            shown(&interactive)
        ));
        match &launch.headless {
            Some(headless) => {
                let def = HeadlessDef {
                    args: headless.args.clone(),
                    format: headless.format.clone(),
                    prompt_via: headless.prompt_via.clone(),
                };
                let mut argv = vec![launch.executable.clone()];
                argv.extend(headless::build_argv(&def, &launch.args, "<prompt>"));
                out.push_str(&format!(
                    "headless: yes: {}{} (output format {})
",
                    shown(&argv),
                    if headless.prompt_via == "stdin" { "  (prompt on stdin)" } else { "" },
                    headless.format,
                ));
            }
            None => out.push_str(
                "headless: no (add --headless-arg / --prompt-via to let orchestrate run it as a child)
",
            ),
        }
        if !definition.approval_pattern_source.is_empty() && !add.approval_options.is_empty() {
            out.push_str(&format!(
                "approval phrases: {} written to {}
",
                add.approval_options.len(),
                definition.approval_pattern_source
            ));
        }
        for warning in warnings {
            out.push_str(&format!(
                "warning: {warning}
"
            ));
        }
        out.push_str(
            "what it can do (from the declarations):
",
        );
        out.push_str(&capability_report::render_text(&capability_report::report(
            &declared::resolve(definition, None),
        )));
        if self.hub.is_some() {
            out.push_str(
                "The running Hub reloaded the registry; open dashboards show a notice with an undo.
",
            );
        } else {
            out.push_str(
                "No Hub is running; the Hub will read it on its next start.
",
            );
        }
        out.push_str(&format!(
            "next: many-ai-cli provider test {}
",
            definition.id
        ));
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
        // The phrase file `provider add` made: exactly `<home>/approval-patterns/<id>.md`
        // and only when the definition points at it. Any other source (a URL, a
        // path elsewhere) is not ours to delete.
        let own_phrase_file = self.effective(id).await.ok().is_some_and(|effective| {
            !effective.definition.approval_pattern_source.is_empty()
                && Path::new(&effective.definition.approval_pattern_source)
                    == self.approval_file(id)
        });
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
        if own_phrase_file {
            self.remove_approval_phrases(id);
        }
        Ok(format!(
            "removed provider {id}\nA backup (reason before-delete) is kept: many-ai-cli provider backup list {id}\n"
        ))
    }

    /// The list of what the provider can and cannot do: the declarations, with
    /// the ones that can be tried here (slash source, conversation folder, model
    /// list command) replaced by the result of trying.
    async fn capability_text(&self, effective: &EffectiveDefinition) -> String {
        let resolved =
            declared::resolved_for(&effective.definition.id, Some(&effective.definition))
                .unwrap_or_else(|| effective.definition.clone());
        let lines = provider_checks::merge(
            capability_report::report(&resolved),
            provider_checks::live_checks(&resolved, self.paths, self.environment, self.cwd).await,
        );
        format!(
            "capabilities (available / unavailable / unsupported):
{}",
            capability_report::render_text(&lines)
        )
    }

    pub async fn test(&self, args: &TestArgs) -> io::Result<String> {
        let effective = self.effective(&args.id).await?;
        let capabilities = self.capability_text(&effective).await;
        match self.run_headless(args, &effective).await {
            Ok(report) => Ok(format!("{report}{capabilities}")),
            Err(failure) => Err(error(format!(
                "{failure}
{capabilities}"
            ))),
        }
    }

    async fn run_headless(
        &self,
        args: &TestArgs,
        effective: &EffectiveDefinition,
    ) -> io::Result<String> {
        let launch = prepare_launch_with_definition(
            self.config,
            &args.id,
            WrapperArgs {
                headless: true,
                ..Default::default()
            },
            Some(effective),
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
        // The tested CLI is not a Hub session: it gets no Hub credentials.
        let env =
            hub_session_variables(self.environment, std::env::vars_os().map(|(name, _)| name));
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
        // The child's output is untrusted terminal text: no escape sequence of it
        // reaches this terminal.
        let lines: Vec<String> = crate::approval::identity::strip_ansi(&text)
            .lines()
            .map(|line| printable_line(line).trim_end().to_owned())
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
