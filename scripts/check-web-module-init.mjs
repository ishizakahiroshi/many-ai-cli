#!/usr/bin/env node
// 画面の JS（web/src）を読み込んだ瞬間に、モジュールの評価が例外で止まらないかを検査する。
//
// 背景:
//   web/src は循環 import を含む（state.ts → session-list.ts → … → approval.ts など）。循環の中の
//   モジュールは、import した相手の本体より先に評価されることがあり、その時点で相手の let / const を
//   同期で読むと TDZ の ReferenceError（Cannot access 'x' before initialization）が出る。出ると
//   モジュールグラフ全体の評価が止まり、画面は「読み込み中...」のまま何も反応しない。
//   同じ型を 3 回踏んだ（files-view の sessionsRef、2c25878 の app.ts、d6ef87b の approval.ts）。
//   直し方は毎回「読み込みの瞬間の呼び出しを queueMicrotask へ遅らせる」で同じなのに、
//   tsc も build も通るので、画面を開くまで気づけない。
//
// 検査内容:
//   1. web/src を esbuild で変換し、OS の一時ディレクトリへ書き出す。変換の条件は
//      web/scripts/build.mjs に合わせる（あちらを変えたらここも合わせる）。web/dist には書かない。
//      一時ディレクトリは終わったら消す
//   2. 子の node で、ブラウザの大域（document・navigator など）を「何を呼んでも何も起きない」stub に
//      して app-entry.js を import し、モジュールグラフの評価が最後まで届くかを見る。途中で投げたら
//      落とす。評価の途中で積まれたマイクロタスクが TDZ を投げても落とす
//   3. 画面側の受け手（web/src/app/boot-guard.ts）の配線。app-entry.ts の最初の import が
//      boot-guard で、最後の文が markAppEntryEvaluated() か。前者が崩れると評価中の例外を
//      受け取れず、後者が崩れると初期化の完了が分からなくなる
//
// この検査が見ないもの（既知の盲点。ここに載っていないものを「検査済み」と読まないこと）:
//   1. イベント（i18n-ready・DOMContentLoaded・クリック）とタイマーの中身。stub は登録を吸うだけで呼ばない
//   2. ブラウザでは要素が無いときにだけ通る経路。stub は常に「要素がある」側へ分岐させる
//   3. fetch の後の処理。fetch は解決しない Promise を返す
//   4. 構文エラーと import 先の欠け（tsc と build が捕まえる）
//   5. try / catch の中で握りつぶされた TDZ（評価は止まらないので、画面も止まらない）
//
// stub に無いブラウザの大域を、モジュールの読み込みの瞬間に使うと「X is not defined」で落ちる。
// ブラウザに実在する名前なら、下の BROWSER_GLOBALS に足す。
//
// 使い方: node scripts/check-web-module-init.mjs [--src <web/src と同じ形のディレクトリ>]
//   --src は陽性対照用（過去の版の web/src に掛けて、落ちることを確かめる）
// 必要なもの: web/node_modules（web で bun install 済みであること）
//
// exit 0 = 問題なし / exit 1 = ブロック / exit 2 = 検査を実行できなかった

import { cpSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const webDir = join(repoRoot, 'web');

const GUARD_IMPORT = './app/boot-guard.js';
const GUARD_DONE_CALL = 'markAppEntryEvaluated();';

// ブラウザにあって node に無い（または本物だと困る）大域。どれも吸い込みの stub にする。
// 2026-09-25 の web/src で読み込みの瞬間に実際に要ったのは addEventListener・removeEventListener・
// matchMedia・MutationObserver・ResizeObserver・HTMLButtonElement の 6 つ。残りは先回りして入れている
// （stub は何もしないので、多く入れても検査は緩まない。WebSocket などは本物だと接続しに行くので止めておく）。
// window / document / navigator / location / localStorage / fetch / タイマーは runner の中で個別に作る。
const BROWSER_GLOBALS = [
  'addEventListener', 'removeEventListener', 'dispatchEvent',
  'matchMedia', 'getComputedStyle', 'getSelection', 'scrollTo', 'open', 'alert', 'confirm', 'prompt',
  'history', 'screen', 'visualViewport', 'customElements', 'caches', 'indexedDB',
  'WebSocket', 'EventSource', 'BroadcastChannel', 'Notification', 'Audio', 'AudioContext', 'Image',
  'MutationObserver', 'ResizeObserver', 'IntersectionObserver', 'DOMParser', 'FileReader',
  'HTMLElement', 'HTMLButtonElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'HTMLDivElement',
  'HTMLCanvasElement', 'HTMLImageElement', 'Element', 'Node', 'CSS', 'KeyboardEvent', 'MouseEvent',
  'Terminal', 'FitAddon', 'Unicode11Addon', 'WebLinksAddon', 'WebglAddon', 'marked', 'DOMPurify', 'hljs', 'QRCode',
];

// 検査そのものを実行できなかった（依存が無い・子の node が結果を返さない等）。exit 2 で終わる。
class SetupError extends Error {}

function exitSetup(message) {
  throw new SetupError(message);
}

function parseArgs(argv) {
  let src = join(webDir, 'src');
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--src' && argv[i + 1]) {
      src = resolve(argv[++i]);
      continue;
    }
    exitSetup(`引数を解釈できない: ${argv[i]}`);
  }
  return { src };
}

// web/scripts/build.mjs の walk と同じ: vendor を除く .ts / .js（.d.ts を除く）。
function walk(dir, srcDir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (relative(srcDir, full).split(sep).includes('vendor')) continue;
    if (entry.isDirectory()) {
      out.push(...walk(full, srcDir));
      continue;
    }
    if (entry.isFile() && /\.(?:js|ts)$/.test(entry.name) && !entry.name.endsWith('.d.ts')) out.push(full);
  }
  return out;
}

async function transformInto(srcDir, outDir) {
  const requireFromWeb = createRequire(join(webDir, 'package.json'));
  let esbuild;
  try {
    esbuild = requireFromWeb('esbuild');
  } catch {
    exitSetup('esbuild が見つからない。web ディレクトリで bun install --frozen-lockfile を実行してから掛け直す');
  }
  // build.mjs と同じく、既定のビルドでは web/src/debug/ の sink を出さず、空の debug/index.js を置く。
  const debugDir = join(srcDir, 'debug');
  const debugKeep = join(debugDir, 'probe.ts');
  const entryPoints = walk(srcDir, srcDir).filter((f) => !f.startsWith(debugDir + sep) || f === debugKeep);
  await esbuild.build({
    entryPoints,
    outdir: outDir,
    outbase: srcDir,
    bundle: false,
    format: 'esm',
    target: 'es2022',
    sourcemap: 'inline',
    sourcesContent: false,
    logLevel: 'silent',
    define: { __MAI_DEBUG__: 'false' },
  });
  mkdirSync(join(outDir, 'debug'), { recursive: true });
  writeFileSync(join(outDir, 'debug', 'index.js'), '', 'utf8');
  // app が import する vendor の ESM（vtype-core など）。build.mjs は vendor を丸ごと写すので、ここでは .js だけ写す。
  cpSync(join(srcDir, 'vendor'), join(outDir, 'vendor'), {
    recursive: true,
    filter: (from) => statSync(from).isDirectory() || from.endsWith('.js'),
  });
  writeFileSync(join(outDir, 'package.json'), JSON.stringify({ type: 'module' }), 'utf8');
  return entryPoints.length;
}

// 子の node で動かす本体。ブラウザの大域を stub にしてから app-entry.js を import する。
// 文字列の中ではバッククォートとドル記号の波括弧を使わない（外側のテンプレート文字列を壊すため）。
const RUNNER_SOURCE = String.raw`
const entryUrl = process.argv[2];
const browserGlobals = JSON.parse(process.argv[3]);
const noop = () => {};

function makeStub(name) {
  const fn = function () {};
  const cache = new Map();
  return new Proxy(fn, {
    get(target, key) {
      if (key === Symbol.toPrimitive) return (hint) => (hint === 'number' ? 0 : '');
      if (key === Symbol.iterator) return function* () {};
      if (typeof key === 'symbol') return undefined;
      if (key === 'length') return 0;
      if (key === 'toString' || key === 'valueOf') return () => '';
      if (cache.has(key)) return cache.get(key);
      const child = makeStub(name + '.' + String(key));
      cache.set(key, child);
      return child;
    },
    set(target, key, value) { cache.set(key, value); return true; },
    deleteProperty(target, key) { cache.delete(key); return true; },
    apply() { return makeStub(name + '()'); },
    construct() { return makeStub('new ' + name); },
    has() { return true; },
  });
}

function withStubFallback(values, name) {
  const stub = makeStub(name);
  return new Proxy(values, {
    get(target, key) { return key in target ? target[key] : stub[key]; },
    has() { return true; },
  });
}

class MemStorage {
  constructor() { this.map = new Map(); }
  get length() { return this.map.size; }
  key(i) { return Array.from(this.map.keys())[i] ?? null; }
  getItem(k) { return this.map.has(String(k)) ? this.map.get(String(k)) : null; }
  setItem(k, v) { this.map.set(String(k), String(v)); }
  removeItem(k) { this.map.delete(String(k)); }
  clear() { this.map.clear(); }
}

const define = (key, value) => Object.defineProperty(globalThis, key, { value, configurable: true, writable: true });
const url = new URL('http://127.0.0.1:47777/?token=module-init-check');

define('window', globalThis);
define('self', globalThis);
define('document', makeStub('document'));
define('navigator', withStubFallback({ language: 'ja', languages: ['ja'], userAgent: 'Mozilla/5.0 (module-init-check)', platform: 'Win32', maxTouchPoints: 0, onLine: true }, 'navigator'));
define('location', withStubFallback({ href: url.href, origin: url.origin, protocol: url.protocol, host: url.host, hostname: url.hostname, port: url.port, pathname: url.pathname, search: url.search, hash: url.hash, reload: noop, replace: noop, assign: noop, toString: () => url.href }, 'location'));
define('localStorage', new MemStorage());
define('sessionStorage', new MemStorage());
define('fetch', () => new Promise(noop));
for (const key of ['setTimeout', 'setInterval', 'requestAnimationFrame', 'requestIdleCallback']) define(key, () => 0);
for (const key of ['clearTimeout', 'clearInterval', 'cancelAnimationFrame', 'cancelIdleCallback']) define(key, noop);
for (const key of browserGlobals) define(key, makeStub(key));

const info = (e) => ({
  name: e && e.name ? String(e.name) : typeof e,
  message: e && e.message !== undefined ? String(e.message) : String(e),
  stack: e && e.stack ? String(e.stack).split('\n').slice(0, 10).join('\n') : '',
});
const asyncErrors = [];
process.on('uncaughtException', (e) => asyncErrors.push(Object.assign({ kind: 'uncaughtException' }, info(e))));
process.on('unhandledRejection', (e) => asyncErrors.push(Object.assign({ kind: 'unhandledRejection' }, info(e))));

const saved = {};
for (const k of ['log', 'info', 'warn', 'error', 'debug']) { saved[k] = console[k]; console[k] = noop; }
let evalError = null;
try {
  await import(entryUrl);
} catch (e) {
  evalError = info(e);
}
for (let i = 0; i < 5; i++) await new Promise((r) => setImmediate(r));
for (const k of Object.keys(saved)) console[k] = saved[k];
process.stdout.write('@@RESULT@@' + JSON.stringify({ evalError, asyncErrors }) + '\n');
process.exit(0);
`;

function runGraph(outDir) {
  const runnerPath = join(outDir, '__module-init-runner.mjs');
  writeFileSync(runnerPath, RUNNER_SOURCE, 'utf8');
  const entryUrl = pathToFileURL(join(outDir, 'app-entry.js')).href;
  const r = spawnSync(process.execPath, ['--enable-source-maps', runnerPath, entryUrl, JSON.stringify(BROWSER_GLOBALS)], {
    encoding: 'utf8',
    timeout: 120_000,
    maxBuffer: 16 * 1024 * 1024,
    env: { ...process.env, NODE_OPTIONS: '' },
  });
  if (r.error) return { runnerError: `子の node が終わらなかった（${r.error.code ?? r.error.message}）。stub の下で無限ループになっている可能性がある` };
  const line = (r.stdout || '').split('\n').find((l) => l.startsWith('@@RESULT@@'));
  if (!line) return { runnerError: `子の node が結果を返さなかった（exit ${r.status}）\n${(r.stderr || '').slice(0, 2000)}` };
  return JSON.parse(line.slice('@@RESULT@@'.length));
}

const isTDZ = (e) => e && e.name === 'ReferenceError' && /before initialization/.test(e.message);
const isMissingGlobal = (e) => e && e.name === 'ReferenceError' && / is not defined$/.test(e.message);

function checkWiring(srcDir) {
  const problems = [];
  const entryPath = join(srcDir, 'app-entry.ts');
  let text;
  try {
    text = readFileSync(entryPath, 'utf8');
  } catch {
    return [`${entryPath} が読めない`];
  }
  const first = text.match(/^\s*import\b[^'"]*['"]([^'"]+)['"]/m);
  if (!first || first[1] !== GUARD_IMPORT) {
    problems.push(`app-entry.ts の最初の import が ${GUARD_IMPORT} ではない（${first ? first[1] : 'import が無い'}）。` +
      '最初に評価されないと、先に評価されたモジュールの例外を受け取れない');
  }
  const statements = text.split(/\r?\n/).map((l) => l.trim()).filter((l) => l && !l.startsWith('//'));
  const last = statements[statements.length - 1] ?? '';
  if (last !== GUARD_DONE_CALL && last !== GUARD_DONE_CALL.slice(0, -1)) {
    problems.push(`app-entry.ts の最後の文が ${GUARD_DONE_CALL} ではない（${last.slice(0, 80) || '空'}）。` +
      'これより後の文の例外は「読み込めなかった」の表示に乗らない');
  }
  return problems;
}

let src;
try {
  ({ src } = parseArgs(process.argv.slice(2)));
} catch (e) {
  console.error(`check-web-module-init: ${e.message}`);
  process.exit(2);
}
const outDir = mkdtempSync(join(tmpdir(), 'mai-module-init-'));
let failures = 0;
let setupError = null;
try {
  const moduleCount = await transformInto(src, outDir);
  const result = runGraph(outDir);
  if (result.runnerError) exitSetup(result.runnerError);

  const { evalError, asyncErrors } = result;
  if (evalError) {
    failures++;
    if (isTDZ(evalError)) {
      console.error('NG: モジュールの評価が TDZ で止まった（画面は「読み込み中...」のまま止まる）');
      console.error('    循環 import の中のモジュールが、読み込みの瞬間に同期で、まだ評価されていないモジュールの let / const を読んでいる。');
      console.error('    その呼び出しを queueMicrotask へ遅らせる（d6ef87b の approval.ts・2c25878 の app.ts と同じ直し方）。');
    } else if (isMissingGlobal(evalError)) {
      console.error('NG: モジュールの評価で、検査の stub に無い大域を使った。');
      console.error('    ブラウザに実在する名前なら、scripts/check-web-module-init.mjs の BROWSER_GLOBALS に足す。');
    } else {
      console.error('NG: モジュールの評価が例外で止まった。');
      console.error('    検査の stub の上でだけ起きる例外なら（ブラウザでは起きないなら）、stub を直す。ここで止めるのは、止まった先のモジュールを検査できないため。');
    }
    console.error(`    ${evalError.name}: ${evalError.message}`);
    console.error(evalError.stack.split('\n').slice(1).map((l) => `    ${l.trim()}`).join('\n'));
  }
  const asyncTDZ = asyncErrors.filter(isTDZ);
  for (const e of asyncTDZ) {
    failures++;
    console.error(`NG: 評価の途中で積まれた処理（${e.kind}）が TDZ を投げた: ${e.message}`);
    console.error(e.stack.split('\n').slice(1).map((l) => `    ${l.trim()}`).join('\n'));
  }
  const others = asyncErrors.filter((e) => !isTDZ(e));
  if (others.length > 0 && failures === 0) {
    // stub の上で後から走った処理の例外。ブラウザでも起きるとは限らないので止めない。
    console.log(`(参考) 評価の後に走った処理が stub の上で ${others.length} 件の例外を出した（TDZ ではないので止めない）: ${others[0].name}: ${others[0].message}`);
  }

  for (const p of checkWiring(src)) {
    failures++;
    console.error(`NG: ${p}`);
  }

  if (failures === 0) {
    console.log(`OK: web/src の ${moduleCount} ファイルを変換し、app-entry.js からのモジュールグラフの評価が最後まで届いた（TDZ なし）。boot-guard の配線もある`);
  }
} catch (e) {
  if (!(e instanceof SetupError)) throw e;
  setupError = e.message;
} finally {
  rmSync(outDir, { recursive: true, force: true });
}
if (setupError) {
  console.error(`check-web-module-init: ${setupError}`);
  process.exit(2);
}
process.exit(failures === 0 ? 0 : 1);
