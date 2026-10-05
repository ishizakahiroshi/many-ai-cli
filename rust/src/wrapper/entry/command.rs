//! Trial lookup is intentionally stricter than the production Go fallback. A
//! missing/bare command must never be rediscovered against the host PATH by the
//! PTY or pipe process owner. This is an executable selection boundary, not a
//! sandbox for arbitrary code intentionally placed in the selected trial root.
use crate::{
    config::RuntimePaths,
    files::safe_fs::Dir,
    process::execpath::{ExecutableFs, NativeFs, Platform, ResolvedCommand, Resolver},
};
use std::{io, io::Read, path::Path, path::PathBuf};

fn boundary() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "trial wrapper requires an executable and working directory inside its runtime root",
    )
}
fn confined(paths: &RuntimePaths, path: &Path) -> io::Result<PathBuf> {
    crate::profile::subscriptions::check_path(paths, path).map_err(|_| boundary())?;
    // Canonicalize the existing executable, not merely its nearest ancestor:
    // an unresolved name cannot reach the process owner's PATH fallback.
    let path = path.canonicalize().map_err(|_| boundary())?;
    if !path.starts_with(paths.root()) {
        return Err(boundary());
    }
    Ok(path)
}

/// Constrain shim discovery as well as its eventual process launch. In trial,
/// the Windows resolver must not read an external .cmd file while preparing a
/// command that will only later be rejected.
struct ScopedFs<'a>(&'a RuntimePaths);
impl ExecutableFs for ScopedFs<'_> {
    fn exists(&self, path: &str) -> bool {
        if self.0.is_trial() && confined(self.0, Path::new(path)).is_err() {
            return false;
        }
        NativeFs.exists(path)
    }
    fn is_executable(&self, path: &str, platform: Platform) -> bool {
        if self.0.is_trial() && confined(self.0, Path::new(path)).is_err() {
            return false;
        }
        NativeFs.is_executable(path, platform)
    }
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        if !self.0.is_trial() {
            return NativeFs.read(path);
        }
        let path = confined(self.0, Path::new(path))?;
        let parent = Dir::open(path.parent().ok_or_else(boundary)?)?;
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(boundary)?;
        let mut bytes = Vec::new();
        parent.open_file(name, false)?.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
    fn same_file(&self, first: &str, second: &str) -> bool {
        (!self.0.is_trial() || (self.exists(first) && self.exists(second)))
            && NativeFs.same_file(first, second)
    }
}

pub(super) struct CommandResolver<'a> {
    paths: &'a RuntimePaths,
    cwd: &'a Path,
    environment: &'a [String],
}
impl<'a> CommandResolver<'a> {
    pub(super) fn new(paths: &'a RuntimePaths, environment: &'a [String], cwd: &'a Path) -> Self {
        Self {
            paths,
            cwd,
            environment,
        }
    }
    fn check(&self, mut command: ResolvedCommand) -> io::Result<ResolvedCommand> {
        if !self.paths.is_trial() {
            return Ok(command);
        }
        if !confined(self.paths, self.cwd)?.is_dir() {
            return Err(boundary());
        }
        let executable = Path::new(&command.executable);
        let executable = if executable.is_absolute() {
            executable.to_owned()
        } else {
            self.cwd.join(executable)
        };
        let executable = confined(self.paths, &executable)?;
        if !executable.is_file()
            || !NativeFs.is_executable(&executable.to_string_lossy(), Platform::native())
        {
            return Err(boundary());
        }
        command.executable = executable.to_str().ok_or_else(boundary)?.into();
        if let Some(shim) = command.shell_shim.as_ref() {
            let shim = self.check_shim(shim)?;
            // Resolver's shell fallback always stores the shim after /c.
            command.args[1] = shim.clone();
            command.shell_shim = Some(shim);
        }
        Ok(command)
    }
    fn check_shim(&self, shim: &str) -> io::Result<String> {
        let path = Path::new(shim);
        let path = if path.is_absolute() {
            path.into()
        } else {
            self.cwd.join(path)
        };
        Ok(confined(self.paths, &path)?
            .to_str()
            .ok_or_else(boundary)?
            .into())
    }
    pub(super) fn resolve_provider(
        &self,
        provider: &str,
        custom: Option<&[String]>,
        args: &[String],
    ) -> io::Result<ResolvedCommand> {
        let fs = ScopedFs(self.paths);
        let resolver = Resolver::new(Platform::native(), self.environment, self.cwd, &fs);
        self.check(resolver.resolve_provider(provider, custom, args))
    }
    pub(super) fn launch_shell_shim(
        &self,
        provider: &str,
        custom: Option<&[String]>,
    ) -> io::Result<Option<String>> {
        let fs = ScopedFs(self.paths);
        let resolver = Resolver::new(Platform::native(), self.environment, self.cwd, &fs);
        // Preserve the source's probe argument and direct cmd.exe /c detection.
        self.check(resolver.resolve_provider(provider, custom, &["probe".into()]))?;
        resolver
            .launch_shell_shim(provider, custom)
            .map(|shim| {
                if self.paths.is_trial() {
                    self.check_shim(&shim)
                } else {
                    Ok(shim)
                }
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests;
