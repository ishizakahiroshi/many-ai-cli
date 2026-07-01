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
  const input = bytes('preamble [MANY-AI-CLI]Q1 question?\n1. opt1\n2. opt2\n[/MANY-AI-CLI]tail');
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  assert.equal(text.includes('Q1 question'), true);
  assert.equal(text.includes('opt1'), true);
  assert.equal(text.includes('opt2'), true);
  assert.equal(text.includes('[MANY-AI-CLI]'), false);
  assert.equal(text.includes('[/MANY-AI-CLI]'), false);
  assert.equal(text.startsWith('preamble '), true);
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
  const text = str(out);
  assert.equal(text.includes('\x1b['), false);
  assert.equal(text.includes('red text and bold'), true);
});

test('filterHubMarkersPure: 開きマーカーで終わるチャンク → 本文は markerBuf に持ち越し', () => {
  const part1 = bytes('preamble [MANY-AI-CLI]body1\nbody2');
  const { out: out1, state: state1 } = filterHubMarkersPure(part1, initialState());
  // OPEN 以降は markerBuf に貯まり out には流れない
  assert.equal(str(out1), 'preamble ');
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
  const open = bytes('pre [MANY-AI-CLI]body[/MANY-AI-CLI');
  const { out: out1, state: state1 } = filterHubMarkersPure(open, initialState());
  // 'pre ' が出力され、'body' は markerBuf に、CLOSE の prefix は carry に持ち越し
  assert.equal(str(out1), 'pre ');
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
  const input = bytes('before [MANY-AI-CLI-DONE]\x1b[32mタスク完了\x1b[mしました[/MANY-AI-CLI-DONE]after');
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  assert.equal(text.includes('タスク完了しました'), true);
  assert.equal(text.includes('\x1b['), false);
  assert.equal(text.includes('[MANY-AI-CLI-DONE]'), false);
  assert.equal(text.includes('[/MANY-AI-CLI-DONE]'), false);
  assert.equal(text.startsWith('before '), true);
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

test('filterHubMarkersPure: stray な閉じマーカー単体は無視（開きなしでも例外を投げない）', () => {
  const input = bytes('text [/MANY-AI-CLI]more');
  const { out, state } = filterHubMarkersPure(input, initialState());
  assert.equal(str(out), `text ${ERASE_BELOW}more`);
  assert.equal(state.inMarker, false);
});

test('filterHubMarkersPure: 連続する 2 ブロックで本文が両方とも残る', () => {
  const input = bytes('A[MANY-AI-CLI]X[/MANY-AI-CLI]B[MANY-AI-CLI]Y[/MANY-AI-CLI]C');
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  assert.equal(text, `AX${ERASE_BELOW}BY${ERASE_BELOW}C`);
  assert.equal(state.inMarker, false);
});

test('filterHubMarkersPure: 開きマーカー文字列の途中で chunk が割れても carry で次に繋ぐ', () => {
  const part1 = bytes('pre [MANY-AI-CL');
  const { out: out1, state: state1 } = filterHubMarkersPure(part1, initialState());
  assert.equal(str(out1), 'pre ');
  assert.equal(state1.inMarker, false);
  assert.ok(state1.carry.length > 0);

  const part2 = bytes('I]body[/MANY-AI-CLI]post');
  const { out: out2, state: state2 } = filterHubMarkersPure(part2, state1);
  assert.equal(str(out2), `body${ERASE_BELOW}post`);
  assert.equal(state2.inMarker, false);
});

test('filterHubMarkersPure セーフガード: DONE close が typo で来ない場合、閾値超過で強制 flush＋状態リセット', () => {
  // 2026-07-01 セッション #16 実測の実例: [/MANARY-AI-CLI-DONE] とタイポし close 不一致
  const summary = 'サマリー本文';
  const filler = 'x'.repeat(MAX_MARKER_BUFFER_BYTES + 100);
  const input = bytes(`[MANY-AI-CLI-DONE]${summary} ${filler}[/MANARY-AI-CLI-DONE]after`);
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  // 強制 flush により本文と filler が xterm へ届く（凍結しない）
  assert.equal(text.includes(summary), true);
  // 状態は復帰しており、後続の 'after' も出力に含まれる
  assert.equal(state.inDone, false);
  assert.equal(state.doneBuf.length, 0);
  // typo の close マーカーは剥がれず素通しされるが、[/MANARY-... の部分文字列が text 末尾側に残る
  // （フィルタの責務は「凍結させない」ことで、typo の可読性は AI 側の責任）
});

test('filterHubMarkersPure セーフガード: マーカー close が来ない場合も閾値超過で強制 flush＋状態リセット', () => {
  const body = 'Q1 質問? 1. 選択肢';
  const filler = 'y'.repeat(MAX_MARKER_BUFFER_BYTES + 100);
  // close が来ないまま追加のバイト列（Claude Code TUI が絶え間なく吐く spinner 相当）
  const input = bytes(`[MANY-AI-CLI]${body} ${filler}`);
  const { out, state } = filterHubMarkersPure(input, initialState());
  const text = str(out);
  // 強制 flush により本文が xterm へ届く（凍結しない）
  assert.equal(text.includes(body), true);
  // 状態が復帰している
  assert.equal(state.inMarker, false);
  assert.equal(state.markerBuf.length, 0);
});
