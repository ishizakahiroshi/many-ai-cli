#!/usr/bin/env node
// 全画面オーバーレイの wheel 除外クラス（.aac-wheel-overlay）の付け忘れを止める。
//
// 背景:
//   web/src/app/terminal.ts の document レベル wheel リスナーは capture / passive:false で、
//   除外されなかったホイールを全部背後の xterm へ転送して preventDefault する。
//   除外に載っていないオーバーレイは「自分はスクロールせず、背後の端末が動く」という
//   同じ壊れ方をする。2026-08 までに #workflow-modal・.bug-report-overlay・#usage-dropdown
//   で 3 回踏んだ。原因は毎回「新しいオーバーレイを除外へ登録し忘れた」で同じ。
//
//   登録先を id の allowlist から共通クラス .aac-wheel-overlay へ移した（2026-08-27・
//   docs/local/plan_overlay-wheel-scroll-exclusion-audit.md 案B）が、クラスを付けるのも
//   人の手順である以上、忘れる余地は残る。忘れた瞬間に落ちるのがこの検査。
//
// 検査内容:
//   web/src/**.css から「position: fixed かつ画面全体を覆う」ルールを機械抽出し、
//   そのセレクタに対応する生成箇所（index.html の id/class、または .ts の id 代入 /
//   className 代入 / テンプレート文字列）で .aac-wheel-overlay が付いているかを
//   突き合わせる。付いていなければブロックする。
//
//   意図的に対象外にするものは EXEMPT に「理由つきで」書く。理由を書かせるのが目的。
//
// この検査が見ないもの（既知の盲点。ここに載っていないものを「検査済み」と読まないこと）:
//   1. inset の 4 辺が 0 でないマスク（例 `inset: env(safe-area-inset-top) 0 0` の
//      #mobile-drawer-backdrop）。実質全画面でも拾わない。誤検出を増やさないための割り切り。
//   2. 画面全体を覆わないポップオーバー（Usage ドロップダウン等）。こちらは data-wheel-native
//      の担当で、付け忘れても「そのポップオーバーだけ効かない」で済むため検査していない。
//   3. 複合セレクタ（`body.x #y` 等）で定義されるオーバーレイ。生成箇所を特定できない。
//
// exit 0 = 問題なし / exit 1 = ブロック。

import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const webSrc = join(repoRoot, 'web', 'src');
const CLASS = 'aac-wheel-overlay';
// Windows のパス区切り。表示用に / へ揃える。
const WIN_SEP = String.fromCharCode(92);

// 対象外にするセレクタと、その理由。理由が空のものはエラーにする。
const EXEMPT = {
  '#mobile-approval-sheet-backdrop':
    'モバイル承認シートの背面マスク。position:fixed の指定が @media (max-width:720px) の内側にしか無いため、' +
    'クラスを付けるとデスクトップ幅で要素が残っていたときに「表示中」と誤判定され画面全体のホイールが死ぬ。' +
    'モバイル専用導線でホイール操作が想定されないため対象外にしている（タッチ対応デスクトップでの挙動は未確認）。',
};

// --- CSS 側: 画面全体を覆う fixed 要素のセレクタを集める -------------------

function cssFiles() {
  const out = [join(webSrc, 'styles.css')];
  const dir = join(webSrc, 'styles');
  for (const name of readdirSync(dir)) {
    if (name.endsWith('.css')) out.push(join(dir, name));
  }
  return out.filter((p) => existsSync(p));
}

// ブロック単位に切り出す。@media 等のネストは stack で追う。
function ruleBlocks(css) {
  const src = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const blocks = [];
  const stack = [];
  let token = '';
  for (const ch of src) {
    if (ch === '{') {
      stack.push(token.trim());
      token = '';
    } else if (ch === '}') {
      const selector = stack.pop();
      if (selector !== undefined) {
        blocks.push({ selector: selector.replace(/\s+/g, ' ').trim(), body: token });
      }
      token = '';
    } else {
      token += ch;
    }
  }
  return blocks;
}

const SIDE_ZERO = ['top', 'right', 'bottom', 'left'].map(
  (side) => new RegExp(String.raw`(^|[;{\s])` + side + String.raw`\s*:\s*0(\s|;|$)`),
);

function coversViewport(body) {
  if (!/position\s*:\s*fixed/.test(body)) return false;
  if (/inset\s*:\s*0(\s|;|$)/.test(body)) return true;
  return SIDE_ZERO.every((re) => re.test(body));
}

// 単一の id / class セレクタだけを対象にする。複合セレクタ（body.x #y 等）は
// 生成箇所の特定が曖昧になるので拾わない。
function simpleTarget(selector) {
  const m = /^([#.])([A-Za-z0-9_-]+)$/.exec(selector);
  if (!m) return null;
  return { kind: m[1] === '#' ? 'id' : 'class', name: m[2], selector };
}

// --- 生成箇所側: そのセレクタに .aac-wheel-overlay が付いているか -----------

function sourceFiles() {
  const out = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (/\.(ts|html)$/.test(entry.name)) out.push(p);
    }
  };
  walk(webSrc);
  return out;
}

// 「同じ生成箇所」の判定窓。id 代入とクラス付与が隣接行に並ぶ書き方を許す。
const NEAR_LINES = 6;

function siteMatchers(target) {
  const name = target.name;
  if (target.kind === 'id') {
    // index.html の id="foo" / .ts の el.id = 'foo' / attrs: { id: 'foo' }
    return [
      new RegExp(String.raw`id\s*=\s*["']` + name + String.raw`["']`),
      new RegExp(String.raw`id\s*:\s*["']` + name + String.raw`["']`),
    ];
  }
  // class="... foo ..." / className = '... foo ...' / class: '... foo ...'
  return [
    new RegExp(String.raw`(class|className)\s*[:=]\s*["'` + '`' + String.raw`][^"'` + '`' + String.raw`]*\b` + name + String.raw`\b`),
  ];
}

function hasRegistration(target, files) {
  const matchers = siteMatchers(target);
  let siteFound = false;
  for (const file of files) {
    const lines = readFileSync(file, 'utf8').split('\n');
    for (let i = 0; i < lines.length; i++) {
      if (!matchers.some((re) => re.test(lines[i]))) continue;
      siteFound = true;
      const from = Math.max(0, i - NEAR_LINES);
      const to = Math.min(lines.length, i + NEAR_LINES + 1);
      if (lines.slice(from, to).some((l) => l.includes(CLASS))) return { registered: true };
    }
  }
  return { registered: false, siteFound };
}

// --- 実行 -------------------------------------------------------------------

const targets = new Map();
for (const file of cssFiles()) {
  for (const { selector, body } of ruleBlocks(readFileSync(file, 'utf8'))) {
    if (!coversViewport(body)) continue;
    const t = simpleTarget(selector);
    if (!t) continue;
    if (!targets.has(t.selector)) targets.set(t.selector, { ...t, file });
  }
}

const errors = [];

for (const [selector, reason] of Object.entries(EXEMPT)) {
  if (!reason || !reason.trim()) {
    errors.push('EXEMPT の ' + selector + ' に理由が書かれていない。対象外にする根拠を書くこと。');
  }
  if (!targets.has(selector)) {
    errors.push(
      'EXEMPT に ' + selector + ' があるが、画面全体を覆う fixed ルールが CSS に見つからない。' +
        '要素を消したか CSS を変えたなら EXEMPT からも消すこと（古い免除が残ると次の穴を隠す）。',
    );
  }
}

const files = sourceFiles();
const unregistered = [];
for (const target of targets.values()) {
  if (Object.hasOwn(EXEMPT, target.selector)) continue;
  const r = hasRegistration(target, files);
  if (!r.registered) unregistered.push({ target, siteFound: r.siteFound });
}

if (unregistered.length > 0) {
  errors.push('画面全体を覆う fixed オーバーレイに .' + CLASS + ' が付いていない（' + unregistered.length + ' 件）:');
  for (const { target, siteFound } of unregistered) {
    const rel = target.file.slice(repoRoot.length + 1).split(WIN_SEP).join('/');
    errors.push(
      '  ' + target.selector + '  (' + rel + ')' +
        (siteFound ? '' : '  ※生成箇所を web/src 配下で特定できなかった。動的生成なら書き方を既存に揃えること。'),
    );
  }
  errors.push('');
  errors.push('  直し方: その要素の生成箇所へ ' + CLASS + ' を足す。');
  errors.push('    index.html なら class="... aac-wheel-overlay"、.ts なら');
  errors.push("    el.className = '<既存> aac-wheel-overlay' か el.classList.add('aac-wheel-overlay')。");
  errors.push('  端末に重なるだけで画面全体は覆わないポップオーバーは、このクラスではなく');
  errors.push('  data-wheel-native 属性を使う（画面全体のホイールを止めると端末が操作できなくなる）。');
  errors.push('  対象外にする判断なら scripts/check-wheel-overlays.mjs の EXEMPT へ理由つきで足す。');
}

if (errors.length > 0) {
  console.error('check-wheel-overlays: NG');
  for (const line of errors) console.error(line);
  process.exit(1);
}

console.log('check-wheel-overlays: OK（全画面オーバーレイ ' + targets.size + ' 件 / 免除 ' + Object.keys(EXEMPT).length + ' 件）');
