use super::{
    ConnectorConfig, Profile, ProfilesFile, profile::SCHEMAS, ssh_import_args, unique_profile_name,
};
use crate::{
    process::Cancellation,
    proto::wire::{GoWire, Schema},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportedProfile {
    pub profile: Profile,
    pub host_candidates: Option<Vec<String>>,
}
impl GoWire for ExportedProfile {
    const GO_TYPE: &'static str = "LauncherExportedProfile";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FetchParams {
    pub host: String,
    pub user: String,
    pub ssh_port: i64,
    pub identity_file: String,
    pub binary: String,
}
impl GoWire for FetchParams {
    const GO_TYPE: &'static str = "LauncherFetchParams";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl FetchParams {
    pub fn normalized_profile(&self) -> Result<Profile, String> {
        let binary = self.binary.trim();
        let mut p = Profile {
            name: "_fetch".into(),
            kind: "ssh".into(),
            mode: "serve".into(),
            host: self.host.clone(),
            user: self.user.clone(),
            ssh_port: self.ssh_port,
            identity_file: self.identity_file.clone(),
            binary: if binary.is_empty() {
                "many-ai-cli".into()
            } else {
                binary.into()
            },
            ..Profile::default()
        };
        p.validate()?;
        p.normalize();
        Ok(p)
    }
    pub fn trim(&mut self) {
        self.host = self.host.trim().into();
        self.user = self.user.trim().into();
        self.identity_file = self.identity_file.trim().into();
        self.binary = self.binary.trim().into();
    }
}
pub fn parse_exported_profile(out: &[u8]) -> Result<ExportedProfile, String> {
    if let Ok(profile) = crate::proto::decode_wire(out) {
        return Ok(profile);
    }
    let Some(start) = out.iter().position(|b| *b == b'{') else {
        return Err("profile-export returned no JSON output".into());
    };
    let Some(end) = out
        .iter()
        .rposition(|b| *b == b'}')
        .filter(|end| *end >= start)
    else {
        return Err("profile-export returned no JSON output".into());
    };
    crate::proto::decode_wire(&out[start..=end])
        .map_err(|_| "parse profile-export output: invalid JSON or field type".into())
}
impl ConnectorConfig {
    pub async fn fetch_remote_profile(
        &self,
        params: &FetchParams,
        cancel: &Cancellation,
    ) -> Result<ExportedProfile, String> {
        super::validate_remote_scope(&self.paths, self.remote_trial.as_ref())?;
        let profile = params.normalized_profile()?;
        let plan = self.plan(
            &self.ssh_executable,
            ssh_import_args(&profile, self.remote_trial.as_ref(), self.paths.port()),
            Duration::from_secs(25),
        );
        let output = self.run_short(plan, cancel).await?;
        // Never echo raw SSH stderr (it may contain a token or credential).
        super::connector::exit_success(&output, "ssh profile-export")?;
        if output.stdout_truncated {
            return Err("profile-export output exceeds limit".into());
        }
        parse_exported_profile(&output.stdout)
    }
}
pub fn complete_fetched_profile(
    mut exported: ExportedProfile,
    params: &FetchParams,
    existing: &ProfilesFile,
) -> Result<ExportedProfile, String> {
    let p = &mut exported.profile;
    p.name = unique_profile_name(existing.list(), &p.name);
    p.host = params.host.clone();
    if !params.user.is_empty() {
        p.user = params.user.clone();
    }
    if params.ssh_port > 0 {
        p.ssh_port = params.ssh_port;
    }
    p.identity_file = params.identity_file.clone();
    p.validate()?;
    Ok(exported)
}
#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    pub name: String,
    pub public_host: String,
    pub cwd: String,
    pub hub_port: i64,
}
#[derive(Clone, Debug, Default)]
pub struct ExportIdentity {
    pub username: String,
    pub user: String,
    pub public_host: String,
    pub ssh_connection: String,
    pub hostname: String,
    pub ipv4: Vec<std::net::Ipv4Addr>,
    pub cwd: String,
    pub executable: String,
}
impl ExportIdentity {
    /// Read only the defined environment fields; never enumerate the process environment.
    pub fn local() -> Self {
        Self {
            username: std::env::var("USERNAME").unwrap_or_default(),
            user: std::env::var("USER").unwrap_or_default(),
            public_host: std::env::var("MANY_AI_CLI_PUBLIC_HOST").unwrap_or_default(),
            ssh_connection: std::env::var("SSH_CONNECTION").unwrap_or_default(),
            hostname: super::platform::hostname(),
            ipv4: super::platform::local_ipv4(),
            cwd: std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            executable: std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }
}
pub fn build_export_profile(opts: &ExportOptions, identity: &ExportIdentity) -> ExportedProfile {
    let mut candidates = Vec::<String>::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut add = |host: &str| {
        let host = host.trim().strip_suffix('.').unwrap_or(host.trim());
        if !host.is_empty() && seen.insert(host.to_lowercase()) {
            candidates.push(host.to_owned());
        }
    };
    add(&opts.public_host);
    add(&identity.public_host);
    add(identity
        .ssh_connection
        .split_whitespace()
        .nth(2)
        .unwrap_or_default());
    for ip in &identity.ipv4 {
        if !ip.is_loopback() && !ip.is_link_local() {
            add(&ip.to_string());
        }
    }
    add(&identity.hostname);
    let host = candidates.first().cloned().unwrap_or_default();
    let name = if !opts.name.trim().is_empty() {
        opts.name.trim()
    } else if !identity.hostname.trim().is_empty() {
        identity.hostname.trim()
    } else if !host.is_empty() {
        &host
    } else {
        "remote"
    };
    let binary = identity
        .executable
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim();
    ExportedProfile {
        profile: Profile {
            name: name.into(),
            kind: "ssh".into(),
            mode: "serve".into(),
            host,
            user: if identity.username.is_empty() {
                identity.user.clone()
            } else {
                identity.username.clone()
            },
            binary: if binary.is_empty() {
                "many-ai-cli".into()
            } else {
                binary.into()
            },
            cwd: if opts.cwd.trim().is_empty() {
                identity.cwd.clone()
            } else {
                opts.cwd.trim().into()
            },
            hub_port: opts.hub_port,
            ..Profile::default()
        },
        host_candidates: if candidates.is_empty() {
            None
        } else {
            Some(candidates)
        },
    }
}
