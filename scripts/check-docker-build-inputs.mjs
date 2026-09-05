#!/usr/bin/env node

import { existsSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const dockerfilePath = join(repoRoot, 'deploy', 'docker', 'Dockerfile');
const dockerignorePath = join(repoRoot, 'deploy', 'docker', 'Dockerfile.dockerignore');
const providerPackagePath = join(repoRoot, 'deploy', 'docker', 'provider-cli', 'package.json');
const providerLockPath = join(repoRoot, 'deploy', 'docker', 'provider-cli', 'package-lock.json');

export const EXPECTED_PROVIDER_VERSIONS = Object.freeze({
  '@anthropic-ai/claude-code': '2.1.162',
  '@openai/codex': '0.137.0',
  '@github/copilot': '1.0.59',
});

export const REQUIRED_DOCKERIGNORE_PATTERNS = Object.freeze([
  '.env',
  '.env.*',
  '*.env',
  '**/.env',
  '**/.env.*',
  '**/*.env',
  'secrets.*',
  '*.secret.*',
  '*credential*',
  'serverpass*',
  '*.pem',
  '*.key',
  '*.p12',
  '*.pfx',
  '*.ppk',
  '**/secrets.*',
  '**/*.secret.*',
  '**/*credential*',
  '**/serverpass*',
  '**/*.pem',
  '**/*.key',
  '**/*.p12',
  '**/*.pfx',
  '**/*.ppk',
  'id_rsa',
  'id_dsa',
  'id_ecdsa',
  'id_ed25519',
  '**/id_rsa',
  '**/id_dsa',
  '**/id_ecdsa',
  '**/id_ed25519',
  '.netrc',
  '.npmrc',
  '.pypirc',
  '**/.netrc',
  '**/.npmrc',
  '**/.pypirc',
  '.ssh',
  '**/.ssh',
  '.aws',
  '**/.aws',
  '.kube',
  '**/.kube',
  '.docker',
  '**/.docker',
  '.claude',
  '**/.claude',
  '.cursor',
  '**/.cursor',
  '.grok',
  '**/.grok',
  '.many-ai-cli',
  '**/.many-ai-cli',
  '.git-worktrees',
  '**/.git-worktrees',
  '.docsweep',
  '**/.docsweep',
  '.codex-tmp',
  '**/.codex-tmp',
  '*.local',
  '*.local.*',
  '*.log',
  '*.jsonl',
  '*.tmp',
  '*.bak',
  '*.orig',
  '*.rej',
  '**/*.local',
  '**/*.local.*',
  '**/*.log',
  '**/*.jsonl',
  '**/*.tmp',
  '**/*.bak',
  '**/*.orig',
  '**/*.rej',
]);

const SHA256_RE = /^sha256:[0-9a-f]{64}$/;
const SHA256_HEX_RE = /^[0-9a-f]{64}$/;
const SHA1_HEX_RE = /^[0-9a-f]{40}$/;
const EXACT_VERSION_RE = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

function normalizeNewlines(value) {
  return String(value).replaceAll('\r\n', '\n').replaceAll('\r', '\n');
}

// Join Dockerfile continuation lines before checking command order or pipes.
export function logicalDockerLines(source) {
  const lines = normalizeNewlines(source).split('\n');
  const logical = [];
  let current = '';
  for (const line of lines) {
    const rightTrimmed = line.replace(/\s+$/, '');
    const part = rightTrimmed.trimStart();
    current = current ? `${current}${part}` : part;
    if (rightTrimmed.endsWith('\\')) {
      current = current.slice(0, -1).trimEnd() + ' ';
      continue;
    }
    if (current.trim() !== '') logical.push(current.trim());
    current = '';
  }
  if (current.trim() !== '') logical.push(current.trim());
  return logical;
}

function activeDockerLines(source) {
  return logicalDockerLines(source).filter((line) => !line.startsWith('#'));
}

function parseFrom(line) {
  const match = /^FROM\s+(?:(?:--platform=\S+)\s+)?(\S+)(?:\s+AS\s+([A-Za-z0-9_.-]+))?$/i.exec(line);
  return match ? { image: match[1], stage: match[2] || null } : null;
}

function parseArg(lines, name) {
  const prefix = `ARG ${name}=`;
  const line = lines.find((candidate) => candidate.startsWith(prefix));
  return line ? line.slice(prefix.length).trim() : null;
}

function checkFromPins(lines, errors) {
  const stages = new Set();
  for (const line of lines) {
    if (!/^FROM\b/i.test(line)) continue;
    const parsed = parseFrom(line);
    if (!parsed) {
      errors.push(`unparseable FROM instruction: ${line}`);
      continue;
    }
    const imageName = parsed.image.toLowerCase();
    if (!stages.has(imageName) && imageName !== 'scratch') {
      const at = parsed.image.lastIndexOf('@');
      const reference = at >= 0 ? parsed.image.slice(0, at) : parsed.image;
      const digest = at >= 0 ? parsed.image.slice(at + 1) : '';
      const lastPart = reference.slice(reference.lastIndexOf('/') + 1);
      if (!lastPart.includes(':')) errors.push(`FROM image has no tag: ${parsed.image}`);
      if (!SHA256_RE.test(digest)) errors.push(`FROM image digest is not a lowercase sha256 pin: ${parsed.image}`);
    }
    if (parsed.stage) stages.add(parsed.stage.toLowerCase());
  }
}

function checkCopyFromPins(lines, errors) {
  const stages = new Set();
  for (const line of lines) {
    const from = parseFrom(line);
    if (from?.stage) stages.add(from.stage.toLowerCase());
  }
  for (const line of lines) {
    const match = /^COPY\s+--from=(\S+)\s+/i.exec(line);
    if (!match) continue;
    const source = match[1];
    if (/^\d+$/.test(source) || stages.has(source.toLowerCase())) continue;
    const at = source.lastIndexOf('@');
    const reference = at >= 0 ? source.slice(0, at) : source;
    const digest = at >= 0 ? source.slice(at + 1) : '';
    const lastPart = reference.slice(reference.lastIndexOf('/') + 1);
    if (!lastPart.includes(':')) errors.push(`COPY external image has no tag: ${source}`);
    if (!SHA256_RE.test(digest)) errors.push(`COPY external image digest is not a lowercase sha256 pin: ${source}`);
  }
}

function checkDangerousInstallPaths(lines, errors) {
  for (const line of lines) {
    if (/(?:curl|wget)\b[\s\S]*\|\s*(?:bash|sh|tar)\b/i.test(line)) {
      errors.push(`unverified download piped to an interpreter or archive tool: ${line}`);
    }
    if (/\bnpm\s+(?:install|i)\b[^\n]*\s(?:-g|--global)(?:\s|$)/i.test(line)) {
      errors.push(`global npm install is forbidden: ${line}`);
    }
    if (/\blatest\b/i.test(line)) errors.push(`mutable latest reference is forbidden: ${line}`);
  }
}

function checkWhisperPin(lines, errors) {
  const tag = parseArg(lines, 'WHISPER_CPP_TAG');
  if (tag !== 'v1.8.6') errors.push(`WHISPER_CPP_TAG must remain v1.8.6, got ${tag || '(missing)'}`);
  const commit = parseArg(lines, 'WHISPER_CPP_COMMIT');
  if (!commit || !SHA1_HEX_RE.test(commit)) {
    errors.push('WHISPER_CPP_COMMIT must be a 40-character lowercase commit SHA');
  }
  const joined = lines.join('\n');
  if (!/checkout\s+--detach[^\n]*WHISPER_CPP_COMMIT/i.test(joined)) {
    errors.push('Whisper source is not checked out at WHISPER_CPP_COMMIT with --detach');
  }
  if (!/rev-parse\s+HEAD\)[^\n]*WHISPER_CPP_COMMIT/i.test(joined)) {
    errors.push('Whisper HEAD is not compared with WHISPER_CPP_COMMIT');
  }
}

function checkCursorPins(lines, errors) {
  const version = parseArg(lines, 'CURSOR_AGENT_VERSION');
  if (!version || version !== '2026.06.03-0bbb28e') {
    errors.push(`CURSOR_AGENT_VERSION must remain 2026.06.03-0bbb28e, got ${version || '(missing)'}`);
  }
  for (const name of ['CURSOR_AGENT_X64_SHA256', 'CURSOR_AGENT_ARM64_SHA256']) {
    const value = parseArg(lines, name);
    if (!value || !SHA256_HEX_RE.test(value)) errors.push(`${name} must be a 64-character lowercase SHA-256 value`);
  }
  const joined = lines.join('\n');
  const checksum = joined.indexOf('sha256sum -c');
  const extract = joined.search(/(?:^|[;&])\s*tar\s+[^\n]*\s-x[^\n]*\s/);
  if (checksum < 0) errors.push('Cursor archive has no sha256sum -c verification');
  if (extract < 0) errors.push('Cursor archive extraction command is missing');
  if (checksum >= 0 && extract >= 0 && checksum > extract) {
    errors.push('Cursor archive is extracted before its checksum is verified');
  }
  if (!/CURSOR_AGENT_X64_SHA256/.test(joined) || !/CURSOR_AGENT_ARM64_SHA256/.test(joined)) {
    errors.push('Cursor supported architectures do not select their pinned hashes');
  }
}

export function checkDockerfileInputs(source) {
  const errors = [];
  const lines = activeDockerLines(source);
  checkFromPins(lines, errors);
  checkCopyFromPins(lines, errors);
  checkDangerousInstallPaths(lines, errors);
  checkWhisperPin(lines, errors);
  checkCursorPins(lines, errors);

  const providerArgs = {
    PROVIDER_CLAUDE_CODE_VERSION: EXPECTED_PROVIDER_VERSIONS['@anthropic-ai/claude-code'],
    PROVIDER_CODEX_VERSION: EXPECTED_PROVIDER_VERSIONS['@openai/codex'],
    PROVIDER_COPILOT_VERSION: EXPECTED_PROVIDER_VERSIONS['@github/copilot'],
  };
  for (const [name, expected] of Object.entries(providerArgs)) {
    const actual = parseArg(lines, name);
    if (actual !== expected) errors.push(`${name} does not match the expected provider version ${expected}`);
  }
  return errors;
}

function normalizeDockerignoreLines(source) {
  return normalizeNewlines(source)
    .split('\n')
    .map((line) => line.trim().replaceAll('\\', '/'))
    .filter((line) => line !== '' && !line.startsWith('#'));
}

export function checkDockerignoreInputs(source) {
  const errors = [];
  const patterns = new Set(normalizeDockerignoreLines(source));
  for (const required of REQUIRED_DOCKERIGNORE_PATTERNS) {
    if (!patterns.has(required)) errors.push(`Dockerfile.dockerignore is missing required pattern: ${required}`);
  }
  if (patterns.has('*.txt')) errors.push('Dockerfile.dockerignore must not exclude *.txt globally');
  const dangerousReincludes = /(?:secret|credential|serverpass|\.env|\.pem|\.key|\.p12|\.pfx|\.ppk|id_rsa|id_dsa|id_ecdsa|id_ed25519|\.netrc|\.npmrc|\.pypirc|\.ssh|\.aws|\.kube|\.docker|\.claude|\.cursor|\.grok|\.many-ai-cli|\.git-worktrees|\.docsweep|\.codex-tmp|\.local|\.log|\.jsonl|\.tmp|\.bak|\.orig|\.rej)/i;
  for (const pattern of patterns) {
    if (pattern.startsWith('!') && dangerousReincludes.test(pattern.slice(1))) {
      errors.push(`Dockerfile.dockerignore re-includes a secret or local-state path: ${pattern}`);
    }
  }
  return errors;
}

export function checkProviderInputs(packageJson, packageLock, dockerfileSource) {
  const errors = [];
  if (packageJson?.private !== true) errors.push('provider-cli/package.json must set private: true');
  const dependencies = packageJson?.dependencies;
  const lockedPackages = packageLock?.packages;
  if (!dependencies || !lockedPackages || packageLock.lockfileVersion !== 3) {
    errors.push('provider-cli must contain an npm lockfileVersion 3 with packages');
    return errors;
  }
  for (const [name, expected] of Object.entries(EXPECTED_PROVIDER_VERSIONS)) {
    const declared = dependencies[name];
    const lockedDeclared = lockedPackages['']?.dependencies?.[name];
    if (declared !== expected || !EXACT_VERSION_RE.test(declared || '')) {
      errors.push(`${name} must use exact version ${expected} in package.json (got ${declared || '(missing)'})`);
    }
    if (lockedDeclared !== expected) {
      errors.push(`${name} lock root dependency must be ${expected} (got ${lockedDeclared || '(missing)'})`);
    }
    const locked = lockedPackages[`node_modules/${name}`]?.version;
    if (locked !== expected) errors.push(`${name} lock entry must be ${expected} (got ${locked || '(missing)'})`);
  }
  const dockerLines = activeDockerLines(dockerfileSource);
  const dockerArgs = {
    PROVIDER_CLAUDE_CODE_VERSION: '@anthropic-ai/claude-code',
    PROVIDER_CODEX_VERSION: '@openai/codex',
    PROVIDER_COPILOT_VERSION: '@github/copilot',
  };
  for (const [arg, packageName] of Object.entries(dockerArgs)) {
    const declared = parseArg(dockerLines, arg);
    if (declared !== EXPECTED_PROVIDER_VERSIONS[packageName]) {
      errors.push(`${arg} must match ${packageName} exact version ${EXPECTED_PROVIDER_VERSIONS[packageName]}`);
    }
  }
  return errors;
}

export function checkDockerInputs({ dockerfile, dockerignore, packageJson, packageLock }) {
  return [
    ...checkDockerfileInputs(dockerfile),
    ...checkDockerignoreInputs(dockerignore),
    ...checkProviderInputs(packageJson, packageLock, dockerfile),
  ];
}

function readJSON(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

export function main() {
  const requiredFiles = [dockerfilePath, dockerignorePath, providerPackagePath, providerLockPath];
  const missing = requiredFiles.filter((path) => !existsSync(path));
  if (missing.length > 0) {
    console.error(`BLOCKED: required Docker input file is missing: ${missing.join(', ')}`);
    return 1;
  }
  const errors = checkDockerInputs({
    dockerfile: readFileSync(dockerfilePath, 'utf8'),
    dockerignore: readFileSync(dockerignorePath, 'utf8'),
    packageJson: readJSON(providerPackagePath),
    packageLock: readJSON(providerLockPath),
  });
  if (errors.length > 0) {
    console.error(`BLOCKED: Docker build input checks found ${errors.length} problem(s)`);
    for (const error of errors) console.error(`  - ${error}`);
    return 1;
  }
  console.log('OK: Docker build inputs are pinned, verified, and secret-safe');
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main();
}
