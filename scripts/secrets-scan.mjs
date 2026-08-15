#!/usr/bin/env node
// kb-driven secrets-scan — unified scanner for layer 2 (husky) / layer 3 (CI) / layer 4 (release) / sweep
//
// 設計詳細: docs/local/secrets-scan-design/index.html
// 原則: ~/.claude/guides/reference_release-pipeline.md P10
//
// kb の表示名列 + family display + 構造 regex（パス・private IP）で公開対象ファイルをスキャン。
// 新しい watchlist テーブルは作らない（id 正典・名前派生の kb 設計を維持）。

import { readFileSync, existsSync, statSync, readdirSync } from 'node:fs';
import { execSync, execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { homedir } from 'node:os';

// === Configuration ===
// KB_ROOT / FAMILY_ROOT are required env vars (no hardcoded default so this
// script can ship as portable OSS without exposing a personal directory).
// If unset, the kb/family watchlists are skipped and only structural regex
// runs. Setup (example, PowerShell):
//   $env:KB_ROOT     = 'C:/path/to/kb'
//   $env:FAMILY_ROOT = 'C:/path/to/family'
// bash/zsh:
//   export KB_ROOT=/path/to/kb
//   export FAMILY_ROOT=/path/to/family

const KB_ROOT = process.env.KB_ROOT || null;
const FAMILY_ROOT = process.env.FAMILY_ROOT || null;

// PERSONAL_DIRS: 実ファイル名を watchlist 化する個人ディレクトリ（`;` 区切り・`~` 可）。
// KB_ROOT と同じく未設定なら丸ごとスキップするので、OSS として配布しても個人パスは露出しない。
//   $env:PERSONAL_DIRS = '~/.claude;~/.many-ai-cli;~/.ssh'
//   export PERSONAL_DIRS='~/.claude:~/.many-ai-cli:~/.ssh' は不可（Windows のドライブレターと衝突するため `;` 固定）
//
// 狙いは kb watchlist が拾えない事故クラス: 「自分の環境を ls して、そこで見えた実ファイル名を
// そのままテスト fixture やドキュメントへ転記してしまう」。名前は事前に列挙できないので
// watchlist を手で書くことができず、構造 regex にも掛からない（パス接頭辞が無い裸のファイル名のため）。
const PERSONAL_DIRS = process.env.PERSONAL_DIRS || null;

// Paths exempted from scanning (substring match against the path returned by git).
// These files legitimately reference watchlist patterns by design.
const EXEMPT_PATHS = [
  'scripts/secrets-scan.mjs',
  'scripts/secrets-scan.',
  '.husky/pre-commit',
  '.github/workflows/secrets-scan',
  'docs/local/',
  // 秘密情報 denylist の定義そのもの。ブロックすべきファイル名を列挙するのが役割なので、
  // watchlist パターンと一致するのは設計どおり（scripts/secrets-scan.mjs 自身と同じ理由）。
  'internal/hub/files_scope.go',
];

// 2 文字の needle は部分一致では検出器として機能しない。2 文字の和語はごく普通の
// 文章の中に現れるし、2 文字の英字略称は lockfile のハッシュ等に当たり続ける。
// 実際の全 tracked sweep では 44/567 件がこの種の誤検出だった。誤検出が多いゲートは
// 「どうせ誤検出」と読み飛ばされて本物を通すので、検出できない長さは最初から
// needle にしない。短い名前は full_name 側（姓+名の連結）の needle で守る。
// 落とした件数は必ず表示する（黙って保護範囲を狭めない）。
const MIN_NEEDLE_LEN = 3;

// 短すぎて needle にできなかった件数。値そのものは個人情報なので絶対に出力しない。
const shortNeedleSkips = { count: 0 };
function countShortNeedle(value) {
  if (value && value.length > 0 && value.length < MIN_NEEDLE_LEN) shortNeedleSkips.count++;
}
const MAX_FILE_SIZE = 1024 * 1024;

// === Public-email allowlist ===
// メールアドレスは構造 regex で全件検知し、「公開する前提のアドレス」だけをここで通す。
// 非公開アドレスそのものは絶対にここへ書かない（allowlist 方式なら書かずに防げる）。
const ALLOWED_EMAILS = [
  'ishizakahiroshi.dev@gmail.com', // 公開コミット名義・package manifest 用
];
const ALLOWED_EMAIL_DOMAINS = [
  'users.noreply.github.com',  // GitHub の noreply
  'anthropic.com',             // AI コミット footer（Co-Authored-By）
  'example.com',               // ドキュメントの例示用
  'example.invalid',           // テスト fixture 用（RFC 2606 予約・名前解決されない）
  // ここに各プロジェクトの公開窓口ドメインを追記する（例: 'manabi-map.app'）
];

function isAllowedEmail(matched) {
  const email = matched.toLowerCase();
  if (ALLOWED_EMAILS.includes(email)) return true;
  const domain = email.split('@')[1] || '';
  return ALLOWED_EMAIL_DOMAINS.some(d => domain === d || domain.endsWith('.' + d));
}

const BINARY_EXTS = new Set([
  '.png', '.jpg', '.jpeg', '.gif', '.bmp', '.webp', '.ico',
  '.pdf', '.zip', '.tar', '.gz', '.bz2', '.xz', '.7z', '.rar',
  '.exe', '.dll', '.so', '.dylib', '.bin', '.o', '.obj',
  '.woff', '.woff2', '.ttf', '.otf', '.eot',
  '.mp3', '.mp4', '.wav', '.avi', '.mov', '.webm', '.m4a',
]);

// Always skip these regardless of mode (huge text files that aren't authored)
const SKIP_FILENAMES = new Set([
  'pnpm-lock.yaml', 'package-lock.json', 'yarn.lock', 'Cargo.lock',
  'go.sum', 'poetry.lock', 'Pipfile.lock',
]);

// === Minimal CSV parser (RFC 4180 subset with quoted fields) ===

function parseCSV(text) {
  const rows = [];
  let row = [];
  let field = '';
  let inQuote = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (inQuote) {
      if (c === '"') {
        if (text[i + 1] === '"') { field += '"'; i++; }
        else { inQuote = false; }
      } else {
        field += c;
      }
    } else {
      if (c === '"') inQuote = true;
      else if (c === ',') { row.push(field); field = ''; }
      else if (c === '\n') { row.push(field); rows.push(row); row = []; field = ''; }
      else if (c === '\r') { /* ignore */ }
      else { field += c; }
    }
  }
  if (field.length > 0 || row.length > 0) { row.push(field); rows.push(row); }
  return rows;
}

// === Watchlist loading ===

// Some kb names include a parenthetical category/disambiguator
// (e.g. "クロノス(勤怠)" / "Nextcloud(メイジエ)"). For matching purposes
// we want BOTH the full string AND the bare name before the paren,
// so a leak of just "クロノス" (without paren) is still caught.
function expandNameVariants(value) {
  const variants = new Set();
  countShortNeedle(value);
  if (value.length >= MIN_NEEDLE_LEN) variants.add(value);
  // Strip both half-width (...) and full-width （...） suffix
  const stripped = value.replace(/[(（].*$/, '').trim();
  if (stripped !== value && stripped.length >= MIN_NEEDLE_LEN) {
    variants.add(stripped);
  }
  return [...variants];
}

function loadKbWatchlist(kbRoot) {
  if (!kbRoot || !existsSync(kbRoot)) {
    return { available: false, items: [] };
  }
  const items = [];
  const specs = [
    { file: 'companies.csv',    col: 3, label: 'companies.short_name' },
    { file: 'people.csv',       col: 1, label: 'people.name' },
    { file: 'servers.csv',      col: 1, label: 'servers.host' },
    { file: 'applications.csv', col: 1, label: 'applications.name' },
  ];
  for (const { file, col, label } of specs) {
    const path = join(kbRoot, file);
    if (!existsSync(path)) continue;
    try {
      const rows = parseCSV(readFileSync(path, 'utf8'));
      for (let i = 1; i < rows.length; i++) {
        const value = (rows[i][col] || '').trim();
        for (const variant of expandNameVariants(value)) {
          items.push({ needle: variant, source: `kb/${file}:${label}` });
        }
      }
    } catch (e) {
      console.error(`WARN: failed to parse ${path}: ${e.message}`);
    }
  }
  return { available: true, items };
}

function loadFamilyWatchlist(familyRoot) {
  if (!familyRoot) {
    return { available: false, items: [] };
  }
  const path = join(familyRoot, 'people.csv');
  if (!existsSync(path)) {
    return { available: false, items: [] };
  }
  const items = [];
  try {
    const rows = parseCSV(readFileSync(path, 'utf8'));
    for (let i = 1; i < rows.length; i++) {
      const familyName = (rows[i][1] || '').trim();
      const givenName  = (rows[i][2] || '').trim();
      countShortNeedle(familyName);
      countShortNeedle(givenName);
      if (familyName.length >= MIN_NEEDLE_LEN) {
        items.push({ needle: familyName, source: 'family/people.csv:family_name' });
      }
      if (givenName.length >= MIN_NEEDLE_LEN) {
        items.push({ needle: givenName,  source: 'family/people.csv:given_name' });
      }
      if (familyName && givenName) {
        items.push({ needle: familyName + givenName, source: 'family/people.csv:full_name' });
      }
    }
  } catch (e) {
    console.error(`WARN: failed to parse ${path}: ${e.message}`);
  }
  return { available: true, items };
}

// === Personal directory filename watchlist ===

// 短い名前・ありふれた名前は誤検知の元なので長さで足切りする。
const PERSONAL_FILE_MIN_LEN = 10;
const PERSONAL_DIR_MAX_DEPTH = 2;
const PERSONAL_DIR_MAX_ENTRIES = 5000;

function expandHome(p) {
  if (p === '~') return homedir();
  if (p.startsWith('~/') || p.startsWith('~\\')) return join(homedir(), p.slice(2));
  return p;
}

// 本製品が自分で作る・連携先の公開仕様である名前は needle にしない。
// 根拠と追加条件は scripts/secrets-scan.owned-names.json の _comment を参照。
function loadOwnedNames() {
  const path = new URL('./secrets-scan.owned-names.json', import.meta.url);
  try {
    const raw = JSON.parse(readFileSync(path, 'utf8'));
    return new Set((raw.ownedNames || []).map(e => e.name));
  } catch (e) {
    // 読めなければ除外なしで続行する。ゲートは緩めるより厳しく倒す。
    console.error(`WARN: failed to read secrets-scan.owned-names.json: ${e.message}`);
    return new Set();
  }
}

function loadPersonalFileWatchlist(spec) {
  if (!spec) return { available: false, items: [] };
  const dirs = spec.split(';').map(s => s.trim()).filter(Boolean);
  if (dirs.length === 0) return { available: false, items: [] };

  const owned = loadOwnedNames();
  const items = [];
  let visited = 0;

  const walk = (dir, label, depth) => {
    if (depth > PERSONAL_DIR_MAX_DEPTH || visited >= PERSONAL_DIR_MAX_ENTRIES) return;
    let entries;
    try { entries = readdirSync(dir, { withFileTypes: true }); } catch { return; }
    for (const entry of entries) {
      if (visited >= PERSONAL_DIR_MAX_ENTRIES) return;
      visited++;
      if (entry.isDirectory()) {
        walk(join(dir, entry.name), label, depth + 1);
        continue;
      }
      if (entry.name.length < PERSONAL_FILE_MIN_LEN) continue;
      if (owned.has(entry.name)) continue;
      items.push({ needle: entry.name, source: `personal-dir/${label}` });
    }
  };

  for (const dir of dirs) {
    const resolved = expandHome(dir);
    if (!existsSync(resolved)) continue;
    walk(resolved, dir, 1);
  }
  return { available: true, items };
}

// === Baseline mode (staged only) ===
//
// 既存リポジトリへ後付けで導入すると、既に入っている値が大量にヒットして毎 commit が落ち、
// 数日でゲートごと無効化される。そこで --staged では「この commit が新たに持ち込んだ値」
// だけをブロックする（baseline 方式）。既存分は --all-tracked の全件 sweep で別途潰す。
//
// 判定は 2 段:
//   1. staged diff の「追加行」に載っているヒットだけを残す
//   2. 個人ディレクトリ由来の needle は、その文字列が既に HEAD にあるなら常に除外する
//      （例: config.yaml のようにプロジェクトが本来参照している名前。新規追加行に出ても正規）

const headNeedleCache = new Map();

function existsInHead(needle) {
  if (headNeedleCache.has(needle)) return headNeedleCache.get(needle);
  let found = false;
  try {
    execFileSync('git', ['grep', '-F', '-q', '-e', needle, 'HEAD'], { stdio: 'ignore' });
    found = true;
  } catch {
    found = false;
  }
  headNeedleCache.set(needle, found);
  return found;
}

// stagedAddedLines は staged diff でその file に追加された行番号の集合を返す。
// diff が取れない場合は null を返し、呼び出し側は絞り込まない（判定不能ならブロック側に倒す）。
function stagedAddedLines(file) {
  let out;
  try {
    out = execFileSync('git', ['diff', '--cached', '-U0', '--', file], { encoding: 'utf8' });
  } catch {
    return null;
  }
  const added = new Set();
  const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@/gm;
  let m;
  while ((m = hunk.exec(out)) !== null) {
    const start = Number(m[1]);
    const count = m[2] === undefined ? 1 : Number(m[2]);
    for (let i = 0; i < count; i++) added.add(start + i);
  }
  return added;
}

function filterToNewlyIntroduced(hits) {
  const addedCache = new Map();
  return hits.filter(hit => {
    if (String(hit.source).startsWith('personal-dir/') && existsInHead(hit.matched)) return false;
    if (!addedCache.has(hit.file)) addedCache.set(hit.file, stagedAddedLines(hit.file));
    const added = addedCache.get(hit.file);
    if (added === null) return true;
    return added.has(hit.lineNumber);
  });
}

// === Structural patterns (regex) ===

function getStructuralPatterns() {
  return [
    {
      // `Program Files` / `Windows` は共有システムパスで個人情報ではないため対象外。
      // `Users` / `dev` のみ personal-path として検知する。
      name: 'Windows absolute path',
      regex: /[A-Za-z]:[\\/](?:Users|dev)[\\/]/g,
      suggestion: '個人パスを削除またはプレースホルダ化 / Remove personal absolute path or use a placeholder',
    },
    {
      name: 'POSIX home path',
      regex: /\/(?:Users|home)\/[a-zA-Z0-9_.-]+\//g,
      suggestion: 'ホームパスを ~/ などにマスク / Mask home path with `~/`',
    },
    {
      // loopback (127.0.0.1) は共有定数で個人情報ではなく、技術文書での例示にも頻出するため対象外。
      // RFC1918 (10/8, 172.16/12, 192.168/16) のみを内部 LAN トポロジー漏洩として検知する。
      name: 'Private IPv4 (RFC1918)',
      regex: /\b(?:10\.\d{1,3}\.\d{1,3}\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3}|192\.168\.\d{1,3}\.\d{1,3})\b/g,
      suggestion: '内部 IP を一般化または削除 / Generalize or remove internal IP',
    },
    {
      // allowlist（ALLOWED_EMAILS / ALLOWED_EMAIL_DOMAINS）に無いメールアドレスは全件ブロック。
      // 個人アドレスを watchlist に書かずに検知するための allowlist 方式。
      name: 'Email address (not on public allowlist)',
      regex: /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9][A-Za-z0-9.-]*\.[A-Za-z]{2,}\b/g,
      allow: isAllowedEmail,
      suggestion: '非公開メールを削除 / 公開前提のアドレスなら ALLOWED_EMAILS(_DOMAINS) へ追加',
    },
  ];
}

// === File listing per mode ===

function getFilesByMode(mode) {
  try {
    switch (mode) {
      case 'staged':
        return execSync('git diff --cached --name-only --diff-filter=ACM', { encoding: 'utf8' })
          .trim().split('\n').filter(Boolean);
      case 'files-from-diff':
        return execSync('git diff --name-only --diff-filter=ACM HEAD', { encoding: 'utf8' })
          .trim().split('\n').filter(Boolean);
      case 'all-tracked':
        return execSync('git ls-files', { encoding: 'utf8' })
          .trim().split('\n').filter(Boolean);
      case 'packaged':
        console.error('ERROR: --packaged mode not yet implemented (TODO: read npm pack output for layer 4)');
        process.exit(2);
      default:
        console.error(`ERROR: unknown mode: ${mode}`);
        process.exit(2);
    }
  } catch (e) {
    if (String(e.message || '').includes('not a git repository')) {
      console.error('ERROR: not a git repository');
      process.exit(2);
    }
    throw e;
  }
}

// === Inline exempt directive ===
// Lines containing `secrets-scan: allow` are exempted.
//   `secrets-scan: allow`            -> allow all hits on this line
//   `secrets-scan: allow Nextcloud`  -> allow only matches whose needle
//                                       contains (case-insensitive) "Nextcloud"
// Multiple directives per line are OR'd.
function isAllowedByDirective(line, matchedNeedle) {
  const re = /secrets-scan:\s*allow(?:\s+([^\s\->]+))?/gi;
  let m;
  while ((m = re.exec(line)) !== null) {
    if (!m[1]) return true; // bare "allow" = whole-line exempt
    const target = m[1].toLowerCase();
    const needle = matchedNeedle.toLowerCase();
    if (needle.includes(target) || target.includes(needle)) return true;
  }
  return false;
}

// === Exemption / binary checks ===

function isExempt(path) {
  // normalize separator for substring matching
  const p = path.replace(/\\/g, '/');
  return EXEMPT_PATHS.some(ex => p.includes(ex));
}

function isBinary(path) {
  const lower = path.toLowerCase();
  for (const ext of BINARY_EXTS) {
    if (lower.endsWith(ext)) return true;
  }
  return false;
}

function isSkipFilename(path) {
  const base = path.replace(/\\/g, '/').split('/').pop();
  return SKIP_FILENAMES.has(base);
}

// === Scanning ===

function scanFile(path, needleMap, structuralPatterns) {
  if (!existsSync(path)) return [];
  let stat;
  try { stat = statSync(path); } catch { return []; }
  if (!stat.isFile()) return [];
  if (stat.size > MAX_FILE_SIZE) return [];

  let content;
  try { content = readFileSync(path, 'utf8'); } catch { return []; }

  const hits = [];
  const lines = content.split('\n');

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // watchlist needles (substring match)
    for (const [needle, source] of needleMap) {
      if (line.includes(needle) && !isAllowedByDirective(line, needle)) {
        hits.push({
          file: path,
          lineNumber: i + 1,
          matched: needle,
          source,
          kind: 'watchlist',
          suggestion: '一般化（kb 由来名称を抽象化）/ Generalize (mask kb-derived name)',
        });
      }
    }

    // structural patterns
    for (const { name, regex, suggestion, allow } of structuralPatterns) {
      regex.lastIndex = 0;
      let m;
      while ((m = regex.exec(line)) !== null) {
        if (!(allow && allow(m[0])) && !isAllowedByDirective(line, m[0])) {
          hits.push({
            file: path,
            lineNumber: i + 1,
            matched: m[0],
            source: `structural: ${name}`,
            kind: 'structural',
            suggestion,
          });
        }
        if (m.index === regex.lastIndex) regex.lastIndex++;
      }
    }
  }
  return hits;
}

// === Output ===

function formatHitsText(hits, mode) {
  const lines = [];
  lines.push('');
  lines.push('================================================================');
  lines.push(`BLOCKED: secrets-scan detected ${hits.length} match(es) in scanned files.`);
  lines.push(`ブロック: スキャン対象に ${hits.length} 件の混入を検知`);
  lines.push('================================================================');
  lines.push('');
  for (const h of hits) {
    lines.push(`  ${h.file}:${h.lineNumber}`);
    lines.push(`    matched : '${h.matched}'`);
    lines.push(`    source  : ${h.source}`);
    lines.push(`    suggest : ${h.suggestion}`);
    lines.push('');
  }
  if (mode === 'staged' || mode === 'files-from-diff') {
    lines.push('To bypass (NOT recommended): git commit --no-verify');
    lines.push('  Note: CI (layer 3) and release gate (layer 4) will run the same check.');
    lines.push('  注意: bypass しても CI（層 3）と release ゲート（層 4）で再 fail します。');
    lines.push('');
  }
  return lines.join('\n');
}

// === Argument parsing ===

function parseArgs(argv) {
  const args = { mode: null, block: false, dryRun: false, format: 'text', help: false };
  for (const a of argv) {
    if (a === '--staged') args.mode = 'staged';
    else if (a === '--files-from-diff') args.mode = 'files-from-diff';
    else if (a === '--all-tracked') args.mode = 'all-tracked';
    else if (a === '--packaged') args.mode = 'packaged';
    else if (a === '--block') args.block = true;
    else if (a === '--dry-run') args.dryRun = true;
    else if (a === '--format=json') args.format = 'json';
    else if (a === '--format=text') args.format = 'text';
    else if (a === '-h' || a === '--help') args.help = true;
  }
  return args;
}

function showHelp() {
  process.stdout.write(`secrets-scan — kb-driven content gate for public-facing files

Usage: node scripts/secrets-scan.mjs <mode> [options]

Modes (exactly one required):
  --staged              scan files staged for commit (layer 2 / pre-commit hook)
                        baseline mode: only hits on lines this commit ADDS are
                        reported, so an existing backlog does not block every
                        commit. Use --all-tracked to see the backlog.
  --files-from-diff     scan files changed since HEAD (layer 3 / CI on PR)
  --all-tracked         scan all git-tracked files (sweep / audit)
  --packaged            scan packaged tarball (layer 4 / release gate) [TODO]

Options:
  --block               exit 1 on any hit (use for enforcement)
  --dry-run             report hits but exit 0 (use for sweep / audit)
  --format=text|json    output format (default: text)
  -h, --help            show this help

Environment (required for full coverage; unset = structural regex only):
  KB_ROOT       path to kb root containing companies.csv / people.csv / servers.csv / applications.csv
  FAMILY_ROOT   path to family CSV root containing people.csv
  PERSONAL_DIRS semicolon-separated personal directories whose real filenames become
                needles (catches "copied a filename seen while inspecting my own machine")
  Example (PowerShell): $env:KB_ROOT = 'C:/path/to/kb'
  Example (bash/zsh)  : export KB_ROOT=/path/to/kb
  Example (PowerShell): $env:PERSONAL_DIRS = '~/.claude;~/.many-ai-cli;~/.ssh'

Exit codes:
  0  no hits, or hits found but --dry-run / no --block
  1  hits found with --block
  2  configuration / usage error

Watchlist sources:
  kb/companies.csv (short_name) / people.csv (name) /
  servers.csv (host) / applications.csv (name)
  family/people.csv (family_name, given_name, family+given)
  PERSONAL_DIRS real filenames (>= 10 chars, depth <= 2; in --staged mode names
  already present in HEAD are exempt so only newly introduced ones block)
  + structural regex (Windows absolute paths, POSIX home paths, RFC1918 IPs)
`);
}

// === Main ===

function main() {
  const args = parseArgs(process.argv.slice(2));

  if (args.help) { showHelp(); process.exit(0); }
  if (!args.mode) { showHelp(); process.exit(2); }

  const kb = loadKbWatchlist(KB_ROOT);
  const family = loadFamilyWatchlist(FAMILY_ROOT);
  const personal = loadPersonalFileWatchlist(PERSONAL_DIRS);

  const warnings = [];
  if (!kb.available) {
    if (!KB_ROOT) {
      warnings.push(`WARN: KB_ROOT env var not set — kb-derived watchlist skipped, structural regex only`);
      warnings.push(`WARN: KB_ROOT env var が未設定 — kb 由来 watchlist をスキップ・構造 regex のみで継続`);
    } else {
      warnings.push(`WARN: KB_ROOT path not found: ${KB_ROOT} — kb-derived watchlist skipped`);
      warnings.push(`WARN: KB_ROOT パスが見つかりません: ${KB_ROOT} — kb 由来 watchlist をスキップ`);
    }
  }
  if (!family.available) {
    if (!FAMILY_ROOT) {
      warnings.push(`WARN: FAMILY_ROOT env var not set — family watchlist skipped`);
      warnings.push(`WARN: FAMILY_ROOT env var が未設定 — family watchlist をスキップ`);
    } else {
      warnings.push(`WARN: FAMILY_ROOT path not found: ${FAMILY_ROOT} — family watchlist skipped`);
      warnings.push(`WARN: FAMILY_ROOT パスが見つかりません: ${FAMILY_ROOT} — family watchlist をスキップ`);
    }
  }

  if (!personal.available) {
    warnings.push(`WARN: PERSONAL_DIRS env var not set — personal filename watchlist skipped`);
    warnings.push(`WARN: PERSONAL_DIRS env var が未設定 — 個人ディレクトリのファイル名 watchlist をスキップ`);
  }

  // 保護範囲を黙って狭めない。落とした件数だけ出す（値は個人情報なので出さない）。
  if (shortNeedleSkips.count > 0) {
    warnings.push(`WARN: ${shortNeedleSkips.count} watchlist name(s) shorter than ${MIN_NEEDLE_LEN} chars were skipped — substring matching cannot detect them reliably`);
    warnings.push(`WARN: ${MIN_NEEDLE_LEN} 文字未満の watchlist 名 ${shortNeedleSkips.count} 件をスキップ — 部分一致では検出できないため。別手段で守る必要がある`);
  }

  // De-duplicate needles (a name can appear in multiple kb tables).
  // personal は最後に足す。kb / family と重複した場合は出所の分かる kb / family 側を残す。
  const needleMap = new Map();
  for (const item of [...kb.items, ...family.items, ...personal.items]) {
    if (!needleMap.has(item.needle)) needleMap.set(item.needle, item.source);
  }

  const structuralPatterns = getStructuralPatterns();

  const allFiles = getFilesByMode(args.mode);
  const filesToScan = allFiles.filter(f => !isExempt(f) && !isBinary(f) && !isSkipFilename(f));

  const rawHits = [];
  for (const file of filesToScan) {
    const hits = scanFile(file, needleMap, structuralPatterns);
    rawHits.push(...hits);
  }

  // staged モードは baseline 方式（この commit が新たに持ち込んだ値だけをブロック）。
  // 既存分は --all-tracked の全件 sweep 側で洗い出す。詳細は filterToNewlyIntroduced のコメント。
  const allHits = args.mode === 'staged' ? filterToNewlyIntroduced(rawHits) : rawHits;

  for (const w of warnings) console.error(w);

  if (args.format === 'json') {
    process.stdout.write(JSON.stringify({
      mode: args.mode,
      scanned: filesToScan.length,
      total_files: allFiles.length,
      exempt_or_skipped: allFiles.length - filesToScan.length,
      kb_needles: kb.items.length,
      family_needles: family.items.length,
      personal_needles: personal.items.length,
      structural_patterns: structuralPatterns.length,
      hits: allHits,
      warnings,
    }, null, 2) + '\n');
  } else {
    if (allHits.length === 0) {
      process.stdout.write(`OK: secrets-scan passed (scanned ${filesToScan.length} files; ${needleMap.size} needles + ${structuralPatterns.length} structural patterns)\n`);
      process.stdout.write(`OK: secrets-scan に問題なし\n`);
    } else {
      process.stderr.write(formatHitsText(allHits, args.mode));
    }
  }

  if (allHits.length > 0 && args.block && !args.dryRun) {
    process.exit(1);
  }
  process.exit(0);
}

main();
