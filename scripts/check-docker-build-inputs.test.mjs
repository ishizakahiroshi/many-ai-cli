import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  checkDockerfileInputs,
  checkDockerignoreInputs,
  checkDockerInputs,
  checkProviderInputs,
} from './check-docker-build-inputs.mjs';

const dockerfile = readFileSync('deploy/docker/Dockerfile', 'utf8');
const dockerfileLf = dockerfile.replaceAll('\r\n', '\n');
const dockerignore = readFileSync('deploy/docker/Dockerfile.dockerignore', 'utf8');
const packageJson = JSON.parse(readFileSync('deploy/docker/provider-cli/package.json', 'utf8'));
const packageLock = JSON.parse(readFileSync('deploy/docker/provider-cli/package-lock.json', 'utf8'));

function hasError(errors, text) {
  return errors.some((error) => error.includes(text));
}

test('current Docker inputs are accepted', () => {
  assert.deepEqual(checkDockerInputs({ dockerfile, dockerignore, packageJson, packageLock }), []);
});

test('digest-less FROM is rejected', () => {
  const fixture = dockerfile.replace(
    'FROM node:22-bookworm@sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d AS node-runtime',
    'FROM node:22-bookworm AS node-runtime',
  );
  assert.ok(hasError(checkDockerfileInputs(fixture), 'FROM image'));
});

test('digest-less external COPY source is rejected', () => {
  const fixture = dockerfile.replace('COPY --from=bun-runtime /usr/local/bin/bun', 'COPY --from=oven/bun:1.3.14 /usr/local/bin/bun');
  assert.ok(hasError(checkDockerfileInputs(fixture), 'COPY external image'));
});

test('curl to bash and curl to tar are rejected across logical lines', () => {
  const fixture = String.raw`${dockerfileLf}
RUN curl -fSL "https://example.invalid/archive.tar.gz" \
  | tar -xzf -`;
  const errors = checkDockerfileInputs(fixture);
  assert.ok(hasError(errors, 'unverified download piped'));

  const bashFixture = fixture.replace('| tar -xzf -', '| bash');
  assert.ok(hasError(checkDockerfileInputs(bashFixture), 'unverified download piped'));
});

test('checksum before extraction is accepted and extraction before checksum is rejected', () => {
  const safeErrors = checkDockerfileInputs(dockerfile);
  assert.ok(!hasError(safeErrors, 'extracted before'));

  const checksum = String.raw`echo "$EXPECTED_SHA256  /tmp/cursor-agent.tar.gz" | sha256sum -c -; \
    tar --strip-components=1 -xzf /tmp/cursor-agent.tar.gz -C "$CURSOR_AGENT_DIR";`;
  const reversed = String.raw`tar --strip-components=1 -xzf /tmp/cursor-agent.tar.gz -C "$CURSOR_AGENT_DIR"; \
    echo "$EXPECTED_SHA256  /tmp/cursor-agent.tar.gz" | sha256sum -c -;`;
  const fixture = dockerfileLf.replace(checksum, reversed);
  assert.ok(hasError(checkDockerfileInputs(fixture), 'extracted before'));
});

test('uppercase, short, and empty Cursor hashes are rejected', () => {
  const uppercase = dockerfile.replace(
    'ARG CURSOR_AGENT_X64_SHA256=f4250e91feea78bb55e1c48b5fc830a57d3dd3681b04652f629726c5b9c8f24a',
    'ARG CURSOR_AGENT_X64_SHA256=F4250E91FEEA78BB55E1C48B5FC830A57D3DD3681B04652F629726C5B9C8F24A',
  );
  assert.ok(hasError(checkDockerfileInputs(uppercase), 'CURSOR_AGENT_X64_SHA256'));

  const short = dockerfile.replace(
    'ARG CURSOR_AGENT_ARM64_SHA256=b207208ee96c9230fc19baf1457f25206430ef3b63d750f690afc20885ba812f',
    'ARG CURSOR_AGENT_ARM64_SHA256=deadbeef',
  );
  assert.ok(hasError(checkDockerfileInputs(short), 'CURSOR_AGENT_ARM64_SHA256'));

  const empty = dockerfile.replace(
    'ARG CURSOR_AGENT_X64_SHA256=f4250e91feea78bb55e1c48b5fc830a57d3dd3681b04652f629726c5b9c8f24a',
    'ARG CURSOR_AGENT_X64_SHA256=',
  );
  assert.ok(hasError(checkDockerfileInputs(empty), 'CURSOR_AGENT_X64_SHA256'));
});

test('required dockerignore entries cannot be omitted or re-included', () => {
  const missing = dockerignore.replace('\n.npmrc\n', '\n');
  assert.ok(hasError(checkDockerignoreInputs(missing), 'missing required pattern: .npmrc'));
  const missingNested = dockerignore.replace('\n**/.claude\n', '\n');
  assert.ok(hasError(checkDockerignoreInputs(missingNested), 'missing required pattern: **/.claude'));

  for (const unsafeReinclude of ['!sample.pem', '!nested/secrets.json', '!nested/state.local', '!nested/file.tmp']) {
    assert.ok(
      hasError(checkDockerignoreInputs(`${dockerignore}\n${unsafeReinclude}\n`), 're-includes a secret'),
      `${unsafeReinclude} must not bypass the context boundary`,
    );
  }
});

test('comments do not activate forbidden Dockerfile rules', () => {
  const fixture = '# RUN curl https://example.invalid/install | bash\n# RUN npm install -g latest\n' + dockerfile;
  assert.deepEqual(checkDockerfileInputs(fixture), []);
});

test('CRLF has the same verdict as LF', () => {
  const lf = checkDockerInputs({ dockerfile, dockerignore, packageJson, packageLock });
  const crlf = checkDockerInputs({
    dockerfile: dockerfile.replaceAll('\n', '\r\n'),
    dockerignore: dockerignore.replaceAll('\n', '\r\n'),
    packageJson,
    packageLock,
  });
  assert.deepEqual(crlf, lf);
});

test('provider package and lock versions must stay exact', () => {
  const rangePackage = structuredClone(packageJson);
  rangePackage.dependencies['@openai/codex'] = '^0.137.0';
  assert.ok(hasError(checkProviderInputs(rangePackage, packageLock, dockerfile), 'exact version 0.137.0'));

  const driftedLock = structuredClone(packageLock);
  driftedLock.packages['node_modules/@github/copilot'].version = '1.0.58';
  assert.ok(hasError(checkProviderInputs(packageJson, driftedLock, dockerfile), 'lock entry must be 1.0.59'));

  const driftedRoot = structuredClone(packageLock);
  driftedRoot.packages[''].dependencies['@github/copilot'] = '^1.0.59';
  assert.ok(hasError(checkProviderInputs(packageJson, driftedRoot, dockerfile), 'lock root dependency must be 1.0.59'));
});
