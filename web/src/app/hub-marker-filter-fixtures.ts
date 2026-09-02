import assert from 'node:assert/strict';
import test from 'node:test';
import {
  filterHubMarkersPure,
  matchMarkerAllowingWrap,
  MAX_MARKER_BUFFER_BYTES,
  MAX_MARKER_WRAP_GAP_BYTES,
  type HubMarkerFilterState,
} from './hub-marker-filter.js';

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8');

function bytes(s: string): Uint8Array { return encoder.encode(s); }
function str(b: Uint8Array): string { return decoder.decode(b); }
function initialState(): HubMarkerFilterState {
  return {
    carry: new Uint8Array(0),
    inDone: false,
    inMarker: false,
    markerSeen: 0,
    doneSeen: 0,
  };
}

test('filterHubMarkersPure: 1 チャンクで完結する [MANY-AI-CLI] ブロックはタグだけ剥がして本文は残す', () => {
  const input = bytes('preamble\n[MANY-AI-CLI]Q1 question?\n1. opt1\n2. opt2\n[/MANY-AI-CLI]tail');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), 'preamble\nQ1 question?\n1. opt1\n2. opt2\ntail');
  assert.equal(state.inMarker, false);
  assert.equal(state.inDone, false);
  assert.equal(state.carry.length, 0);
  assert.equal(state.markerSeen, 0);
});

test('filterHubMarkersPure: 開きマーカーで終わるチャンクでも本文はその場で素通しする', () => {
  const part1 = bytes('preamble\n[MANY-AI-CLI]body1\nbody2');
  const { out: out1, state: state1 } = filterHubMarkersPure(part1, initialState());
  // 案 H: 貯めないので OPEN 以降の本文もこのチャンクで出る
  assert.equal(str(out1), 'preamble\nbody1\nbody2');
  assert.equal(state1.inMarker, true);
  assert.equal(state1.inDone, false);
  assert.equal(state1.carry.length, 0);
  assert.ok((state1.markerSeen ?? 0) > 0);

  const part2 = bytes('body3\n[/MANY-AI-CLI]tail');
  const { out: out2, state: state2 } = filterHubMarkersPure(part2, state1);
  assert.equal(str(out2), 'body3\ntail');
  assert.equal(state2.inMarker, false);
  assert.equal(state2.markerSeen, 0);
});

test('filterHubMarkersPure: チャンク跨ぎの ANSI シーケンスもそのまま連結して出る', () => {
  // ESC [ がチャンク1末尾、続きがチャンク2 という分割
  const part1 = bytes('[MANY-AI-CLI]hello\x1b[2');
  const { out: out1, state: state1 } = filterHubMarkersPure(part1, initialState());
  assert.equal(str(out1), 'hello\x1b[2');

  const part2 = bytes(';1Hworld[/MANY-AI-CLI]end');
  const { out: out2 } = filterHubMarkersPure(part2, state1);
  assert.equal(str(out2), ';1Hworldend');
});

test('filterHubMarkersPure: 閉じマーカーの途中で chunk が割れても carry で次に繋ぐ', () => {
  const open = bytes('pre\n[MANY-AI-CLI]body[/MANY-AI-CLI');
  const { out: out1, state: state1 } = filterHubMarkersPure(open, initialState());
  // 'pre\nbody' が出力され、CLOSE の prefix だけが carry に持ち越される
  assert.equal(str(out1), 'pre\nbody');
  assert.equal(state1.inMarker, true);
  assert.ok(state1.carry.length > 0);

  const rest = bytes(']post');
  const { out: out2, state: state2 } = filterHubMarkersPure(rest, state1);
  assert.equal(str(out2), 'post');
  assert.equal(state2.inMarker, false);
  assert.equal(state2.carry.length, 0);
  assert.equal(state2.markerSeen, 0);
});

test('filterHubMarkersPure (案 J): [MANY-AI-CLI-DONE] ブロックはタグだけ剥がして本文は残す', () => {
  const input = bytes('before\n[MANY-AI-CLI-DONE]\x1b[32mタスク完了\x1b[mしました[/MANY-AI-CLI-DONE]after');
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  // 案 J: 承認ブロックと同じ扱い。SGR も含めて本文はそのまま、剥がすのはタグ文字列だけ。
  // 本文は CLI 自身が描いたものなので、こちらは何も足さない・何も削らない。
  assert.equal(text, 'before\n\x1b[32mタスク完了\x1b[mしましたafter');
  assert.equal(text.includes('[MANY-AI-CLI-DONE]'), false);
  assert.equal(text.includes('[/MANY-AI-CLI-DONE]'), false);
  assert.equal(state.inDone, false);
});

test('filterHubMarkersPure (案 H/J): 承認ブロックの本文も落とさない', () => {
  const input = bytes('before\n[MANY-AI-CLI]\x1b[32m質問\x1b[mです[/MANY-AI-CLI]after');
  const { out } = filterHubMarkersPure(input, initialState());
  // 案 H: SGR も含めて本文はそのまま。剥がすのはタグ文字列だけ。
  assert.equal(str(out), 'before\n\x1b[32m質問\x1b[mですafter');
});

test('filterHubMarkersPure: マーカー無しの通常バイトは素通し（ANSI も含む）', () => {
  const input = bytes('hello \x1b[1mworld\x1b[m');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), 'hello \x1b[1mworld\x1b[m');
  assert.equal(state.inMarker, false);
  assert.equal(state.inDone, false);
});

test('filterHubMarkersPure: 行頭の stray な閉じマーカーは隠す（開きなしでも例外を投げない）', () => {
  const input = bytes('text\n[/MANY-AI-CLI]more');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), 'text\nmore');
  assert.equal(state.inMarker, false);
});

test('filterHubMarkersPure 行頭ゲート: 文中の stray な閉じマーカーはリテラルのまま素通し', () => {
  const input = bytes('text [/MANY-AI-CLI]more');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), 'text [/MANY-AI-CLI]more');
  assert.equal(state.inMarker, false);
});

test('filterHubMarkersPure: 連続する 2 ブロックで本文が両方とも残る', () => {
  const input = bytes('A\n[MANY-AI-CLI]X[/MANY-AI-CLI]B\n[MANY-AI-CLI]Y[/MANY-AI-CLI]C');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), 'A\nXB\nYC');
  assert.equal(state.inMarker, false);
});

test('filterHubMarkersPure: 開きマーカー文字列の途中で chunk が割れても carry で次に繋ぐ', () => {
  const part1 = bytes('pre\n[MANY-AI-CL');
  const { out: out1, state: state1 } = filterHubMarkersPure(part1, initialState());
  assert.equal(str(out1), 'pre\n');
  assert.equal(state1.inMarker, false);
  assert.ok(state1.carry.length > 0);

  const part2 = bytes('I]body[/MANY-AI-CLI]post');
  const { out: out2, state: state2 } = filterHubMarkersPure(part2, state1);
  assert.equal(str(out2), 'bodypost');
  assert.equal(state2.inMarker, false);
});

test('filterHubMarkersPure セーフガード: DONE close が typo で来なくても閾値超過でラッチだけ解除', () => {
  // 2026-07-01 セッション #16 実測の実例: [/MANARY-AI-CLI-DONE] とタイポし close 不一致
  const summary = 'サマリー本文';
  const filler = 'x'.repeat(MAX_MARKER_BUFFER_BYTES + 100);
  const input = bytes(`[MANY-AI-CLI-DONE]${summary} ${filler}[/MANARY-AI-CLI-DONE]after`);
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  // 案 J: 本文は貯めずに素通し済みなので、出力からは 1 バイトも欠けない
  assert.equal(text.includes(summary), true);
  assert.equal(text.includes('xxx'), true);
  // ラッチだけ解除する。放置すると以後の stray close を位置に関係なく受理してしまうため。
  assert.equal(state.inDone, false);
  assert.equal(state.doneSeen, 0);
  assert.equal(text.includes('after'), true);
});

test('filterHubMarkersPure セーフガード: 承認ブロックの close が来なくても閾値超過でラッチだけ解除', () => {
  const body = 'Q1 質問? 1. 選択肢';
  const filler = 'y'.repeat(MAX_MARKER_BUFFER_BYTES + 100);
  // close が来ないまま追加のバイト列（Claude Code TUI が絶え間なく吐く spinner 相当）
  const input = bytes(`[MANY-AI-CLI]${body} ${filler}`);
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  // 案 H: 本文は貯めずに素通し済みなので、捨てるものが無い（出力からは欠けない）
  assert.equal(text.includes(body), true);
  assert.equal(text.includes('yyy'), true);
  // ラッチだけ解除する。放置すると以後の stray close を位置に関係なく受理してしまうため。
  assert.equal(state.inMarker, false);
  assert.equal(state.markerSeen, 0);
});

// ── 行頭ゲート（2026-07-04・bugfix_spinner-cup-not-consumed-in-webui_2026-07-02.md）──
// AI が地の文でマーカーをリテラル引用した場合に inMarker へ誤ラッチし、後続の
// 画面再描画 32KB を飲み込んで剥離ゴミを一括ダンプしていた事故の再発防止。

test('行頭ゲート: 文中の prose リテラル OPEN はラッチせず素通し', () => {
  // 2026-07-04 orchestration 指揮者セッションの実測パターン
  const input = bytes('以後の確認・選択はすべて\x1b[1C[MANY-AI-CLI]\x1b[25;3Hマーカー形式で出力します。');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), '以後の確認・選択はすべて\x1b[1C[MANY-AI-CLI]\x1b[25;3Hマーカー形式で出力します。');
  assert.equal(state.inMarker, false);
});

test('行頭ゲート: CUP（絶対カーソル移動）＋インデント空白の直後の OPEN はラッチする', () => {
  // 正規マーカーの実測パターン: \x1b[30;1H + 2 スペース + OPEN
  const input = bytes('前置き\x1b[30;1H  [MANY-AI-CLI]Q1 質問?\n1. 選択肢[/MANY-AI-CLI]tail');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), '前置き\x1b[30;1H  Q1 質問?\n1. 選択肢tail');
  assert.equal(state.inMarker, false);
});

test('行頭ゲート: CR LF ＋空白の直後の OPEN はラッチする', () => {
  const input = bytes('前置き\r\n  \x1b[K[MANY-AI-CLI]body[/MANY-AI-CLI]tail');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), '前置き\r\n  \x1b[Kbodytail');
  assert.equal(state.inMarker, false);
});

test('行頭ゲート: 文中の prose リテラル DONE もラッチせず素通し', () => {
  const input = bytes('完了時は [MANY-AI-CLI-DONE] マーカーで報告します');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), '完了時は [MANY-AI-CLI-DONE] マーカーで報告します');
  assert.equal(state.inDone, false);
});

test('行頭ゲート: 承認ブロック本文の行頭に DONE リテラルが来ても DONE としてラッチしない', () => {
  // 案 H で本文も行頭追跡を通るようになったため、ブロック内でも lineStart が立つ。
  // 承認ブロック内の DONE リテラルを飲み込まないことを明示で押さえる。
  const input = bytes('\n[MANY-AI-CLI]Q1?\n[MANY-AI-CLI-DONE] と書く場合の説明\n[/MANY-AI-CLI]tail');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), '\nQ1?\n[MANY-AI-CLI-DONE] と書く場合の説明\ntail');
  assert.equal(state.inDone, false);
  assert.equal(state.inMarker, false);
});

test('行頭ゲート: 行頭状態はチャンクを跨いで保持される', () => {
  // チャンク 1 が文中で終わり、チャンク 2 の先頭に OPEN が来ても文中扱い
  const { out: out1, state: state1 } = filterHubMarkersPure(bytes('文中テキスト'), initialState());
  assert.equal(str(out1), '文中テキスト');
  const { out: out2, state: state2 } = filterHubMarkersPure(bytes('[MANY-AI-CLI]続き'), state1);
  assert.equal(str(out2), '[MANY-AI-CLI]続き');
  assert.equal(state2.inMarker, false);
  // 改行を挟めば次チャンク先頭の OPEN はラッチする
  const { state: state3 } = filterHubMarkersPure(bytes('\r\n'), state2);
  const { out: out4, state: state4 } = filterHubMarkersPure(bytes('[MANY-AI-CLI]body[/MANY-AI-CLI]'), state3);
  assert.equal(str(out4), 'body');
  assert.equal(state4.inMarker, false);
});

// ── 案 H（2026-08-26）: 承認ブロックへは何も足さない ──
// 案 E〜F は本文を 200 桁の VT グリッドへ描き直して現在カーソル位置へ書き戻していた。
// Claude Code は代替画面バッファで全セルを Ink が絶対座標で管理しているため、その書き戻しは
// Ink の管理外セルへ乗って以後消えず、200 桁ぶんの行末パディングが実画面（128 桁）で
// 折り返して「行頭に長い空白を伴う断片」を残していた（hub-marker-filter.ts 冒頭の実測表）。
// 以下は「本文へ 1 バイトも足さない・並べ替えない」ことを固定する。

test('案 H: ブロック内の絶対カーソル位置指定はそのまま通す（畳み込まない）', () => {
  const block = '\x1b[24;3HQ1 質問?\x1b[25;3H1. opt1\x1b[26;3H2. opt2';
  const input = bytes(`[MANY-AI-CLI]${block}[/MANY-AI-CLI]`);
  const { out } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), block);
});

test('案 H: 同じ行への上書き描画（スピナー等の再描画）もそのまま通す', () => {
  const block = '\x1b[1;1Hloading...\x1b[1;1H\x1b[Kdone!';
  const input = bytes(`[MANY-AI-CLI]${block}[/MANY-AI-CLI]`);
  const { out } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), block);
});

test('案 H: 複数世代の再描画が混ざっていてもそのまま通す', () => {
  const block = '\x1b[30;1Hstale first draft\x1b[2;1Hfinal answer';
  const input = bytes(`[MANY-AI-CLI]${block}[/MANY-AI-CLI]`);
  const { out } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), block);
});

test('案 H: 出力はタグのバイト数ぶんだけ短くなる（本文へ何も足さない）', () => {
  const block = '\x1b[24;3HQ1 日本語の質問?\x1b[25;3H1. 選択肢 (Recommended)\x1b[26;3HN. User specifies';
  const input = bytes(`[MANY-AI-CLI]${block}[/MANY-AI-CLI]`);
  const { out } = filterHubMarkersPure(input, initialState());
  const tagBytes = '[MANY-AI-CLI]'.length + '[/MANY-AI-CLI]'.length;
  assert.equal(out.length, input.length - tagBytes);
});

// ── 案 I（2026-09-02）: 折り返しで分断されたマーカー ──
// CLI は端末幅で長い行を折り返すので、マーカー文字列の途中に CR / LF・行末パディングの空白・
// カーソル移動の CSI が割り込む。連続バイト列の完全一致では CLOSE を取りこぼし、ラッチした
// まま以降の端末出力を全部捨てていた（セッション #3 で 31.2% の出力が xterm へ届いていない）。

test('案 I/J: 折り返しで分断された DONE の CLOSE を受理し、タグだけ剥がす', () => {
  // 2026-09-02 セッション #3（Codex 130 桁）の実測パターン
  const wrapped = '[/MANY-AI-CLI-\r\n' + ' '.repeat(130) + '\r\n\x1b[?25l\x1b[35;1H\n\x1b[30;1H  DONE]';
  const input = bytes(`\x1b[30;1H  [MANY-AI-CLI-DONE] 対象 plan は未完了です。 ${wrapped}\x1b[30;1H■ You've hit your usage limit.`);
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  // 案 J: 本文も CLOSE 以降の CLI 出力（使用量上限のエラー行）も届き、タグだけ消える
  assert.equal(text.includes('対象 plan は未完了です。'), true);
  assert.equal(text.includes("■ You've hit your usage limit."), true);
  assert.equal(text.includes('[MANY-AI-CLI-DONE]'), false);
  assert.equal(text.includes('[/MANY-AI-CLI-'), false);
  assert.equal(state.inDone, false);
});

test('案 I/J: 折り返しで分断された DONE の OPEN もラッチし、割り込んだバイトは素通しする', () => {
  const input = bytes('\n[MANY-AI-CLI-\r\n  DONE]サマリー[/MANY-AI-CLI-DONE]after');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), '\n\r\n  サマリーafter');
  assert.equal(state.inDone, false);
});

test('案 I: 承認マーカーが折り返しで分断されてもタグだけ落とし、割り込んだバイトは素通しする', () => {
  // 案 H の「本文へ 1 バイトも足さない・削らない」を保つ。割り込んだ CR/LF/空白は CLI の描画指示。
  const input = bytes('\n[MANY-AI-CLI]Q1?\n1. opt[/MANY-AI-CL\r\n   I]tail');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), '\nQ1?\n1. opt\r\n   tail');
  assert.equal(state.inMarker, false);
});

test('案 I: 折り返しで分断された承認 OPEN も行頭ゲートを通ればラッチする', () => {
  const input = bytes('\r\n  [MANY-AI-CL\r\n  I]Q1?[/MANY-AI-CLI]tail');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), '\r\n  \r\n  Q1?tail');
  assert.equal(state.inMarker, false);
});

test('案 I: マーカーの途中に印字文字が割り込んだら一致させない（誤爆を増やさない）', () => {
  const input = bytes('\n[MANY-AI-CLI-DONE]本文 [/MANY-AI-CLI-x DONE]tail');
  const { out, state } = filterHubMarkersPure(input, initialState());
  // CLOSE と認めないので DONE ラッチは続くが、案 J では本文も壊れた CLOSE も素通しで届く
  assert.equal(str(out), '\n本文 [/MANY-AI-CLI-x DONE]tail');
  assert.equal(state.inDone, true);
});

test('案 I: 読み飛ばし総量が上限を超えたら一致させない', () => {
  const gap = ' '.repeat(MAX_MARKER_WRAP_GAP_BYTES + 10);
  const input = bytes(`\n[MANY-AI-CLI-DONE]本文 [/MANY-AI-CLI-${gap}DONE]tail`);
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  assert.equal(text.includes('本文 [/MANY-AI-CLI-'), true);
  assert.equal(text.endsWith('DONE]tail'), true);
  assert.equal(state.inDone, true);
});

test('案 I: 折り返しの隙間で chunk が割れても carry で次に繋ぐ', () => {
  const part1 = bytes('\n[MANY-AI-CLI-DONE]サマリー [/MANY-AI-CLI-\r\n   ');
  const { out: out1, state: state1 } = filterHubMarkersPure(part1, initialState());
  assert.equal(str(out1), '\nサマリー ');
  assert.equal(state1.inDone, true);
  assert.ok(state1.carry.length > 0);

  const part2 = bytes('DONE]after');
  const { out: out2, state: state2 } = filterHubMarkersPure(part2, state1);
  assert.equal(str(out2), '\r\n   after');
  assert.equal(state2.inDone, false);
  assert.equal(state2.carry.length, 0);
});

test('案 I: matchMarkerAllowingWrap は不一致・バイト不足・一致を区別する', () => {
  const pattern = bytes('[/MANY-AI-CLI-DONE]');
  assert.equal(matchMarkerAllowingWrap(bytes('xyz'), 0, pattern).consumed, 0);
  assert.equal(matchMarkerAllowingWrap(bytes('[/MANY-AI-CLI-'), 0, pattern).consumed, -1);
  const wrapped = bytes('[/MANY-AI-CLI-\r\n  DONE]');
  const m = matchMarkerAllowingWrap(wrapped, 0, pattern);
  assert.equal(m.consumed, wrapped.length);
  assert.equal(str(m.noise), '\r\n  ');
});

// ── 案 J（2026-09-03）: CLOSE が 1 バイトも描かれない再描画フレーム ──
// 代替画面では CLI が 2 次元のセルを絶対座標で塗り直すので、1 回の再描画に OPEN だけが入り、
// CLOSE は画面外にあって描かれないことが起きる（折り返しで分断されているのではなく不在）。
// 案 G〜I の「OPEN が来たら CLOSE まで捨てる」だと、そこから 32KB 溜まるまで端末が固まる。
// 2026-09-03 セッション #7 実測: 171 万バイト中 33 万バイト（19.3%）が xterm へ届かず、
// 最長 3 分 6 秒フリーズ。利用者からは「処理中にホイールで遡っても画面が動かない」に見えた。

test('案 J: CLOSE の来ない再描画フレームが後続フレームを飲み込まない', () => {
  // 遡りスクロール中の再描画: OPEN を含む行は描かれるが CLOSE の行は画面外で描かれない
  const frame1 = bytes('\x1b[2;3H  [MANY-AI-CLI-DONE] 完了サマリー本文\x1b[3;3H次の行');
  const { out: out1, state: state1 } = filterHubMarkersPure(frame1, initialState());
  assert.equal(str(out1), '\x1b[2;3H   完了サマリー本文\x1b[3;3H次の行');
  assert.equal(state1.inDone, true);

  // 次のホイール 1 ノッチぶんの再描画。旧実装ではここが丸ごと捨てられて画面が固まっていた。
  const frame2 = bytes('\x1b[2;3H遡った先の本文\x1b[3;3Hさらに前の行');
  const { out: out2 } = filterHubMarkersPure(frame2, state1);
  assert.equal(str(out2), '\x1b[2;3H遡った先の本文\x1b[3;3Hさらに前の行');
});

test('案 J: CLOSE が OPEN より先に描かれても後続フレームを飲み込まない', () => {
  // 絶対座標の塗り直しでは行の描画順が転倒しうる（CLOSE の行 → OPEN の行）
  const frame = bytes('\x1b[27;3H[/MANY-AI-CLI-DONE]\x1b[25;3H[MANY-AI-CLI-DONE] サマリー');
  const { out, state } = filterHubMarkersPure(frame, initialState());
  const text = str(out);
  assert.equal(text.includes('[MANY-AI-CLI-DONE]'), false);
  assert.equal(text.includes('サマリー'), true);
  assert.equal(state.inDone, true);

  const next = bytes('\x1b[2;3H次のフレーム');
  const { out: out2 } = filterHubMarkersPure(next, state);
  assert.equal(str(out2), '\x1b[2;3H次のフレーム');
});
