#!/usr/bin/env node
// Tags are the version source. Stored Cargo/npm/winres versions may remain old.
// Check the *writers*, rather than comparing stored versions with today's tag.
// The frozen Go release wiring is no longer evidence for a Rust release.
// This source guard complements actual runner/resource/package tests; it does
// not prove a binary's stamped version or a hosted release has succeeded.
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const WIRING = [
  ['.github/workflows/release.yml', /scripts\/rust-candidate-ci\.py[^\r\n]+--release-tag\s+(?:"\$RELEASE_TAG"|\$env:RELEASE_TAG)/, 'formal native runner receives the release tag'],
  ['.github/workflows/release.yml', /scripts\/sync-npm-version\.mjs\s+"\$RELEASE_TAG"/, 'npm package versions receive the same tag'],
  ['scripts/rust-candidate-ci.py', /parser\.add_argument\(["']--release-tag["']/, 'native CLI accepts explicit formal mode'],
  ['scripts/rust-candidate-ci.py', /version\s*=\s*release_version\(release_tag\)/, 'formal version derives from the validated tag'],
  ['scripts/rust-candidate-ci.py', /return\s+tag\[1:\]/, 'version drops only the tag prefix'],
  ['scripts/rust-candidate-ci.py', /["']MANY_AI_BUILD_VERSION["']:\s*stamp\[["']version["']\]/, 'Cargo version uses the formal stamp'],
  ['scripts/rust-candidate-ci.py', /["']MANY_AI_BUILD_COMMIT["']:\s*stamp\[["']build_commit["']\]/, 'Cargo commit uses the formal stamp'],
  ['scripts/rust-candidate-ci.py', /["']MANY_AI_BUILD_TIME["']:\s*stamp\[["']build_time["']\]/, 'Cargo timestamp uses the formal stamp'],
  ['scripts/rust-candidate-ci.py', /prepare_windows_resources\(resource_dir,\s*stamp\[["']version["']\],\s*env\)/, 'Windows resources use that same version'],
  ['scripts/rust-candidate-ci.py', /VALUE\s+"FileVersion",\s*"\{version\}/, 'Windows file version embeds the formal version'],
  ['scripts/rust-candidate-ci.py', /VALUE\s+"ProductVersion",\s*"\{version\}/, 'Windows product version embeds the formal version'],
  ['rust/build.rs', /link_windows_release_resources\(\);/, 'build script calls the Windows resource linker'],
  ['rust/build.rs', /cargo:rustc-link-arg-bin=\{binary\}/, 'Windows resources are linked per binary'],
  ...['MANY_AI_BUILD_VERSION', 'MANY_AI_BUILD_COMMIT', 'MANY_AI_BUILD_TIME'].map((name) => ['rust/build.rs', new RegExp(`"${name}"`), `build script forwards ${name}`]),
  ['rust/build.rs', /cargo:rustc-env=\{name\}=\{\}/, 'build script forwards stamp values into Rust compilation'],
];

export function checkVersionWiring(read = (file) => readFileSync(join(repoRoot, file), 'utf8')) {
  const problems = [];
  const cache = new Map();
  for (const [file, needle, what] of WIRING) {
    if (!cache.has(file)) {
      try { cache.set(file, read(file)); }
      catch { cache.set(file, null); }
    }
    const source = cache.get(file);
    if (source === null || !needle.test(source)) problems.push(`${file}: ${what}`);
  }
  return problems;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const problems = checkVersionWiring();
  if (problems.length) {
    console.error('NG: tag-derived Rust version wiring is broken\n' + problems.map((p) => `  - ${p}`).join('\n'));
    process.exitCode = 1;
  } else {
    console.log(`OK: tag remains the version source (${WIRING.length} Rust/npm/resource wiring checks)`);
  }
}
