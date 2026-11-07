#!/usr/bin/env node
// Thin launcher for the `many-ai-cli` npm package.
//
// The real program is a native binary shipped in a platform-specific
// optional dependency (e.g. `many-ai-cli-windows-x64`). This shim finds the
// matching package for the current OS/arch inside its own installation and
// execs its bundled binary, forwarding argv and stdio and reporting its exit
// status.
//
// The shim is independent of the implementation language. Rust release
// candidates use the same package names and binary paths; this file alone does
// not establish which release is published to the registry. Package-manager
// installation does not guarantee that Windows security policies allow an exe.

import { spawnSync } from 'node:child_process';
import { chmodSync, readFileSync, statSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

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

// This package's directory (the shim is bin/many-ai-cli.mjs). Node starts the
// shim from its real path, so a package manager's link is already followed.
const PACKAGE_DIR = dirname(dirname(fileURLToPath(import.meta.url)));

// The platform package is looked for only inside the installation that holds
// this shim: this package's own node_modules, then the node_modules of each
// directory above it, up to the directory that contains the outermost
// node_modules on the shim's path. require.resolve would go on to every
// node_modules up to the drive root, NODE_PATH and the global folders; on
// Windows any signed-in user can create C:\node_modules, so a missing or
// incomplete platform package would start their executable with this user's
// rights (audit U38). Candidates are judged by where they sit, never by where
// a link points: pnpm links the platform package to a store directory outside
// this tree.
function searchDirs(pkg) {
  const chain = [];
  for (let dir = PACKAGE_DIR; ; ) {
    chain.push(dir);
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  let rootIndex = 0;
  chain.forEach((dir, index) => {
    if (basename(dir) === 'node_modules' && index + 1 < chain.length) rootIndex = index + 1;
  });
  return {
    installRoot: chain[rootIndex],
    dirs: chain
      .slice(0, rootIndex + 1)
      .filter((dir) => basename(dir) !== 'node_modules')
      .map((dir) => join(dir, 'node_modules', pkg)),
  };
}

function stat(path) {
  try {
    return statSync(path);
  } catch {
    return undefined;
  }
}

function readManifest(dir) {
  try {
    return JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
  } catch {
    return undefined;
  }
}

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
  const binName = `many-ai-cli${process.platform === 'win32' ? '.exe' : ''}`;
  // The root package pins the platform package to its exact version.
  const wanted = readManifest(PACKAGE_DIR)?.optionalDependencies?.[pkg];
  const { installRoot, dirs } = searchDirs(pkg);
  const problems = [];
  if (typeof wanted !== 'string' || !wanted) {
    problems.push(`many-ai-cli's package.json does not name the version of ${pkg}.`);
  } else {
    for (const dir of dirs) {
      const manifest = readManifest(dir);
      if (manifest === undefined) {
        if (stat(dir)) problems.push(`${dir} has no readable package.json.`);
        continue;
      }
      if (manifest?.name !== pkg || manifest?.version !== wanted) {
        problems.push(
          `${dir} holds ${manifest?.name}@${manifest?.version}; this many-ai-cli needs ${pkg}@${wanted}.`
        );
        continue;
      }
      const binary = join(dir, 'bin', binName);
      if (stat(binary)?.isFile()) return binary;
      problems.push(`${dir} has no bin/${binName}.`);
    }
  }
  throw new Error(
    `The platform package "${pkg}" is not installed.\n` +
      `It is an optionalDependency of many-ai-cli and should be installed automatically.\n` +
      `Reinstall with optional dependencies enabled, e.g. "pnpm add -g many-ai-cli" ` +
      `(avoid --no-optional / --ignore-optional).\n` +
      `many-ai-cli looks for it only inside its own installation: ${installRoot}` +
      problems.map((problem) => `\n${problem}`).join('')
  );
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
