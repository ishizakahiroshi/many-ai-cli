// \x1b[?25l（カーソル非表示）〜 \x1b[?25h（表示）ブロックの分類・破棄フィルタの純関数版。
// terminal.ts の filterCursorHideShowBlocksForDisplay から状態の橋渡しのみ受けて呼ばれる。
// hub-marker-filter.ts と同じ方式で、DOM/xterm 依存なしで node:test から検証可能にしている。
//
// 分類ルール:
// - 絶対カーソル移動（CUP \x1b[row;colH）なし、または LF あり → 本文描画として通過
// - CUP あり・LF なし → ステータスバー更新（スピナー等）として破棄
//   （破棄ブロックの可読テキスト抽出は呼び元がイベント経由で行う）
// - ただし alternate screen buffer（\x1b[?1049h〜\x1b[?1049l）中は破棄しない。
//   本フィルタの目的は「メインバッファの scrollback へのステータスバー化石化防止」であり、
//   scrollback を持たない alt buffer では防御自体が不要。Grok Build は alt buffer 上で
//   応答本文も CUP・LF なしで描画するため、破棄すると応答が画面に一切出なくなる
//   （bugfix_grok-response-discarded-by-cursor-hide-filter_2026-07-03.md）。
//   alt buffer 判定は xterm の buffer.active.type ではなく本ストリーム内の ?1049h/l を
//   自前追跡する（write キューの遅延でストリーム位置とずれるため）。
//   ?1049h/l は ?25l ブロック外・内の両方で検出する。Grok はブロック外で発行するが、
//   opencode（opentui）は起動時に ?25l → OSC 群 → ?1049h の順でブロック内に発行するため、
//   ブロック外のみの検出だと altScreen=false のままモデルピッカー等の再描画
//   （CUP・LF なし）が全て filter-discard され、上下キーの反応が数秒単位で欠落する
//   （bugfix_opencode-picker-discarded-by-cursor-hide-filter_2026-07-03.md）。
//   ブロック内で検出したシーケンスは blockBuf に積み、通過時にそのまま再生される。

import { bytesStartWith, isPossiblePrefix } from './hub-marker-filter.js';

const asciiEncoder = new TextEncoder();
export const hideCursorSeq = asciiEncoder.encode('\x1b[?25l');
export const showCursorSeq = asciiEncoder.encode('\x1b[?25h');
export const altScreenEnterSeq = asciiEncoder.encode('\x1b[?1049h');
export const altScreenExitSeq = asciiEncoder.encode('\x1b[?1049l');

// Claude Code の /model 等のセレクタダイアログは描画後カーソルを非表示のままにして
// ?25h を送らないため、閉じを待つ実装だと描画全体（<2KB）が blockBuf に滞留する。
// 閾値超過時は非ステータス扱いで通過させる（詳細は terminal.ts の旧コメント由来）。
export const MAX_CURSOR_HIDE_BUF = 2048;

export interface CursorHideFilterState {
  carry: Uint8Array;
  inBlock: boolean;
  blockBuf: number[];
  hasAbsPos: boolean;
  hasNewline: boolean;
  altScreen: boolean;
}

export type CursorHideEventKind =
  | 'block-passthrough-overflow'
  | 'block-passthrough-show'
  | 'block-passthrough-alt'
  | 'block-passthrough-lf'
  | 'filter-discard';

export interface CursorHideEvent {
  kind: CursorHideEventKind;
  blockBuf: number[];
  hasAbsPos: boolean;
  hasNewline: boolean;
}

export function filterCursorHideBlocksPure(
  bytes: Uint8Array,
  state: CursorHideFilterState,
): { out: Uint8Array; state: CursorHideFilterState; events: CursorHideEvent[] } {
  const combined = new Uint8Array(state.carry.length + bytes.length);
  combined.set(state.carry, 0);
  combined.set(bytes, state.carry.length);

  const out: number[] = [];
  const events: CursorHideEvent[] = [];
  let i = 0;
  let inBlock = state.inBlock;
  let blockBuf: number[] = [...state.blockBuf];
  let hasAbsPos = state.hasAbsPos;
  let hasNewline = state.hasNewline;
  let altScreen = state.altScreen;

  while (i < combined.length) {
    if (!inBlock) {
      if (bytesStartWith(combined, i, altScreenEnterSeq)) {
        altScreen = true;
        for (const b of altScreenEnterSeq) out.push(b);
        i += altScreenEnterSeq.length;
        continue;
      }
      if (bytesStartWith(combined, i, altScreenExitSeq)) {
        altScreen = false;
        for (const b of altScreenExitSeq) out.push(b);
        i += altScreenExitSeq.length;
        continue;
      }
      if (bytesStartWith(combined, i, hideCursorSeq)) {
        inBlock = true;
        blockBuf = [];
        hasAbsPos = false;
        hasNewline = false;
        i += hideCursorSeq.length;
        continue;
      }
      if (isPossiblePrefix(combined, i, [hideCursorSeq, altScreenEnterSeq, altScreenExitSeq])) {
        return {
          out: new Uint8Array(out),
          state: { carry: combined.slice(i), inBlock: false, blockBuf: [], hasAbsPos: false, hasNewline: false, altScreen },
          events,
        };
      }
      out.push(combined[i]);
      i++;
    } else {
      // バッファ上限超過時は非ステータス扱いで通過
      if (blockBuf.length >= MAX_CURSOR_HIDE_BUF) {
        events.push({ kind: 'block-passthrough-overflow', blockBuf, hasAbsPos, hasNewline });
        for (const b of hideCursorSeq) out.push(b);
        for (const b of blockBuf) out.push(b);
        inBlock = false;
        blockBuf = [];
        hasAbsPos = false;
        hasNewline = false;
        continue;
      }
      if (bytesStartWith(combined, i, showCursorSeq)) {
        if (!hasAbsPos || hasNewline) {
          // ステータスバー更新でない（絶対移動なし or 複数行の本文描画） → 通過
          events.push({ kind: 'block-passthrough-show', blockBuf, hasAbsPos, hasNewline });
          for (const b of hideCursorSeq) out.push(b);
          for (const b of blockBuf) out.push(b);
          for (const b of showCursorSeq) out.push(b);
        } else if (altScreen) {
          // alt buffer 中は scrollback が無く化石化しないため素通し。
          // ライブ進捗行の抽出は呼び元がこのイベントで従来どおり行う。
          events.push({ kind: 'block-passthrough-alt', blockBuf, hasAbsPos, hasNewline });
          for (const b of hideCursorSeq) out.push(b);
          for (const b of blockBuf) out.push(b);
          for (const b of showCursorSeq) out.push(b);
        } else {
          // ステータスバー更新（スピナー進捗等）は scrollback へ描かず破棄するが、
          // 可読テキストを抽出して専用ライブ行に出し、進捗を可視化する（呼び元が実施）。
          events.push({ kind: 'filter-discard', blockBuf, hasAbsPos, hasNewline });
        }
        inBlock = false;
        blockBuf = [];
        hasAbsPos = false;
        hasNewline = false;
        i += showCursorSeq.length;
        continue;
      }
      // ブロック内の alt screen enter/exit も altScreen 状態へ反映する（opencode 対応）。
      // シーケンス自体は blockBuf に積み、ブロック通過時にそのまま再生される。
      if (bytesStartWith(combined, i, altScreenEnterSeq)) {
        altScreen = true;
        for (const b of altScreenEnterSeq) blockBuf.push(b);
        i += altScreenEnterSeq.length;
        continue;
      }
      if (bytesStartWith(combined, i, altScreenExitSeq)) {
        altScreen = false;
        for (const b of altScreenExitSeq) blockBuf.push(b);
        i += altScreenExitSeq.length;
        continue;
      }
      if (isPossiblePrefix(combined, i, [showCursorSeq, altScreenEnterSeq, altScreenExitSeq])) {
        return {
          out: new Uint8Array(out),
          state: { carry: combined.slice(i), inBlock: true, blockBuf, hasAbsPos, hasNewline, altScreen },
          events,
        };
      }
      // \x1b[row;colH（row・col ともに数字あり）を検出したらステータス更新とみなす
      if (!hasAbsPos && combined[i] === 0x1b && i + 4 < combined.length && combined[i + 1] === 0x5b) {
        let j = i + 2;
        let rowDigits = 0;
        while (j < combined.length && combined[j] >= 0x30 && combined[j] <= 0x39) { j++; rowDigits++; }
        if (rowDigits > 0 && j < combined.length && combined[j] === 0x3b) {
          j++;
          let colDigits = 0;
          while (j < combined.length && combined[j] >= 0x30 && combined[j] <= 0x39) { j++; colDigits++; }
          if (colDigits > 0 && j < combined.length && combined[j] === 0x48) {
            hasAbsPos = true;
            for (let k = i; k <= j; k++) blockBuf.push(combined[k]);
            i = j + 1;
            continue;
          }
        }
      }
      if (combined[i] === 0x0A) {
        // 改行入り = 本文描画と確定。?25h を待たずに通過させてブロックを抜ける。
        // 以降のバイトは生のまま通過し、後続の ?25h も（来れば）そのまま流れる。
        blockBuf.push(combined[i]);
        i++;
        events.push({ kind: 'block-passthrough-lf', blockBuf, hasAbsPos, hasNewline: true });
        for (const b of hideCursorSeq) out.push(b);
        for (const b of blockBuf) out.push(b);
        inBlock = false;
        blockBuf = [];
        hasAbsPos = false;
        hasNewline = false;
        continue;
      }
      blockBuf.push(combined[i]);
      i++;
    }
  }

  return {
    out: new Uint8Array(out),
    state: { carry: new Uint8Array(0), inBlock, blockBuf, hasAbsPos, hasNewline, altScreen },
    events,
  };
}
