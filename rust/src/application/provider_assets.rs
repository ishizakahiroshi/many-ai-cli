//! Private icon storage shared by uploads and provider mutation cleanup.
use crate::{
    config::{Resource, RuntimePaths},
    files::safe_fs::Dir,
    profile::store::ProviderRegistryStore,
};
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};
pub const ICON_LIMIT: usize = 512 * 1024;
pub(crate) static ICON_GATE: Mutex<()> = Mutex::new(());
pub type IconWarning = Arc<dyn Fn(&str, &io::Error) + Send + Sync>;
pub struct ProviderAssets {
    path: PathBuf,
    held: Mutex<Option<Arc<Dir>>>,
    pub registry: Arc<ProviderRegistryStore>,
    warning: IconWarning,
}
impl ProviderAssets {
    pub fn new(
        paths: &RuntimePaths,
        registry: Arc<ProviderRegistryStore>,
        warning: IconWarning,
    ) -> Arc<Self> {
        Arc::new(Self {
            path: paths.resource(Resource::ProviderIcons),
            held: Mutex::new(None),
            registry,
            warning,
        })
    }
    fn dir(&self, create: bool) -> io::Result<Arc<Dir>> {
        let mut held = self
            .held
            .lock()
            .map_err(|_| io::Error::other("icon directory lock poisoned"))?;
        if let Some(dir) = &*held {
            return Ok(dir.clone());
        }
        let dir = Arc::new(if create {
            Dir::open_or_create_private_components(&self.path)?
        } else {
            Dir::open(&self.path)?
        });
        *held = Some(dir.clone());
        Ok(dir)
    }
    pub fn read(&self, id: &str) -> io::Result<Vec<u8>> {
        self.dir(false)?.read(&format!("{id}.bin"), usize::MAX)
    }
    pub fn write(&self, id: &str, bytes: &[u8]) -> io::Result<()> {
        if !valid_icon_id(id) {
            return Err(io::Error::other("invalid icon id"));
        }
        let result = (|| {
            let _g = ICON_GATE
                .lock()
                .map_err(|_| io::Error::other("icon update lock poisoned"))?;
            self.dir(true)?.replace(&format!("{id}.bin"), bytes, 0o600)
        })();
        if let Err(e) = &result {
            (self.warning)(id, e)
        }
        result
    }
    pub fn remove(&self, id: &str) {
        if !valid_icon_id(id) {
            return;
        }
        let result = (|| {
            let _g = ICON_GATE
                .lock()
                .map_err(|_| io::Error::other("icon update lock poisoned"))?;
            self.dir(false)?.remove_file(&format!("{id}.bin"))
        })();
        if let Err(e) = result
            && e.kind() != io::ErrorKind::NotFound
        {
            (self.warning)(id, &e)
        }
    }
    pub fn version(&self, id: &str) -> String {
        if !valid_icon_id(id) {
            return String::new();
        }
        self.dir(false)
            .and_then(|d| d.metadata(&format!("{id}.bin")))
            .map(version_metadata)
            .unwrap_or_default()
    }
}
pub fn valid_icon_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}
fn version_metadata(info: std::fs::Metadata) -> String {
    if !info.is_file() {
        return String::new();
    }
    let Ok(at) = info.modified() else {
        return String::new();
    };
    let nanos = match at.duration_since(std::time::UNIX_EPOCH) {
        Ok(v) => v.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    };
    if nanos < 0 {
        format!("-{:x}-{:x}", -nanos, info.len())
    } else {
        format!("{nanos:x}-{:x}", info.len())
    }
}
pub fn icon_version(paths: &RuntimePaths, id: &str) -> String {
    if !valid_icon_id(id) {
        return String::new();
    }
    Dir::open(&paths.resource(Resource::ProviderIcons))
        .and_then(|d| d.metadata(&format!("{id}.bin")))
        .map(version_metadata)
        .unwrap_or_default()
}
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    crate::routine::memo::sniff_image(bytes).map(|ext| match ext {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "gif" => "image/gif",
        _ => "image/webp",
    })
}
