//! Official CLI authentication-status boundary. Raw stdout never reaches HTTP,
//! logs, configuration or errors; credentials remain vendor-owned files.
use crate::{
    config::RuntimePaths,
    process::{self, ExitOutcome, ManagedProcess, ProcessPlan, SpawnOptions},
    proto::core::{CoreFuture, TaskCancellation},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Default, PartialEq, Eq, Serialize)]
pub struct Status {
    pub logged_in: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub plan: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub method: String,
}
pub const PROVIDERS: &[&str] = &["claude", "codex", "grok", "opencode"];
pub fn env_key(provider: &str) -> Option<&'static str> {
    match provider {
        "claude" => Some("CLAUDE_CONFIG_DIR"),
        "codex" => Some("CODEX_HOME"),
        "grok" => Some("GROK_HOME"),
        "opencode" => Some("XDG_DATA_HOME"),
        _ => None,
    }
}
pub trait SubscriptionCli: Send + Sync {
    fn status<'a>(
        &'a self,
        provider: &'a str,
        profile: &'a Path,
        timeout: Duration,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, io::Result<Status>>;
}
pub struct NativeSubscriptionCli {
    paths: RuntimePaths,
    cwd: PathBuf,
    pub(super) environment: Vec<String>,
}
impl NativeSubscriptionCli {
    pub fn new(paths: RuntimePaths, cwd: PathBuf, environment: Vec<String>) -> Self {
        Self {
            paths,
            cwd,
            environment,
        }
    }
    pub(super) fn lookup(&self, provider: &str) -> io::Result<String> {
        process::execpath::Resolver::new(
            process::execpath::Platform::native(),
            &self.environment,
            &self.cwd,
            &process::execpath::NativeFs,
        )
        .look_path(provider)
    }
    pub(super) fn command(
        &self,
        provider: &str,
        profile: &Path,
        timeout: Duration,
    ) -> io::Result<ProcessPlan> {
        let key = env_key(provider)
            .ok_or_else(|| io::Error::other("provider does not support subscription profiles"))?;
        crate::profile::subscriptions::check_path(&self.paths, profile)?;
        let resolver = process::execpath::Resolver::new(
            process::execpath::Platform::native(),
            &self.environment,
            &self.cwd,
            &process::execpath::NativeFs,
        );
        let executable = resolver.look_path(provider).map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("provider CLI not found in PATH: {provider}"),
            )
        })?;
        // Trial acceptance can invoke only an explicitly supplied synthetic CLI.
        if self.paths.is_trial()
            && !crate::files::scope::under_roots(
                Path::new(&executable),
                &[self.paths.root().to_owned()],
            )
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trial requires a synthetic profile CLI",
            ));
        }
        let args: Vec<OsString> = match provider {
            "claude" => vec!["auth".into(), "status".into()],
            "codex" => vec!["login".into(), "status".into()],
            "grok" => vec!["models".into()],
            _ => vec!["providers".into(), "list".into()],
        };
        let (executable, args) = vendor_command(&self.environment, executable, args);
        let mut env: BTreeMap<OsString, Option<OsString>> = BTreeMap::new();
        for entry in &self.environment {
            if let Some((name, value)) = entry.split_once('=') {
                if cfg!(windows) {
                    let old = env
                        .keys()
                        .find(|name0| name0.to_string_lossy().eq_ignore_ascii_case(name))
                        .cloned();
                    if let Some(old) = old {
                        env.remove(&old);
                    }
                }
                env.insert(name.into(), Some(value.into()));
            }
        }
        if cfg!(windows) {
            let old = env
                .keys()
                .find(|name0| name0.to_string_lossy().eq_ignore_ascii_case(key))
                .cloned();
            if let Some(old) = old {
                env.remove(&old);
            }
        }
        env.insert(key.into(), Some(profile.as_os_str().to_owned()));
        Ok(ProcessPlan {
            executable: executable.into(),
            args,
            cwd: self.cwd.clone(),
            env,
            stdin: vec![],
            timeout,
            output_cap: 16 * 1024 * 1024,
            pipe_drain_timeout: Duration::from_secs(2),
        })
    }
}
impl SubscriptionCli for NativeSubscriptionCli {
    fn status<'a>(
        &'a self,
        provider: &'a str,
        profile: &'a Path,
        timeout: Duration,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, io::Result<Status>> {
        Box::pin(async move {
            if cancel.token().is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "provider status command cancelled",
                ));
            }
            let plan = self.command(provider, profile, timeout)?;
            let (mut child, _) = ManagedProcess::spawn_owned_with_options(
                plan,
                1,
                SpawnOptions {
                    no_window: true,
                    stderr_null: true,
                    stdin_null: true,
                    env_clear: true,
                    ..Default::default()
                },
            );
            let output=tokio::select!{result=child.wait()=>result,_=cancel.token().cancelled()=>{child.close();child.wait().await}}.map_err(|_|io::Error::other("provider status command failed"))?;
            if output.stdout_truncated {
                return Err(io::Error::other("provider status output exceeds limit"));
            }
            let code = match output.outcome {
                ExitOutcome::Exited {
                    code: Some(code), ..
                } => code,
                ExitOutcome::TimedOut => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "provider status command timed out",
                    ));
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "provider status command cancelled",
                    ));
                }
            };
            parse_status(provider, &String::from_utf8_lossy(&output.stdout), code)
        })
    }
}
pub(super) fn vendor_command(
    environment: &[String],
    path: String,
    args: Vec<OsString>,
) -> (String, Vec<OsString>) {
    if cfg!(windows)
        && (path.to_ascii_lowercase().ends_with(".cmd")
            || path.to_ascii_lowercase().ends_with(".bat"))
    {
        let comspec = environment
            .iter()
            .rev()
            .find_map(|entry| {
                entry
                    .split_once('=')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("COMSPEC"))
                    .map(|(_, value)| value)
            })
            .filter(|value| !value.is_empty())
            .unwrap_or(r"C:\Windows\System32\cmd.exe");
        let mut argv = vec![OsString::from("/c"), path.into()];
        argv.extend(args);
        (comspec.into(), argv)
    } else {
        (path, args)
    }
}
pub fn parse_status(provider: &str, out: &str, code: i32) -> io::Result<Status> {
    let lower = crate::proto::unicode::simple_lower(out);
    match provider {
        "claude" => {
            #[derive(Default, serde::Deserialize)]
            #[serde(default)]
            struct Claude {
                #[serde(rename = "loggedIn")]
                logged_in: bool,
                #[serde(rename = "authMethod")]
                method: String,
                #[serde(rename = "subscriptionType")]
                plan: String,
            }
            impl crate::proto::wire::GoWire for Claude {
                const GO_TYPE: &'static str = "ClaudeAuthStatus";
                const SCHEMAS: &'static [crate::proto::wire::Schema] =
                    &[crate::proto::wire::Schema {
                        name: "ClaudeAuthStatus",
                        fields: &[
                            crate::proto::wire::Field {
                                name: "loggedIn",
                                kind: "bool",
                            },
                            crate::proto::wire::Field {
                                name: "authMethod",
                                kind: "string",
                            },
                            crate::proto::wire::Field {
                                name: "subscriptionType",
                                kind: "string",
                            },
                        ],
                    }];
            }
            let parsed: Claude = crate::proto::wire::decode(out.trim().as_bytes())
                .map_err(|_| io::Error::other("could not read `claude auth status` output"))?;
            if parsed.logged_in {
                Ok(Status {
                    logged_in: true,
                    plan: parsed.plan,
                    method: parsed.method,
                })
            } else {
                Ok(Status::default())
            }
        }
        "codex" => {
            if lower.contains("not logged in") || (code != 0 && !lower.contains("logged in")) {
                return Ok(Status::default());
            }
            let method = if lower.contains("chatgpt") {
                "chatgpt"
            } else if lower.contains("api key") || lower.contains("api-key") {
                "api-key"
            } else {
                ""
            };
            Ok(Status {
                logged_in: true,
                method: method.into(),
                ..Default::default()
            })
        }
        "grok" => {
            if lower.contains("not authenticated")
                || lower.contains("not logged in")
                || (!lower.contains("logged in") && !lower.contains("authenticated"))
            {
                return Ok(Status::default());
            }
            let method = ["grok.com", "x.ai"]
                .into_iter()
                .find(|method| lower.contains(method))
                .unwrap_or("");
            Ok(Status {
                logged_in: true,
                method: method.into(),
                ..Default::default()
            })
        }
        "opencode" => {
            let regex = regex::Regex::new(r"([0-9]+)[\t\n\x0b\x0c\r ]+credentials?")
                .expect("fixed Go status regex");
            let count = regex
                .captures(&lower)
                .and_then(|capture| capture[1].parse::<isize>().ok())
                .unwrap_or(-1);
            if count == 0 || (count < 0 && !lower.contains("credential")) {
                return Ok(Status::default());
            }
            Ok(Status {
                logged_in: true,
                plan: if lower.contains("opencode go") {
                    "go".into()
                } else {
                    String::new()
                },
                ..Default::default()
            })
        }
        _ => Err(io::Error::other(
            "provider does not support subscription profiles",
        )),
    }
}
#[cfg(test)]
mod tests;
