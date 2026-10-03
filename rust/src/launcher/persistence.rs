//! One held directory capability owns every launcher file, including locks and temps.
use super::{Profile, ProfilesFile};
use crate::{
    config::{Resource, RuntimePaths},
    files::safe_fs::Dir,
};
use std::{fs::File, io, path::Path, sync::Arc};

#[derive(Clone)]
pub struct LauncherStore {
    pub(crate) dir: Arc<Dir>,
    pub paths: RuntimePaths,
}
impl LauncherStore {
    pub fn open(paths: RuntimePaths) -> io::Result<Self> {
        let dir = Dir::open_or_create_private(paths.root())?;
        Ok(Self {
            dir: Arc::new(dir),
            paths,
        })
    }
    pub(crate) fn name(&self, resource: Resource) -> io::Result<String> {
        let path = self.paths.resource(resource);
        if path.parent() != Some(self.paths.root()) {
            return Err(io::Error::other("launcher resource escapes its root"));
        }
        path.file_name()
            .and_then(|s| s.to_str())
            .map(String::from)
            .ok_or_else(|| io::Error::other("invalid launcher resource name"))
    }
    pub(crate) fn lock(&self, name: &str) -> io::Result<File> {
        let file = self.dir.open_lock(name)?;
        file.lock()?;
        Ok(file)
    }
    pub fn load_profiles(&self) -> io::Result<ProfilesFile> {
        let name = self.name(Resource::LauncherProfiles)?;
        let bytes = match self.dir.read(&name, 8 * 1024 * 1024 + 1) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ProfilesFile::fresh());
            }
            Err(error) => return Err(io::Error::other(format!("read launcher profiles: {error}"))),
        };
        decode_profiles_yaml(&bytes)
    }
    /// Save is a serialization operation, like Go SaveProfiles. Validate newly
    /// submitted values in replace_profiles, never reject tolerated old data here.
    pub fn save_profiles(&self, profiles: &ProfilesFile) -> io::Result<()> {
        let _lock = self.lock("launcher-profiles.yaml.lock")?;
        self.save_profiles_unlocked(profiles)
    }
    fn save_profiles_unlocked(&self, profiles: &ProfilesFile) -> io::Result<()> {
        let yaml = serde_saphyr::to_string(profiles)
            .map_err(|_| io::Error::other("marshal launcher profiles"))?;
        self.dir.replace(
            &self.name(Resource::LauncherProfiles)?,
            yaml.as_bytes(),
            0o600,
        )
    }
    pub fn update_profiles<T>(
        &self,
        mutate: impl FnOnce(&mut ProfilesFile) -> io::Result<T>,
    ) -> io::Result<T> {
        let _lock = self.lock("launcher-profiles.yaml.lock")?;
        let mut profiles = self.load_profiles()?;
        let result = mutate(&mut profiles)?;
        self.save_profiles_unlocked(&profiles)?;
        Ok(result)
    }
    pub fn replace_profiles(&self, profiles: Option<Vec<Profile>>) -> io::Result<()> {
        self.update_profiles(|file| {
            file.profiles = profiles;
            file.validate()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
        })
    }
    pub fn set_last_used(&self, name: &str) -> io::Result<()> {
        self.update_profiles(|file| {
            file.last_used = name.into();
            Ok(())
        })
    }
    pub fn root(&self) -> &Path {
        self.paths.root()
    }
}
pub fn decode_profiles_yaml(bytes: &[u8]) -> io::Result<ProfilesFile> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| io::Error::other("parse launcher profiles: invalid UTF-8"))?;
    if text.trim().is_empty() {
        return Ok(ProfilesFile::fresh());
    }
    // Shared schema-directed Go YAML decoder is supplied by the integration owner.
    let value = crate::config::decode_yaml_schema(
        text,
        "LauncherProfilesFile",
        super::profile::YAML_SCHEMAS,
    )
    .map_err(|error| io::Error::other(format!("parse launcher profiles: {error}")))?;
    let mut profiles: ProfilesFile = if value.is_null() {
        ProfilesFile::default()
    } else {
        crate::proto::decode_wire(&serde_json::to_vec(&value).map_err(io::Error::other)?)
            .map_err(|_| io::Error::other("parse launcher profiles: field type mismatch"))?
    };
    if profiles.version > 1 {
        return Err(io::Error::other(format!(
            "unsupported launcher-profiles.yaml version {} (max supported: 1)",
            profiles.version
        )));
    }
    profiles.normalize();
    Ok(profiles)
}
