// bugfix_spinner-cup-not-consumed-in-webui_2026-07-02.md の仮説確定実験。
// 実セッションの生 PTY ログを @xterm/headless に食わせ、
//   A) 無加工（ネイティブ端末相当）
//   B) cursor-hide-filter 通過後（Web UI 相当）
// の scrollback + 画面バッファに 'thinking with medium effort' が何回化石化するかを比較する。
// 使い方: node scripts/repro-spinner-fossil.mjs <生ログパス> [rows] [cols]
import { readFileSync } from 'node:fs';
import xterm from '@xterm/headless';
import { filterCursorHideBlocksPure } from '../dist/app/cursor-hide-filter.js';
import { filterHubMarkersPure, bytesStartWith } from '../dist/app/hub-marker-filter.js';

const { Terminal } = xterm;
const NEEDLE = 'thinking with medium effort';

const logPath = process.argv[2];
if (!logPath) {
  console.error('usage: node scripts/repro-spinner-fossil.mjs <raw.log> [rows] [cols]');
  process.exit(1);
}
const raw = new Uint8Array(readFileSync(logPath));
const rawText = new TextDecoder().decode(raw);
const rawCount = rawText.split(NEEDLE).length - 1;

// 生ログの CUP 最大行から当時の端末 rows を推定（引数で上書き可）
let maxCupRow = 0;
for (const m of rawText.matchAll(/\x1b\[(\d+);\d+H/g)) {
  maxCupRow = Math.max(maxCupRow, Number(m[1]));
}
const rows = Number(process.argv[3]) || Math.max(maxCupRow, 24);
const cols = Number(process.argv[4]) || 155;

function countInBuffer(term) {
  const buf = term.buffer.active;
  let text = '';
  for (let y = 0; y < buf.length; y++) {
    const line = buf.getLine(y);
    if (line) text += line.translateToString(true) + '\n';
  }
  return { count: text.split(NEEDLE).length - 1, lines: buf.length };
}

function writeAll(term, chunks) {
  return new Promise((resolve) => {
    let pending = chunks.length;
    if (pending === 0) return resolve();
    for (const c of chunks) {
      term.write(c, () => { if (--pending === 0) resolve(); });
    }
  });
}

function chunked(bytes, size) {
  const out = [];
  for (let i = 0; i < bytes.length; i += size) out.push(bytes.slice(i, i + size));
  return out;
}

const CHUNK = 1024; // WS フレーム相当の分割

// ── terminal.ts の非純関数フィルタの忠実な複製（DOM 依存部を除去） ──────────
const asciiBytes = (str) => Uint8Array.from(str, (c) => c.charCodeAt(0) & 0xff);

// filterReverseVideoForDisplay 相当
function makeReverseVideoFilter() {
  let carry = new Uint8Array(0);
  return (bytes) => {
    const combined = new Uint8Array(carry.length + bytes.length);
    combined.set(carry, 0);
    combined.set(bytes, carry.length);
    const out = [];
    let i = 0;
    while (i < combined.length) {
      if (combined[i] !== 0x1b) { out.push(combined[i]); i++; continue; }
      if (i + 1 >= combined.length) break;
      if (combined[i + 1] !== 0x5b) { out.push(combined[i]); i++; continue; }
      let j = i + 2;
      while (j < combined.length && !(combined[j] >= 0x40 && combined[j] <= 0x7e)) j++;
      if (j >= combined.length) break;
      if (combined[j] !== 0x6d) {
        for (let k = i; k <= j; k++) out.push(combined[k]);
        i = j + 1;
        continue;
      }
      const params = Array.from(combined.slice(i + 2, j), (b) => String.fromCharCode(b)).join('');
      const parts = params.split(';');
      const hasReverse = parts.includes('7');
      const hasReverseOff = parts.includes('27');
      const filtered = parts.filter((p) => p !== '7' && p !== '27');
      if (hasReverse) filtered.push('48', '5', '238');
      if (hasReverseOff) filtered.push('49');
      if (filtered.length > 0) {
        for (const b of asciiBytes(`\x1b[${filtered.join(';')}m`)) out.push(b);
      }
      i = j + 1;
    }
    carry = combined.slice(i);
    return new Uint8Array(out);
  };
}

// filterBareCarriageReturnForDisplay 相当
function makeBareCrFilter() {
  let carry = new Uint8Array(0);
  const EL = asciiBytes('\x1b[K');
  return (bytes) => {
    const combined = new Uint8Array(carry.length + bytes.length);
    combined.set(carry, 0);
    combined.set(bytes, carry.length);
    const out = [];
    for (let i = 0; i < combined.length; i++) {
      out.push(combined[i]);
      if (combined[i] === 0x0d) {
        if (i + 1 < combined.length) {
          if (combined[i + 1] !== 0x0a) for (const b of EL) out.push(b);
        } else {
          carry = combined.slice(i);
          return new Uint8Array(out.slice(0, out.length - 1));
        }
      }
    }
    carry = new Uint8Array(0);
    return new Uint8Array(out);
  };
}

// filterSynchronizedUpdateForDisplay 相当
function makeSyncUpdateFilter() {
  let carry = new Uint8Array(0);
  const patterns = [asciiBytes('\x1b[?2026h'), asciiBytes('\x1b[?2026l')];
  const carryLen = Math.max(...patterns.map((p) => p.length)) - 1;
  return (bytes) => {
    const combined = new Uint8Array(carry.length + bytes.length);
    combined.set(carry, 0);
    combined.set(bytes, carry.length);
    const out = [];
    let i = 0;
    const carryStartLimit = Math.max(0, combined.length - carryLen);
    while (i < combined.length) {
      const seq = patterns.find((p) => bytesStartWith(combined, i, p));
      if (seq) { i += seq.length; continue; }
      if (i >= carryStartLimit) {
        const maybePrefix = patterns.some((p) => {
          const remaining = combined.length - i;
          if (remaining >= p.length) return false;
          for (let j = 0; j < remaining; j++) if (combined[i + j] !== p[j]) return false;
          return true;
        });
        if (maybePrefix) break;
      }
      out.push(combined[i]);
      i++;
    }
    carry = combined.slice(i);
    return new Uint8Array(out);
  };
}

// filterHubMarkersForDisplay 相当（純関数 + state 橋渡し）
function makeHubMarkerFilter() {
  let state = { carry: new Uint8Array(0), inDone: false, inMarker: false, markerBuf: new Uint8Array(0), doneBuf: new Uint8Array(0) };
  return (bytes) => {
    const r = filterHubMarkersPure(bytes, state);
    state = r.state;
    return r.out;
  };
}

// filterCursorHideShowBlocksForDisplay 相当
function makeCursorHideFilter(eventTotals) {
  let state = { carry: new Uint8Array(0), inBlock: false, blockBuf: [], hasAbsPos: false, hasNewline: false, altScreen: false };
  return (bytes) => {
    const r = filterCursorHideBlocksPure(bytes, state);
    state = r.state;
    if (eventTotals) for (const ev of r.events) eventTotals[ev.kind] = (eventTotals[ev.kind] || 0) + 1;
    return r.out;
  };
}

// writePTYChunk と同じ合成順: sync(bareCr(cursorHide(reverseVideo(hubMarker(bytes)))))
// ※ erase-scrollback は provider=codex のみのため省略
function makeFullChain(eventTotals) {
  const hub = makeHubMarkerFilter();
  const rv = makeReverseVideoFilter();
  const ch = makeCursorHideFilter(eventTotals);
  const cr = makeBareCrFilter();
  const su = makeSyncUpdateFilter();
  return (bytes) => su(cr(ch(rv(hub(bytes)))));
}

// A) 無加工
const termA = new Terminal({ rows, cols, scrollback: 100000, allowProposedApi: true });
await writeAll(termA, chunked(raw, CHUNK));
const a = countInBuffer(termA);

// B) cursor-hide-filter 単独通過後
const termB = new Terminal({ rows, cols, scrollback: 100000, allowProposedApi: true });
const chOnly = makeCursorHideFilter(null);
const filteredChunks = [];
for (const c of chunked(raw, CHUNK)) {
  const out = chOnly(c);
  if (out.length > 0) filteredChunks.push(out);
}
await writeAll(termB, filteredChunks);
const b = countInBuffer(termB);

// C) 実機 writePTYChunk と同じフルチェーン + 各フィルタ単独での比較（犯人特定の二分探索）
async function runChain(label, makeFns) {
  const term = new Terminal({ rows, cols, scrollback: 100000, allowProposedApi: true });
  const fns = makeFns();
  const chainChunks = [];
  for (const c of chunked(raw, CHUNK)) {
    let out = c;
    for (const fn of fns) out = fn(out);
    if (out.length > 0) chainChunks.push(out);
  }
  await writeAll(term, chainChunks);
  return { label, ...countInBuffer(term) };
}

const results = [];
results.push(await runChain('full: hub>rv>ch>cr>su', () => [makeHubMarkerFilter(), makeReverseVideoFilter(), makeCursorHideFilter(null), makeBareCrFilter(), makeSyncUpdateFilter()]));
results.push(await runChain('hubMarker only', () => [makeHubMarkerFilter()]));
results.push(await runChain('reverseVideo only', () => [makeReverseVideoFilter()]));
results.push(await runChain('bareCr only', () => [makeBareCrFilter()]));
results.push(await runChain('syncUpdate only', () => [makeSyncUpdateFilter()]));
results.push(await runChain('rv>ch', () => [makeReverseVideoFilter(), makeCursorHideFilter(null)]));
results.push(await runChain('ch>cr', () => [makeCursorHideFilter(null), makeBareCrFilter()]));
results.push(await runChain('ch>su', () => [makeCursorHideFilter(null), makeSyncUpdateFilter()]));

console.log(JSON.stringify({
  logPath, rows, cols, rawBytes: raw.length,
  needle: NEEDLE,
  rawStreamOccurrences: rawCount,
  unfiltered: a,
  cursorHideOnly: b,
  chains: results,
}, null, 2))
process.exit(0);
