//! Flatten only manifest-selected regular files into a pinned private directory.
use super::*;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Write},
};
pub struct Cleanup {
    directory: Arc<Dir>,
    name: String,
}
impl Cleanup {
    pub fn new(directory: Arc<Dir>, name: String) -> Self {
        Self { directory, name }
    }
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = self.directory.remove_file(&self.name);
        let _ = self
            .directory
            .remove_file(&format!("{}.download", self.name));
    }
}
// Temporary names live under the same held directory capability as the final
// files, so a rename of its path cannot redirect extraction or publication.
struct Extraction {
    directory: Arc<Dir>,
    staged: Vec<(String, String)>,
    published: Vec<String>,
    committed: bool,
}
impl Extraction {
    fn new(directory: Arc<Dir>) -> Self {
        Self {
            directory,
            staged: Vec::new(),
            published: Vec::new(),
            committed: false,
        }
    }

    fn copy(
        &mut self,
        reader: &mut dyn Read,
        name: &str,
        size: u64,
        total: &mut u64,
        cancel: &Cancellation,
    ) -> Result<(), WhisperError> {
        check_cancelled(cancel)?;
        let temporary = format!(".whisper-extract-{}.tmp", crate::process::random_token()?);
        // Reserve exclusively before registering cleanup: a collision must never
        // make this attempt overwrite or remove somebody else's file.
        self.directory.create_new(&temporary, &[], 0o700)?;
        self.staged.push((name.to_owned(), temporary.clone()));
        copy(reader, &self.directory, &temporary, size, total, cancel)
    }

    fn publish(
        &mut self,
        binary: &manifest::Binary,
        cancel: &Cancellation,
    ) -> Result<(), WhisperError> {
        // The executable is the manager's installed/readiness marker. Publish it
        // only after every dependency is complete and in place; nothing fallible
        // follows its final rename. Existing files are never replaced.
        self.staged.sort_by_key(|(name, _)| {
            binary
                .server_names
                .iter()
                .any(|server| name.eq_ignore_ascii_case(server))
        });
        for (name, temporary) in &self.staged {
            check_cancelled(cancel)?;
            self.directory.rename_to(temporary, &self.directory, name)?;
            self.published.push(name.clone());
        }
        self.committed = true;
        Ok(())
    }
}
impl Drop for Extraction {
    fn drop(&mut self) {
        if !self.committed {
            for name in self.published.iter().rev() {
                let _ = self.directory.remove_file(name);
            }
        }
        for (_, temporary) in &self.staged {
            let _ = self.directory.remove_file(temporary);
        }
    }
}

fn check_cancelled(cancel: &Cancellation) -> Result<(), WhisperError> {
    if cancel.is_cancelled() {
        Err(WhisperError::new(
            499,
            "cancelled",
            "Whisper extraction cancelled",
        ))
    } else {
        Ok(())
    }
}

const CAP: u64 = 256 * 1024 * 1024;
const MEMBERS: usize = 1024;
#[cfg(test)]
pub(super) fn test_copy_bound(
    reader: &mut dyn Read,
    dir: &Dir,
    total: &mut u64,
) -> Result<(), WhisperError> {
    copy(
        reader,
        dir,
        "bomb",
        CAP + 1,
        total,
        &Cancellation::default(),
    )
}
fn selected(path: &str, binary: &manifest::Binary) -> Result<Option<String>, WhisperError> {
    let path = path.replace('\\', "/");
    if path.starts_with('/')
        || path
            .split('/')
            .any(|part| part == ".." || part.contains(':'))
    {
        return Err(WhisperError::new(
            500,
            "whisper_install_failed",
            "unsafe Whisper archive path",
        ));
    }
    let leaf = path.rsplit('/').next().unwrap_or("");
    Ok(binary
        .keep
        .iter()
        .find(|name| leaf.eq_ignore_ascii_case(name))
        .map(|name| (*name).to_owned()))
}
fn copy(
    reader: &mut dyn Read,
    dir: &Dir,
    name: &str,
    size: u64,
    total: &mut u64,
    cancel: &Cancellation,
) -> Result<(), WhisperError> {
    *total = total
        .checked_add(size)
        .filter(|value| *value <= CAP)
        .ok_or_else(|| {
            WhisperError::new(
                500,
                "whisper_install_failed",
                "Whisper archive extraction bound exceeded",
            )
        })?;
    let mut output = dir.open_file(name, true)?;
    let mut remaining = size;
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        check_cancelled(cancel)?;
        let limit = buffer.len().min(remaining as usize);
        let count = reader.read(&mut buffer[..limit])?;
        if count == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "archive truncated").into());
        }
        output.write_all(&buffer[..count])?;
        remaining -= count as u64;
    }
    // Reading EOF also forces ZIP CRC verification rather than trusting size.
    if reader.read(&mut buffer[..1])? != 0 {
        return Err(WhisperError::new(
            500,
            "whisper_install_failed",
            "Whisper archive size mismatch",
        ));
    }
    output.sync_all()?;
    Ok(())
}
pub fn extract(
    file: File,
    directory: Arc<Dir>,
    binary: &manifest::Binary,
    cancel: &Cancellation,
) -> Result<(), WhisperError> {
    check_cancelled(cancel)?;
    let mut extraction = Extraction::new(directory);
    let mut found = BTreeSet::new();
    let mut total = 0;
    match binary.archive {
        "zip" => {
            let mut archive = zip::ZipArchive::new(file).map_err(|_| {
                WhisperError::new(500, "whisper_install_failed", "invalid Whisper ZIP archive")
            })?;
            if archive.len() > MEMBERS {
                return Err(WhisperError::new(
                    500,
                    "whisper_install_failed",
                    "Whisper archive member bound exceeded",
                ));
            }
            for index in 0..archive.len() {
                check_cancelled(cancel)?;
                let mut entry = archive.by_index(index).map_err(|_| {
                    WhisperError::new(500, "whisper_install_failed", "invalid Whisper ZIP entry")
                })?;
                let chosen = selected(entry.name(), binary)?;
                if entry.is_dir() {
                    continue;
                }
                if entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
                {
                    return Err(WhisperError::new(
                        500,
                        "whisper_install_failed",
                        "Whisper archive symlink rejected",
                    ));
                }
                if let Some(name) = chosen {
                    if !found.insert(name.clone()) {
                        return Err(WhisperError::new(
                            500,
                            "whisper_install_failed",
                            "duplicate Whisper archive file",
                        ));
                    }
                    let size = entry.size();
                    extraction.copy(&mut entry, &name, size, &mut total, cancel)?;
                }
            }
        }
        "tar.gz" => {
            let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
            let mut declared = 0u64;
            for (index, item) in archive.entries()?.enumerate() {
                check_cancelled(cancel)?;
                if index >= MEMBERS {
                    return Err(WhisperError::new(
                        500,
                        "whisper_install_failed",
                        "Whisper archive member bound exceeded",
                    ));
                }
                let mut entry = item?;
                declared = declared
                    .checked_add(entry.size())
                    .filter(|size| *size <= CAP)
                    .ok_or_else(|| {
                        WhisperError::new(
                            500,
                            "whisper_install_failed",
                            "Whisper archive extraction bound exceeded",
                        )
                    })?;
                let path = entry.path()?.to_string_lossy().into_owned();
                let chosen = selected(&path, binary)?;
                if !entry.header().entry_type().is_file() {
                    if chosen.is_some() {
                        return Err(WhisperError::new(
                            500,
                            "whisper_install_failed",
                            "Whisper archive nonregular file rejected",
                        ));
                    }
                    continue;
                }
                if let Some(name) = chosen {
                    if !found.insert(name.clone()) {
                        return Err(WhisperError::new(
                            500,
                            "whisper_install_failed",
                            "duplicate Whisper archive file",
                        ));
                    }
                    let size = entry.size();
                    extraction.copy(&mut entry, &name, size, &mut total, cancel)?;
                }
            }
            // tar stops at its end marker before gzip necessarily validates its
            // CRC/trailer. Drain the remaining decoded bytes before publishing,
            // with cancellation and a bound on any data after the tar members.
            let mut decoder = archive.into_inner();
            let mut remaining = CAP;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                check_cancelled(cancel)?;
                let count = decoder.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                remaining = remaining.checked_sub(count as u64).ok_or_else(|| {
                    WhisperError::new(
                        500,
                        "whisper_install_failed",
                        "Whisper archive extraction bound exceeded",
                    )
                })?;
            }
        }
        _ => {
            return Err(WhisperError::new(
                500,
                "whisper_install_failed",
                "unsupported Whisper archive format",
            ));
        }
    }
    for name in binary.keep {
        if !found.contains(*name) {
            return Err(WhisperError::new(
                500,
                "whisper_install_failed",
                format!("release archive missing {name}"),
            ));
        }
    }
    extraction.publish(binary, cancel)
}
