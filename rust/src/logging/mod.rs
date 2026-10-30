//! Private rolling log owner. Application audit and Hub diagnostics share this
//! storage component; callers supply the live log configuration explicitly.
mod diagnostic;
pub(crate) use diagnostic::write_diagnostic;

use crate::{
    config::LogConfig,
    files::safe_fs::{self, Dir},
};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, Write},
    sync::{Arc, Mutex},
};
struct State {
    file: Option<File>,
    size: u64,
}
pub struct RollingLog {
    directory: Arc<Dir>,
    name: String,
    state: Mutex<State>,
}
impl RollingLog {
    pub fn new(directory: Arc<Dir>, name: &str) -> io::Result<Self> {
        safe_fs::basename(name)?;
        Ok(Self {
            directory,
            name: name.into(),
            state: Mutex::new(State {
                file: None,
                size: 0,
            }),
        })
    }
    pub fn write(&self, config: &LogConfig, bytes: &[u8]) -> io::Result<()> {
        self.write_at(config, bytes, chrono::Utc::now())
    }
    fn write_at(
        &self,
        config: &LogConfig,
        bytes: &[u8],
        now: chrono::DateTime<chrono::Utc>,
    ) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.write_locked(&mut state, config, bytes, now)
    }
    fn write_locked(
        &self,
        state: &mut State,
        config: &LogConfig,
        bytes: &[u8],
        now: chrono::DateTime<chrono::Utc>,
    ) -> io::Result<()> {
        let maximum = if config.max_size_mb > 0 {
            config.max_size_mb as u64
        } else {
            100
        }
        .saturating_mul(1024 * 1024);
        let length = bytes.len() as u64;
        if length > maximum {
            return Err(io::Error::other("log write exceeds maximum file size"));
        }
        if state.file.is_none() {
            // Every maintenance failure is diagnosed, but must not suppress
            // writes to an otherwise usable active log (Lumberjack parity).
            let _ = self.cleanup(config);
            let size = match self.directory.metadata(&self.name) {
                Ok(metadata) => metadata.len(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
                Err(error) => return Err(error),
            };
            if size.saturating_add(length) >= maximum {
                self.rotate(state, config, now)?;
            } else {
                state.file = Some(self.directory.open_append(&self.name)?);
                state.size = size;
            }
        }
        if state.size.saturating_add(length) > maximum {
            self.rotate(state, config, now)?;
        }
        state
            .file
            .as_mut()
            .expect("log owner opened file")
            .write_all(bytes)?;
        state.size = state.size.saturating_add(length);
        Ok(())
    }
    pub fn close(&self) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(mut file) = state.file.take() {
            file.flush()?;
        }
        Ok(())
    }
    /// Go's audit constructs and closes a roller for each line, including its
    /// inclusive existing-file threshold on the next write.
    pub fn write_and_close(&self, config: &LogConfig, bytes: &[u8]) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let result = self.write_locked(&mut state, config, bytes, chrono::Utc::now());
        let closing = match state.file.take() {
            Some(mut file) => file.flush(),
            None => Ok(()),
        };
        result.and(closing)
    }
    fn rotate(
        &self,
        state: &mut State,
        config: &LogConfig,
        now: chrono::DateTime<chrono::Utc>,
    ) -> io::Result<()> {
        state.file.take();
        match self.directory.metadata(&self.name) {
            Ok(_) => {
                let (stem, ext) = self.parts();
                let backup = format!("{stem}-{}{ext}", now.format("%Y-%m-%dT%H-%M-%S%.3f"));
                self.directory
                    .rename_to(&self.name, &self.directory, &backup)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        state.file = Some(self.directory.open_append(&self.name)?);
        state.size = 0;
        // Cleanup reports each failure and still attempts the remaining files.
        let _ = self.cleanup(config);
        Ok(())
    }
    fn parts(&self) -> (&str, &str) {
        match self.name.rfind('.') {
            Some(at) => (&self.name[..at], &self.name[at..]),
            None => (&self.name, ""),
        }
    }
    fn backups(&self) -> io::Result<Vec<(String, String)>> {
        let (stem, ext) = self.parts();
        let prefix = format!("{stem}-");
        let mut files = Vec::new();
        for name in self.directory.entries()? {
            let plain = name.strip_suffix(".gz").unwrap_or(&name);
            let Some(time) = plain
                .strip_prefix(&prefix)
                .and_then(|part| part.strip_suffix(ext))
            else {
                continue;
            };
            if time.len() != 23
                || chrono::NaiveDateTime::parse_from_str(time, "%Y-%m-%dT%H-%M-%S%.3f").is_err()
            {
                continue;
            }
            files.push((name.clone(), plain.to_owned()));
        }
        files.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(files)
    }
    fn cleanup(&self, config: &LogConfig) -> io::Result<()> {
        self.cleanup_with_diagnostics(config, &mut write_diagnostic)
    }
    fn cleanup_with_diagnostics(
        &self,
        config: &LogConfig,
        diagnostic: &mut impl FnMut(&'static str),
    ) -> io::Result<()> {
        if config.max_backups <= 0 && !config.compress {
            return Ok(());
        }
        let backups = self.backups().inspect_err(|_| {
            diagnostic("Rolling log backup listing failed\n");
        })?;
        cleanup_backups(
            config,
            backups,
            |name| self.directory.metadata(name).map(|_| ()),
            |name| self.directory.remove_file(name),
            |name| self.compress(name),
            diagnostic,
        )
    }
    fn compress(&self, name: &str) -> io::Result<()> {
        let target = format!("{name}.gz");
        let temporary = format!(".many-ai-cli-{}.tmp", crate::process::random_token()?);
        self.directory.create_new(&temporary, &[], 0o600)?;
        let result = (|| {
            let mut input = self.directory.open_file(name, false)?;
            let output = self.directory.open_file(&temporary, true)?;
            let mut encoder = flate2::write::GzEncoder::new(output, flate2::Compression::default());
            io::copy(&mut input, &mut encoder)?;
            let file = encoder.finish()?;
            file.sync_all()?;
            drop(file);
            drop(input);
            // Compression is allowed to replace an older completed gzip for
            // this same recognized backup; final entries stay directory-bound.
            match self.directory.remove_file(&target) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            self.directory
                .rename_to(&temporary, &self.directory, &target)?;
            self.directory.remove_file(name)
        })();
        if result.is_err() {
            let _ = self.directory.remove_file(&temporary);
        }
        result
    }
}
fn cleanup_backups(
    config: &LogConfig,
    backups: Vec<(String, String)>,
    mut inspect: impl FnMut(&str) -> io::Result<()>,
    mut remove: impl FnMut(&str) -> io::Result<()>,
    mut compress: impl FnMut(&str) -> io::Result<()>,
    diagnostic: &mut impl FnMut(&'static str),
) -> io::Result<()> {
    let mut first_error = None;
    let mut failed = |error, message| {
        // Never include backup names, paths, or raw OS errors in diagnostics.
        diagnostic(message);
        if first_error.is_none() {
            first_error = Some(error);
        }
    };
    let mut preserved = BTreeSet::new();
    let mut removals = Vec::new();
    let mut compressions = Vec::new();
    for (name, plain) in backups {
        // The directory capability rejects directories, aliases and other
        // non-regular entries. Skip them before counting retained backups.
        if let Err(error) = inspect(&name) {
            failed(error, "Rolling log backup inspection failed\n");
            continue;
        }
        preserved.insert(plain);
        if config.max_backups > 0 && preserved.len() > config.max_backups as usize {
            removals.push(name);
        } else if config.compress && !name.ends_with(".gz") {
            compressions.push(name);
        }
    }
    // Lumberjack attempts every removal and then every compression, returning
    // the first error only after the rest of the maintenance work has run.
    for name in removals {
        if let Err(error) = remove(&name) {
            failed(error, "Rolling log backup removal failed\n");
        }
    }
    for name in compressions {
        if let Err(error) = compress(&name) {
            failed(error, "Rolling log backup compression failed\n");
        }
    }
    first_error.map_or(Ok(()), Err)
}
#[cfg(test)]
mod tests;
