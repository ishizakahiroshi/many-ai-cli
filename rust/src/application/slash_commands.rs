//! Source-compatible slash command discovery, settings and 24-hour cache.
mod markdown;
mod skills;
mod sources;
#[cfg(test)]
mod tests;
use crate::{
    application::event_observer::EventWarning,
    config::{ConfigError, ConfigStore, RuntimePaths, SlashCmdSources},
    hub::task_owner::HubTaskHandle,
    proto::{
        core::*,
        time::{Timestamp, format_go_rfc3339_layout},
        unicode::simple_lower,
    },
    terminal::session::SessionEngine,
};
use serde::{Deserialize, Serialize};
pub use sources::{NativeSlashIo, validate_source};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
const BUILTINS: &[&str] = crate::proto::provider::BUILTIN_PROVIDER_IDS;
const _: () = assert!(BUILTINS.len() == 7, "PROVIDERS lists every built-in");
/// The providers whose sources are user-editable settings, in the order of
/// [`source_fields`]: the built-ins and Devin CLI (Settings -> Slash command
/// sources has a field for it). Any other registered provider reads the source
/// or the list its definition declares.
pub const PROVIDERS: &[&str] = &[
    BUILTINS[0],
    BUILTINS[1],
    BUILTINS[2],
    BUILTINS[3],
    BUILTINS[4],
    BUILTINS[5],
    BUILTINS[6],
    "devin-cli",
];
#[derive(Clone, Default, Debug, PartialEq, Serialize)]
pub struct SlashCmd {
    pub cmd: String,
    pub desc: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip)]
    pub path: PathBuf,
}
#[derive(Clone, Default)]
pub struct SearchContext {
    pub home_dir: String,
    pub codex_home: String,
    pub claude_dir: String,
    pub cwd: String,
}
#[derive(Clone, Serialize)]
pub struct SlashResponse {
    pub cmds: Option<Vec<SlashCmd>>,
    pub fetched_at: String,
    pub source_url: String,
}
#[derive(Clone)]
struct Entry {
    cmds: Option<Vec<SlashCmd>>,
    fetched: Timestamp,
    source: String,
}
impl Entry {
    fn response(&self) -> SlashResponse {
        SlashResponse {
            cmds: self.cmds.clone(),
            fetched_at: format_go_rfc3339_layout(self.fetched, 0, false).unwrap_or_default(),
            source_url: self.source.clone(),
        }
    }
}
pub trait SlashIo: Send + Sync {
    fn read<'a>(&'a self, source: &'a str) -> CoreFuture<'a, io::Result<Vec<u8>>>;
    fn skills(&self, provider: &str, context: &SearchContext) -> Vec<SlashCmd>;
}
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct SourcePatch {
    pub claude: Option<String>,
    pub codex: Option<String>,
    pub copilot: Option<String>,
    #[serde(rename = "cursor-agent")]
    pub cursor_agent: Option<String>,
    pub opencode: Option<String>,
    pub grok: Option<String>,
    #[serde(rename = "command-code")]
    pub command_code: Option<String>,
    #[serde(rename = "devin-cli")]
    pub devin_cli: Option<String>,
}
impl crate::proto::wire::GoWire for SourcePatch {
    const GO_TYPE: &'static str = "slashCmdSourcesPatch";
    const SCHEMAS: &'static [crate::proto::wire::Schema] = &[crate::proto::wire::Schema {
        name: "slashCmdSourcesPatch",
        fields: &[
            crate::proto::wire::Field {
                name: "claude",
                kind: "*string",
            },
            crate::proto::wire::Field {
                name: "codex",
                kind: "*string",
            },
            crate::proto::wire::Field {
                name: "copilot",
                kind: "*string",
            },
            crate::proto::wire::Field {
                name: "cursor-agent",
                kind: "*string",
            },
            crate::proto::wire::Field {
                name: "opencode",
                kind: "*string",
            },
            crate::proto::wire::Field {
                name: "grok",
                kind: "*string",
            },
            crate::proto::wire::Field {
                name: "command-code",
                kind: "*string",
            },
            crate::proto::wire::Field {
                name: "devin-cli",
                kind: "*string",
            },
        ],
    }];
}
#[derive(Debug)]
pub enum SlashFailure {
    InvalidProvider,
    NotConfigured,
    Fetch(String),
    Unavailable,
}
pub struct SlashCommands {
    config: Arc<ConfigStore>,
    paths: RuntimePaths,
    core: Weak<SessionEngine>,
    tasks: HubTaskHandle,
    io: Arc<dyn SlashIo>,
    warning: EventWarning,
    cache: Mutex<BTreeMap<String, Entry>>,
}
impl SlashCommands {
    pub fn new(
        config: Arc<ConfigStore>,
        paths: RuntimePaths,
        core: Weak<SessionEngine>,
        tasks: HubTaskHandle,
        environment: Vec<String>,
        home: Option<PathBuf>,
        warning: EventWarning,
    ) -> io::Result<Arc<Self>> {
        let native = NativeSlashIo::new(paths.clone(), environment, home)?;
        Ok(Self::with_io(
            config,
            paths,
            core,
            tasks,
            Arc::new(native),
            warning,
        ))
    }
    pub fn with_io(
        config: Arc<ConfigStore>,
        paths: RuntimePaths,
        core: Weak<SessionEngine>,
        tasks: HubTaskHandle,
        io: Arc<dyn SlashIo>,
        warning: EventWarning,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            paths,
            core,
            tasks,
            io,
            warning,
            cache: Mutex::default(),
        })
    }
    pub fn sources(&self) -> Result<SlashCmdSources, ConfigError> {
        Ok(self.config.snapshot()?.config.slash_cmd_sources.effective())
    }
    pub fn patch_sources(&self, patch: &SourcePatch) -> Result<(), SourceFailure> {
        for _ in 0..8 {
            let snapshot = self.config.snapshot().map_err(|_| SourceFailure::Save)?;
            let previous = snapshot.config.slash_cmd_sources.clone();
            let mut next = snapshot.config;
            let fields = source_fields_mut(&mut next.slash_cmd_sources);
            let patches = [
                &patch.claude,
                &patch.codex,
                &patch.copilot,
                &patch.cursor_agent,
                &patch.opencode,
                &patch.grok,
                &patch.command_code,
                &patch.devin_cli,
            ];
            for (field, value) in fields.into_iter().zip(patches) {
                if let Some(value) = value {
                    *field = value.trim().into();
                }
            }
            for (provider, value) in PROVIDERS.iter().zip(source_fields(&next.slash_cmd_sources)) {
                validate_source(value, &self.paths).map_err(|error| {
                    SourceFailure::Invalid(format!("invalid {provider} source: {error}"))
                })?;
            }
            let changed = PROVIDERS
                .iter()
                .zip(
                    source_fields(&previous)
                        .into_iter()
                        .zip(source_fields(&next.slash_cmd_sources)),
                )
                .filter_map(|(provider, (a, b))| (a != b).then_some(*provider))
                .collect::<Vec<_>>();
            match self
                .config
                .publish_then_persist_legacy_with(snapshot.revision, next, |_| {
                    let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                    for provider in &changed {
                        cache.retain(|key, _| {
                            key != provider && !key.starts_with(&format!("{provider}|"))
                        });
                    }
                }) {
                Ok(_) => return Ok(()),
                Err(ConfigError::Conflict { .. }) => continue,
                Err(_) => return Err(SourceFailure::Save),
            }
        }
        Err(SourceFailure::Save)
    }
    pub fn search_context(&self, provider: &str, session: &str) -> SearchContext {
        let Some(id) = session.trim().parse::<i64>().ok().filter(|id| *id > 0) else {
            return SearchContext::default();
        };
        let Some(details) = self
            .core
            .upgrade()
            .and_then(|core| core.details(LiveSessionId(id)))
            .filter(|details| details.snapshot.provider == provider)
        else {
            return SearchContext::default();
        };
        SearchContext {
            home_dir: details.transcript.home_dir,
            codex_home: details.transcript.codex_home,
            claude_dir: details.transcript.claude_dir,
            cwd: details.transcript.cwd,
        }
    }
    pub async fn commands(
        self: &Arc<Self>,
        provider: &str,
        context: SearchContext,
        force: bool,
        now: Timestamp,
    ) -> Result<SlashResponse, SlashFailure> {
        // A built-in (and Devin CLI) reads its user editable source from the
        // settings. Any other registered AI CLI is valid when the registry knows it, and reads the
        // source or the list its definition declares (`slash_commands`).
        let declared = if PROVIDERS.contains(&provider) {
            None
        } else {
            crate::profile::declared::declared(provider)
        };
        self.commands_declared(provider, declared, context, force, now)
            .await
    }
    /// [`Self::commands`] with the definition of a provider outside
    /// [`PROVIDERS`] passed in (`None` for one of them).
    pub async fn commands_declared(
        self: &Arc<Self>,
        provider: &str,
        declared: Option<crate::proto::provider::Definition>,
        context: SearchContext,
        force: bool,
        now: Timestamp,
    ) -> Result<SlashResponse, SlashFailure> {
        if declared.is_none() && !PROVIDERS.contains(&provider) {
            return Err(SlashFailure::InvalidProvider);
        }
        if declared
            .as_ref()
            .is_some_and(|def| !def.enabled.unwrap_or(true))
        {
            return Err(SlashFailure::InvalidProvider);
        }
        let direct = declared
            .as_ref()
            .and_then(|def| def.slash_commands.as_ref())
            .is_some_and(|slash| !slash.items.is_empty());
        let mut key = cache_key(provider, &context);
        if let Some(slash) = declared.as_ref().and_then(|d| d.slash_commands.as_ref()) {
            // A changed definition must not be answered from the old entry.
            key = format!("{key}|{}", slash.source);
        }
        let entry = if direct {
            None
        } else {
            self.cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&key)
                .cloned()
        };
        if !force
            && let Some(entry) = &entry
            && now
                .duration_since(entry.fetched)
                .is_ok_and(|elapsed| elapsed < Duration::from_secs(24 * 3600))
        {
            return Ok(entry.response());
        }
        // Go time.Since accepts a future fetchedAt as younger than the TTL.
        if !force
            && let Some(entry) = &entry
            && now < entry.fetched
        {
            return Ok(entry.response());
        }
        let source = match &declared {
            None => {
                let source = self.sources().map_err(|_| SlashFailure::Unavailable)?;
                source_fields(&source)[PROVIDERS.iter().position(|p| *p == provider).unwrap()]
                    .to_owned()
            }
            Some(def) => {
                let slash = def.slash_commands.clone().unwrap_or_default();
                if !slash.items.is_empty() {
                    // A list written into the definition needs no fetch.
                    let skills = self.io.skills(provider, &context);
                    let cmds = merge(
                        provider,
                        slash
                            .items
                            .iter()
                            .map(|item| SlashCmd {
                                cmd: item.cmd.clone(),
                                desc: item.desc.clone(),
                                ..Default::default()
                            })
                            .collect(),
                        skills,
                    );
                    let entry = Entry {
                        cmds: if cmds.is_empty() { None } else { Some(cmds) },
                        fetched: now,
                        source: String::new(),
                    };
                    let response = entry.response();
                    self.cache
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(key, entry);
                    return Ok(response);
                }
                slash.source
            }
        };
        if source.is_empty() {
            return Err(SlashFailure::NotConfigured);
        }
        let permit = self
            .tasks
            .effect_permit()
            .map_err(|_| SlashFailure::Unavailable)?;
        let owner = self.clone();
        let provider = provider.to_owned();
        let fetch_started = tokio::time::Instant::now();
        permit
            .start(async move {
                let skills = owner.io.skills(&provider, &context);
                let body = owner.io.read(&source).await;
                let cmds = match body {
                    Ok(body) => merge(
                        &provider,
                        markdown::parse(&crate::proto::wire::go_utf8_lossy(&body)),
                        skills,
                    ),
                    Err(error) => {
                        (owner.warning)(
                            "slash cmd fetch failed",
                            &SessionError::Transport(crate::storage::mask_secrets(
                                &error.to_string(),
                            )),
                        );
                        if let Some(mut entry) = entry {
                            if !skills.is_empty() {
                                entry.cmds =
                                    Some(merge(&provider, entry.cmds.unwrap_or_default(), skills));
                            }
                            return Ok(entry.response());
                        }
                        if skills.is_empty() {
                            return Err(SlashFailure::Fetch(crate::storage::mask_secrets(
                                &error.to_string(),
                            )));
                        }
                        dedupe(skills)
                    }
                };
                // Go nil markdown parse is JSON null; successful merge/dedupe is [] only when explicitly allocated.
                let cmds = if cmds.is_empty() { None } else { Some(cmds) };
                let entry = Entry {
                    cmds,
                    fetched: now.checked_add(fetch_started.elapsed()).unwrap_or(now),
                    source,
                };
                let response = entry.response();
                owner
                    .cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key, entry);
                Ok(response)
            })
            .wait()
            .await
            .map_err(|_| SlashFailure::Unavailable)?
    }
}
#[derive(Debug)]
pub enum SourceFailure {
    Invalid(String),
    Save,
}
fn source_fields(source: &SlashCmdSources) -> [&str; 8] {
    [
        &source.claude,
        &source.codex,
        &source.copilot,
        &source.cursor_agent,
        &source.opencode,
        &source.grok,
        &source.command_code,
        &source.devin_cli,
    ]
}
fn source_fields_mut(source: &mut SlashCmdSources) -> [&mut String; 8] {
    [
        &mut source.claude,
        &mut source.codex,
        &mut source.copilot,
        &mut source.cursor_agent,
        &mut source.opencode,
        &mut source.grok,
        &mut source.command_code,
        &mut source.devin_cli,
    ]
}
fn cache_key(provider: &str, ctx: &SearchContext) -> String {
    format!(
        "{provider}|{}|{}|{}|{}",
        simple_lower(ctx.home_dir.trim()),
        simple_lower(ctx.codex_home.trim()),
        simple_lower(ctx.claude_dir.trim()),
        simple_lower(ctx.cwd.trim())
    )
}
fn merge(provider: &str, cmds: Vec<SlashCmd>, skills: Vec<SlashCmd>) -> Vec<SlashCmd> {
    if skills.is_empty() {
        return cmds;
    }
    if provider == "claude" {
        dedupe(skills.into_iter().chain(cmds).collect())
    } else {
        dedupe(cmds.into_iter().chain(skills).collect())
    }
}
fn dedupe(cmds: Vec<SlashCmd>) -> Vec<SlashCmd> {
    let mut seen = BTreeSet::new();
    cmds.into_iter()
        .filter(|cmd| !cmd.cmd.trim().is_empty() && seen.insert(cmd.cmd.trim().to_owned()))
        .collect()
}
