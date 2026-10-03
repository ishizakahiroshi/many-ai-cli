//! Candidate packaging identities. This does not publish, install or execute artifacts.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{self, Read},
    path::{Component, Path},
};

pub const ORACLE: &str = "21d0bc7935a2c4696fb89ccff2e324157a528c2d";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    WindowsX64,
    LinuxX64,
    MacosIntel,
    MacosAppleSilicon,
}
impl Target {
    pub const ALL: [Self; 4] = [
        Self::WindowsX64,
        Self::LinuxX64,
        Self::MacosIntel,
        Self::MacosAppleSilicon,
    ];
    pub fn suffix(self) -> &'static str {
        match self {
            Self::WindowsX64 => "windows-x64",
            Self::LinuxX64 => "linux-x64",
            Self::MacosIntel => "macos-intel",
            Self::MacosAppleSilicon => "macos-apple-silicon",
        }
    }
    pub fn triple(self) -> &'static str {
        match self {
            Self::WindowsX64 => "x86_64-pc-windows-msvc",
            Self::LinuxX64 => "x86_64-unknown-linux-gnu",
            Self::MacosIntel => "x86_64-apple-darwin",
            Self::MacosAppleSilicon => "aarch64-apple-darwin",
        }
    }
    pub fn goos(self) -> &'static str {
        match self {
            Self::WindowsX64 => "windows",
            Self::LinuxX64 => "linux",
            Self::MacosIntel | Self::MacosAppleSilicon => "darwin",
        }
    }
    pub fn goarch(self) -> &'static str {
        if self == Self::MacosAppleSilicon {
            "arm64"
        } else {
            "amd64"
        }
    }
    pub fn npm_package(self) -> String {
        format!("many-ai-cli-{}", self.suffix())
    }
    pub fn binary(self, launcher: bool) -> String {
        format!(
            "many-ai-cli{}{}",
            if launcher { "-launcher" } else { "" },
            if self == Self::WindowsX64 { ".exe" } else { "" }
        )
    }
    pub fn archive(self, version: &str) -> String {
        format!("many-ai-cli-{version}-{}.zip", self.suffix())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Zip,
    Npm,
    Deb,
    Rpm,
    Homebrew,
    Winget,
}
pub fn package_files(target: Target, channel: Channel) -> Result<Vec<String>, String> {
    let mut files = match channel {
        Channel::Npm => {
            return Ok(vec![
                "package.json".into(),
                format!("bin/{}", target.binary(false)),
            ]);
        }
        Channel::Deb | Channel::Rpm if target != Target::LinuxX64 => {
            return Err("deb/rpm is supported only for Linux x64".into());
        }
        Channel::Homebrew if !matches!(target, Target::MacosIntel | Target::MacosAppleSilicon) => {
            return Err("Homebrew cask is supported only for macOS".into());
        }
        Channel::Winget if target != Target::WindowsX64 => {
            return Err("winget portable package is supported only for Windows x64".into());
        }
        Channel::Deb | Channel::Rpm => {
            return Ok(vec![
                format!("usr/bin/{}", target.binary(false)),
                format!("usr/bin/{}", target.binary(true)),
            ]);
        }
        _ => vec![target.binary(false), target.binary(true)],
    };
    files.extend(
        [
            "README.md",
            "README.ja.md",
            "CHANGELOG.md",
            "LICENSE",
            "THIRD_PARTY_NOTICES.md",
            "web/src/vendor/THIRD_PARTY_LICENSES.txt",
        ]
        .map(String::from),
    );
    if target == Target::WindowsX64 {
        files.push("unblock-windows.cmd".into());
    }
    Ok(files)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BinaryArtifact {
    pub target: Target,
    pub launcher: bool,
    #[serde(flatten)]
    pub file: ArtifactFile,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeArtifact {
    pub target: Target,
    pub identity: String,
    pub license: String,
    pub verification: String,
    #[serde(flatten)]
    pub file: ArtifactFile,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryManifest {
    pub schema: u32,
    pub oracle: String,
    pub source_revision: String,
    pub lockfile_sha256: String,
    pub version: String,
    pub toolchain: String,
    pub build_time: String,
    pub binaries: Vec<BinaryArtifact>,
    pub embedded_assets: Vec<ArtifactFile>,
    pub runtimes: Vec<RuntimeArtifact>,
}
fn hex(value: &str, n: usize) -> bool {
    value.len() == n && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
impl DeliveryManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.oracle != ORACLE {
            return Err("unsupported delivery schema or Go oracle".into());
        }
        if !hex(&self.source_revision, 40) || !hex(&self.lockfile_sha256, 64) {
            return Err("source revision and lockfile digest are required".into());
        }
        if self.version.trim().is_empty()
            || !self.version.contains("candidate")
            || self.toolchain.trim().is_empty()
            || self.build_time.trim().is_empty()
        {
            return Err("explicit candidate version/toolchain/build time are required".into());
        }
        if self.binaries.len() != 8 {
            return Err("exactly both binaries for all four supported targets are required".into());
        }
        let mut pairs = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for artifact in &self.binaries {
            if !pairs.insert((artifact.target.suffix(), artifact.launcher)) {
                return Err("duplicate target/binary".into());
            }
            if Path::new(&artifact.file.path)
                .file_name()
                .and_then(|p| p.to_str())
                != Some(artifact.target.binary(artifact.launcher).as_str())
            {
                return Err("binary basename does not match its target and role".into());
            }
        }
        for artifact in self
            .binaries
            .iter()
            .map(|a| &a.file)
            .chain(self.embedded_assets.iter())
            .chain(self.runtimes.iter().map(|a| &a.file))
        {
            if !relative(&artifact.path)
                || !hex(&artifact.sha256, 64)
                || artifact.bytes == 0
                || !paths.insert(&artifact.path)
            {
                return Err("artifact path/hash/size is missing, unsafe or duplicated".into());
            }
        }
        for needed in ["web/dist/index.html", "internal/launcher/ui/index.html"] {
            if !self.embedded_assets.iter().any(|a| a.path == needed) {
                return Err(format!("missing embedded asset identity: {needed}"));
            }
        }
        for runtime in &self.runtimes {
            if runtime.identity.is_empty()
                || runtime.license.is_empty()
                || runtime.verification.is_empty()
            {
                return Err("runtime provenance/license/verification evidence is required".into());
            }
        }
        for name in [
            "vcomp140.dll",
            "msvcp140.dll",
            "vcruntime140.dll",
            "vcruntime140_1.dll",
        ] {
            if !self.runtimes.iter().any(|r| {
                r.target == Target::WindowsX64
                    && Path::new(&r.file.path).file_name().and_then(|p| p.to_str()) == Some(name)
            }) {
                return Err(format!("missing verified Windows runtime input: {name}"));
            }
        }
        Ok(())
    }
    /// Readback uses non-symlink held-directory opens, not unchecked manifest paths.
    pub fn verify_files(&self, root: &Path) -> io::Result<()> {
        self.validate().map_err(io::Error::other)?;
        for artifact in self
            .binaries
            .iter()
            .map(|a| &a.file)
            .chain(self.embedded_assets.iter())
            .chain(self.runtimes.iter().map(|a| &a.file))
        {
            let file = Path::new(&artifact.path);
            let mut dir = crate::files::safe_fs::Dir::open(root)?;
            for part in file.parent().unwrap_or_else(|| Path::new("")).components() {
                let Component::Normal(name) = part else {
                    return Err(io::Error::other("unsafe artifact parent"));
                };
                dir = dir.child_dir(
                    name.to_str()
                        .ok_or_else(|| io::Error::other("invalid artifact path"))?,
                    false,
                )?;
            }
            let mut handle = dir.open_file(
                file.file_name()
                    .and_then(|p| p.to_str())
                    .ok_or_else(|| io::Error::other("invalid artifact path"))?,
                false,
            )?;
            if handle.metadata()?.len() != artifact.bytes {
                return Err(io::Error::other("artifact size mismatch"));
            }
            let mut hash = Sha256::new();
            let mut bytes = [0u8; 64 * 1024];
            loop {
                let count = handle.read(&mut bytes)?;
                if count == 0 {
                    break;
                }
                hash.update(&bytes[..count]);
            }
            let actual = hash
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if actual != artifact.sha256.to_ascii_lowercase() {
                return Err(io::Error::other("artifact SHA256 mismatch"));
            }
        }
        Ok(())
    }
    /// Adapter consumed by existing scripts/stage-npm-binaries.mjs. Paths stay
    /// relative to the repository as that script requires. Launcher records are
    /// present but its ID filter intentionally stages the main binary only.
    pub fn goreleaser_artifacts(
        &self,
        repository_relative_candidate_dir: &str,
    ) -> Result<serde_json::Value, String> {
        self.validate()?;
        if !relative(repository_relative_candidate_dir) {
            return Err("candidate directory must be repository-relative".into());
        }
        Ok(serde_json::Value::Array(self.binaries.iter().map(|artifact|serde_json::json!({
            "type":"Binary","name":artifact.target.binary(artifact.launcher),"path":format!("{repository_relative_candidate_dir}/{}",artifact.file.path),
            "goos":artifact.target.goos(),"goarch":artifact.target.goarch(),"extra":{"ID":if artifact.launcher{"many-ai-cli-launcher"}else{"many-ai-cli"}},
        })).collect()))
    }
}
