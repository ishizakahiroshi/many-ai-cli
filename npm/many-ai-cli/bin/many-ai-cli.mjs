#!/usr/bin/env node
// Thin launcher for the `many-ai-cli` npm package.
//
// The real program is a native Go binary shipped in a platform-specific
// optional dependency (e.g. `many-ai-cli-win32-x64`). This shim resolves the
// matching package for the current OS/arch and execs its bundled binary,
// forwarding argv, stdio, and the exit code / terminating signal.
//
// Distributing the binary through npm (rather than a browser-downloaded exe)
// means the launcher is materialized locally at install time, so it carries no
// Mark-of-the-Web and does not trigger the Windows SmartScreen prompt.

import { spawnSync } from 'node:child_process';
import { chmodSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

// process.platform/arch -> optional dependency package name.
// Package names mirror the GitHub release archive suffixes
// (windows-x64 / linux-x64 / macos-intel / macos-apple-silicon) rather than
// Node's win32/darwin tokens, so the npm and GitHub artifact naming line up.
// The packages' own os/cpu fields still use Node conventions (win32/darwin/...).
const PKG_BY_TARGET = {
  'win32 x64': 'many-ai-cli-windows-x64',
  'linux x64': 'many-ai-cli-linux-x64',
  'darwin x64': 'many-ai-cli-macos-intel',
  'darwin arm64': 'many-ai-cli-macos-apple-silicon',
};

function resolveBinary() {
  const target = `${process.platform} ${process.arch}`;
  const pkg = PKG_BY_TARGET[target];
  if (!pkg) {
    const supported = Object.keys(PKG_BY_TARGET).join(', ');
    throw new Error(
      `many-ai-cli does not ship a prebuilt binary for ${target}.\n` +
        `Supported targets: ${supported}.\n` +
        `Install from GitHub Releases instead: https://github.com/ishizakahiroshi/many-ai-cli/releases`
    );
  }
  const subpath = `${pkg}/bin/many-ai-cli${process.platform === 'win32' ? '.exe' : ''}`;
  try {
    return require.resolve(subpath);
  } catch {
    throw new Error(
      `The platform package "${pkg}" is not installed.\n` +
        `It is an optionalDependency of many-ai-cli and should be installed automatically.\n` +
        `Reinstall with optional dependencies enabled, e.g. "pnpm add -g many-ai-cli" ` +
        `(avoid --no-optional / --ignore-optional).`
    );
  }
}

function main() {
  let binary;
  try {
    binary = resolveBinary();
  } catch (err) {
    process.stderr.write(`${err.message}\n`);
    process.exit(1);
  }

  // The platform binary must carry the unix exec bit on Linux/macOS. npm
  // preserves the file mode from the published tarball, but a tarball packed on
  // Windows (which has no exec bit) arrives as 0644 and spawnSync would fail
  // with EACCES. Restore it defensively so the package works regardless of where
  // it was packed — this removes the need to pack on WSL/Linux just for the bit.
  // Best-effort: a root-owned global install run as a non-root user may reject
  // chmod, in which case the binary was already 0755 from a posix pack anyway.
  if (process.platform !== 'win32') {
    try {
      chmodSync(binary, 0o755);
    } catch {
      // ignore — fall through to spawn, which works if the mode is already ok.
    }
  }

  const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });

  if (result.error) {
    process.stderr.write(`many-ai-cli: failed to launch binary: ${result.error.message}\n`);
    process.exit(1);
  }
  // Propagate a terminating signal as the conventional 128+signal exit code.
  if (result.signal) {
    process.exit(1);
  }
  process.exit(result.status ?? 0);
}

main();
