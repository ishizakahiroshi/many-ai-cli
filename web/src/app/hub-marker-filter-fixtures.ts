import assert from 'node:assert/strict';
import test from 'node:test';
import {
  filterHubMarkersPure,
  stripAnsiFromString,
  MAX_MARKER_BUFFER_BYTES,
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
    markerBuf: new Uint8Array(0),
    doneBuf: new Uint8Array(0),
  };
}

const ERASE_BELOW = '\x1b[J';

test('stripAnsiFromString: CSI / OSC / 単一 ESC / 裸 ESC をすべて剥がす', () => {
  // CSI (cursor positioning, SGR, erase)
  assert.equal(stripAnsiFromString('hello\x1b[31mred\x1b[0m'), 'hellored');
  assert.equal(stripAnsiFromString('a\x1b[24;3Hb'), 'ab');
  assert.equal(stripAnsiFromString('x\x1b[2Jy'), 'xy');
  // OSC (window title), terminated by BEL or ESC \
  assert.equal(stripAnsiFromString('p\x1b]0;title\x07q'), 'pq');
  assert.equal(stripAnsiFromString('p\x1b]0;title\x1b\\q'), 'pq');
  // ESC + single byte
  assert.equal(stripAnsiFromString('a\x1bMb'), 'ab');
  // bare ESC
  assert.equal(stripAnsiFromString('a\x1bb'), 'ab');
  // plain
  assert.equal(stripAnsiFromString('plain'), 'plain');
});

test('filterHubMarkersPure: 1 チャンクで完結する [MANY-AI-CLI] ブロックはタグだけ剥がして本文は残す', () => {
  const input = bytes('preamble\n[MANY-AI-CLI]Q1 question?\n1. opt1\n2. opt2\n[/MANY-AI-CLI]tail');
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  assert.equal(text.includes('Q1 question'), true);
  assert.equal(text.includes('opt1'), true);
  assert.equal(text.includes('opt2'), true);
  assert.equal(text.includes('[MANY-AI-CLI]'), false);
  assert.equal(text.includes('[/MANY-AI-CLI]'), false);
  assert.equal(text.startsWith('preamble\n'), true);
  assert.equal(text.endsWith(`${ERASE_BELOW}tail`), true, `text="${text}"`);
  assert.equal(state.inMarker, false);
  assert.equal(state.inDone, false);
  assert.equal(state.carry.length, 0);
  assert.equal(state.markerBuf.length, 0);
});

test('filterHubMarkersPure (案 E): ブロック内の絶対カーソル位置指定は剥がされ本文だけ残る', () => {
  // INK 衝突の原因である \x1b[<row>;<col>H をブロック内に混ぜたケース
  const block = '\x1b[24;3HQ1 質問?\x1b[25;3H1. opt1\x1b[26;3H2. opt2';
  const input = bytes(`[MANY-AI-CLI]${block}[/MANY-AI-CLI]`);
  const { out } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  // 絶対位置指定は剥がれる（衝突原理消失）
  assert.equal(text.includes('\x1b[24;3H'), false);
  assert.equal(text.includes('\x1b[25;3H'), false);
  assert.equal(text.includes('\x1b[26;3H'), false);
  // 本文テキストは残る
  assert.equal(text.includes('Q1 質問?'), true);
  assert.equal(text.includes('1. opt1'), true);
  assert.equal(text.includes('2. opt2'), true);
  assert.equal(text.endsWith(ERASE_BELOW), true);
});

test('filterHubMarkersPure (案 E): SGR 色指定もブロック内なら剥がれる', () => {
  const input = bytes('[MANY-AI-CLI]\x1b[31mred text\x1b[0m and \x1b[1mbold\x1b[m[/MANY-AI-CLI]');
  const { out } = filterHubMarkersPure(input, initialState());
  // close flush 後の erase-below（\x1b[J）だけが残り、SGR は全て剥がれる
  assert.equal(str(out), `red text and bold${ERASE_BELOW}`);
});

test('filterHubMarkersPure: 開きマーカーで終わるチャンク → 本文は markerBuf に持ち越し', () => {
  const part1 = bytes('preamble\n[MANY-AI-CLI]body1\nbody2');
  const { out: out1, state: state1 } = filterHubMarkersPure(part1, initialState());
  // OPEN 以降は markerBuf に貯まり out には流れない
  assert.equal(str(out1), 'preamble\n');
  assert.equal(state1.inMarker, true);
  assert.equal(state1.inDone, false);
  assert.equal(state1.carry.length, 0);
  assert.ok(state1.markerBuf.length > 0);

  const part2 = bytes('body3\n[/MANY-AI-CLI]tail');
  const { out: out2, state: state2 } = filterHubMarkersPure(part2, state1);
  // CLOSE 時に貯めた本文をまとめて ANSI 剥離して出す
  assert.equal(str(out2), `body1\nbody2body3\n${ERASE_BELOW}tail`);
  assert.equal(state2.inMarker, false);
  assert.equal(state2.markerBuf.length, 0);
});

test('filterHubMarkersPure: チャンク跨ぎの ANSI シーケンスも正しく剥がれる', () => {
  // ESC [ がチャンク1末尾、続きがチャンク2 という分割
  const part1 = bytes('[MANY-AI-CLI]hello\x1b[24');
  const { out: out1, state: state1 } = filterHubMarkersPure(part1, initialState());
  assert.equal(str(out1), '');
  assert.ok(state1.markerBuf.length > 0);

  const part2 = bytes(';3Hworld[/MANY-AI-CLI]end');
  const { out: out2 } = filterHubMarkersPure(part2, state1);
  // 跨いだ \x1b[24;3H が剥がれて helloworld になる
  assert.equal(str(out2), `helloworld${ERASE_BELOW}end`);
});

test('filterHubMarkersPure: 閉じマーカーの途中で chunk が割れても carry で次に繋ぐ', () => {
  const open = bytes('pre\n[MANY-AI-CLI]body[/MANY-AI-CLI');
  const { out: out1, state: state1 } = filterHubMarkersPure(open, initialState());
  // 'pre\n' が出力され、'body' は markerBuf に、CLOSE の prefix は carry に持ち越し
  assert.equal(str(out1), 'pre\n');
  assert.equal(state1.inMarker, true);
  assert.ok(state1.carry.length > 0);
  assert.ok(state1.markerBuf.length > 0);

  const rest = bytes(']post');
  const { out: out2, state: state2 } = filterHubMarkersPure(rest, state1);
  assert.equal(str(out2), `body${ERASE_BELOW}post`);
  assert.equal(state2.inMarker, false);
  assert.equal(state2.carry.length, 0);
  assert.equal(state2.markerBuf.length, 0);
});

test('filterHubMarkersPure (案 E): [MANY-AI-CLI-DONE] ブロックも ANSI 剥離して本文を出す', () => {
  const input = bytes('before\n[MANY-AI-CLI-DONE]\x1b[32mタスク完了\x1b[mしました[/MANY-AI-CLI-DONE]after');
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  assert.equal(text.includes('タスク完了しました'), true);
  // ANSI は close flush 後の erase-below（\x1b[J）以外すべて剥がれる
  assert.equal(text, `before\nタスク完了しました${ERASE_BELOW}after`);
  assert.equal(text.includes('[MANY-AI-CLI-DONE]'), false);
  assert.equal(text.includes('[/MANY-AI-CLI-DONE]'), false);
  assert.equal(text.startsWith('before\n'), true);
  assert.equal(text.endsWith(`${ERASE_BELOW}after`), true);
  assert.equal(state.inDone, false);
  assert.equal(state.doneBuf.length, 0);
});

test('filterHubMarkersPure: マーカー無しの通常バイトは素通し（ANSI も含む）', () => {
  const input = bytes('hello \x1b[1mworld\x1b[m');
  const { out, state } = filterHubMarkersPure(input, initialState());
  // ブロック外の ANSI はそのまま通す
  assert.equal(str(out), 'hello \x1b[1mworld\x1b[m');
  assert.equal(state.inMarker, false);
  assert.equal(state.inDone, false);
});

test('filterHubMarkersPure: 行頭の stray な閉じマーカーは隠す（開きなしでも例外を投げない）', () => {
  const input = bytes('text\n[/MANY-AI-CLI]more');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), `text\n${ERASE_BELOW}more`);
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
  const text = str(out);
  assert.equal(text, `A\nX${ERASE_BELOW}B\nY${ERASE_BELOW}C`);
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
  assert.equal(str(out2), `body${ERASE_BELOW}post`);
  assert.equal(state2.inMarker, false);
});

test('filterHubMarkersPure セーフガード: DONE close が typo で来ない場合、閾値超過で破棄＋状態リセット', () => {
  // 2026-07-01 セッション #16 実測の実例: [/MANARY-AI-CLI-DONE] とタイポし close 不一致
  const summary = 'サマリー本文';
  const filler = 'x'.repeat(MAX_MARKER_BUFFER_BYTES + 100);
  const input = bytes(`[MANY-AI-CLI-DONE]${summary} ${filler}[/MANARY-AI-CLI-DONE]after`);
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  // 2026-07-04: 閾値超過分は本文ではなく再描画の堆積とみなし破棄する
  // （旧仕様の強制 flush は 32KB の剥離ゴミを scrollback へ化石化させていた）
  assert.equal(text.includes(summary), false);
  // 状態は復帰しており、閾値超過後のバイトは素通しで届く（凍結しない）
  assert.equal(state.inDone, false);
  assert.equal(state.doneBuf.length, 0);
  assert.equal(text.includes('after'), true);
});

test('filterHubMarkersPure セーフガード: マーカー close が来ない場合も閾値超過で破棄＋状態リセット', () => {
  const body = 'Q1 質問? 1. 選択肢';
  const filler = 'y'.repeat(MAX_MARKER_BUFFER_BYTES + 100);
  // close が来ないまま追加のバイト列（Claude Code TUI が絶え間なく吐く spinner 相当）
  const input = bytes(`[MANY-AI-CLI]${body} ${filler}`);
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  // 閾値超過分は破棄（凍結せず、ゴミも出さない）
  assert.equal(text.includes(body), false);
  // 状態が復帰しており、超過後のバイトは素通し
  assert.equal(state.inMarker, false);
  assert.equal(state.markerBuf.length, 0);
  assert.equal(text.includes('yyy'), true);
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
  const text = str(out);
  assert.equal(text.includes('[MANY-AI-CLI]'), false);
  assert.equal(text.includes('Q1 質問?'), true);
  assert.equal(state.inMarker, false);
});

test('行頭ゲート: \\r\\n＋空白の直後の OPEN はラッチする', () => {
  const input = bytes('前置き\r\n  \x1b[K[MANY-AI-CLI]body[/MANY-AI-CLI]tail');
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  assert.equal(text.includes('[MANY-AI-CLI]'), false);
  assert.equal(text.includes('body'), true);
  assert.equal(state.inMarker, false);
});

test('行頭ゲート: 文中の prose リテラル DONE もラッチせず素通し', () => {
  const input = bytes('完了時は [MANY-AI-CLI-DONE] マーカーで報告します');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), '完了時は [MANY-AI-CLI-DONE] マーカーで報告します');
  assert.equal(state.inDone, false);
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
  assert.equal(str(out4), `body${ERASE_BELOW}`);
  assert.equal(state4.inMarker, false);
});
