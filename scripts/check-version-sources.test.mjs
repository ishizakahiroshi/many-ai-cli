import test from 'node:test';
import assert from 'node:assert/strict';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, symlinkSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import { checkVersionWiring } from './check-version-sources.mjs';

function fixture() {
  return new Map([
    ['.github/workflows/release.yml', 'python3 scripts/rust-candidate-ci.py --target x --release-tag "$RELEASE_TAG"\nnode scripts/sync-npm-version.mjs "$RELEASE_TAG"'],
    ['scripts/rust-candidate-ci.py', `
parser.add_argument("--release-tag")
def release_version(tag):
    return tag[1:]
version = release_version(release_tag)
env.update({"MANY_AI_BUILD_VERSION": stamp['version'],
            "MANY_AI_BUILD_COMMIT": stamp['build_commit'],
            "MANY_AI_BUILD_TIME": stamp['build_time']})
resources = prepare_windows_resources(resource_dir, stamp['version'], env)
VALUE "FileVersion", "{version}"
VALUE "ProductVersion", "{version}"
`],
    ['rust/build.rs', `
link_windows_release_resources();
for binary in ["many-ai-cli", "many-ai-cli-launcher"] {
    println!("cargo:rustc-link-arg-bin={binary}={}", resource.display());
}
for name in ["MANY_AI_BUILD_VERSION", "MANY_AI_BUILD_COMMIT", "MANY_AI_BUILD_TIME"] {
    println!("cargo:rustc-env={name}={}", env::var(name).unwrap());
}
`],
  ]);
}

test('formal Rust and npm tag writers are connected without frozen Go files', () => {
  const files = fixture();
  assert.deepEqual(checkVersionWiring((file) => files.get(file)), []);
});

test('a constant binary version cannot masquerade as a formal stamp', () => {
  const files = fixture();
  files.set('scripts/rust-candidate-ci.py', files.get('scripts/rust-candidate-ci.py').replace(`stamp['version']`, '"0.3.1"'));
  assert.ok(checkVersionWiring((file) => files.get(file)).some((p) => p.includes('Cargo version')));
});

test('removing the caller tag argument blocks the release wiring', () => {
  const files = fixture();
  files.set('.github/workflows/release.yml', files.get('.github/workflows/release.yml').replace(' --release-tag "$RELEASE_TAG"', ''));
  assert.ok(checkVersionWiring((file) => files.get(file)).some((p) => p.includes('runner receives')));
});

test('unlinking Windows resources or freezing their version is detected', () => {
  for (const [file, original] of [['rust/build.rs', 'link_windows_release_resources();'], ['scripts/rust-candidate-ci.py', `prepare_windows_resources(resource_dir, stamp['version'], env)`]]) {
    const files = fixture();
    files.set(file, files.get(file).replace(original, 'removed'));
    assert.ok(checkVersionWiring((path) => files.get(path)).length > 0);
  }
});

test('Go-only writers and absent source files cannot satisfy a Rust release', () => {
  const files = new Map([['go/.goreleaser.yaml', '-X main.version={{ .Version }} go-winres --file-version={{ .Version }}']]);
  const problems = checkVersionWiring((file) => {
    if (!files.has(file)) throw new Error('missing source');
    return files.get(file);
  });
  assert.ok(problems.some((p) => p.includes('formal native runner')));
  assert.ok(problems.some((p) => p.includes('Windows resource linker')));
});

test('a run through a junction or symlink still checks (and fails when wiring is absent)', () => {
  const work = mkdtempSync(join(tmpdir(), 'version-sources-link-'));
  try {
    const repo = join(work, 'repo');
    mkdirSync(join(repo, 'scripts'), { recursive: true });
    copyFileSync(new URL('./check-version-sources.mjs', import.meta.url), join(repo, 'scripts/check-version-sources.mjs'));
    const link = join(work, 'link');
    symlinkSync(repo, link, 'junction');
    for (const root of [repo, link]) {
      const r = spawnSync(process.execPath, [join(root, 'scripts', 'check-version-sources.mjs')], { encoding: 'utf8' });
      assert.equal(r.status, 1, `${root}: ${r.stdout}${r.stderr}`);
      assert.match(r.stderr, /version wiring is broken/);
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
});
