// xterm に渡すバイト列から [MANY-AI-CLI]…[/MANY-AI-CLI] / [MANY-AI-CLI-DONE]…
// [/MANY-AI-CLI-DONE] の OPEN/CLOSE タグを剥がし、ブロック本文は ANSI escape を全て
// 除去した plain text として xterm に流す純粋関数。
// terminal.ts の filterHubMarkersForDisplay から状態の橋渡しのみ受けて呼ばれる。
// 純関数として切り出すことで DOM/xterm 依存なしで node:test から検証可能にしている。
//
// 案 E（2026-06-24）: マーカーブロック内の本文を一旦 buf に貯め、CLOSE 到達時に
// ANSI escape（CSI / OSC / ESC+単一バイト / 裸 ESC）を全部除去して xterm に出す。
// 案 B（2026-06-23）の「本文ごと非表示」では CLI 側で質問本文が読めず、案 C（タグだけ
// 剥がし本文 pass-through）では Claude Code の INK popup が出す絶対カーソル位置指定
// (\x1b[<row>;<col>H 等) が xterm のスクロールと衝突して画面崩れを起こす可能性が残った。
// 案 E は「衝突原理を生む ESC シーケンスをマーカー内では一切通さない」ことで両立する。
// 質問本文の色情報は失うが、現状そこに色は付いていないので実害なし。
//
// close 後の erase-below は、OPEN マーカー到達前に部分流出した popup 残骸を掃除する保険。

export const hubMarkerBytePatterns = [
  new TextEncoder().encode('[MANY-AI-CLI]'),
  new TextEncoder().encode('[/MANY-AI-CLI]'),
];
export const hubMarkerEndBytes = hubMarkerBytePatterns[1];
export const hubDoneMarkerOpen = new TextEncoder().encode('[MANY-AI-CLI-DONE]');
export const hubDoneMarkerClose = new TextEncoder().encode('[/MANY-AI-CLI-DONE]');
export const eraseDisplayBelowBytes = new TextEncoder().encode('\x1b[J');

// 案 E セーフガード（2026-07-01）: 実運用のマーカー本文は数百 B〜数 KB。
// close マーカー typo（例: [/MANARY-AI-CLI-DONE]）や AI 側の切断で close が来ないと、
// inMarker / inDone 状態のまま bytes が永久蓄積し、以降の CLI 側描画が全部 buf に飲まれて
// xterm が凍結する（2026-07-01 セッション #16 実測で確認）。閾値を超えたら諦めて
// 状態リセットして描画を復帰させる。閾値は正常運用の 10 倍以上のマージン。
// 2026-07-04: 超過時の扱いを「強制 flush（剥離テキスト一括ダンプ）」から「破棄」へ変更。
// close が 32KB 来ない時点で buf の中身は本文ではなく、Ink が interleave した
// スピナー再描画・ステータス行の堆積であり、剥離ダンプすると 32KB のゴミテキストが
// scrollback に化石化する（bugfix_spinner-cup-not-consumed-in-webui_2026-07-02.md で実測）。
// 承認 UI は Hub 側 Go の PTY 解析から出るため、破棄しても承認ボタンには影響しない。
export const MAX_MARKER_BUFFER_BYTES = 32 * 1024;

const utf8Encoder = new TextEncoder();
const utf8Decoder = new TextDecoder('utf-8');

// 案 E: マーカーブロック内で剥がす ANSI シーケンス
// - CSI: ESC [ <params> <intermediate> <final>  (final は @-~)
// - OSC: ESC ] <data> (BEL | ESC \)
// - ESC + 単一バイト（@-Z, \, ]-_ 等）
// - 上記に該当しない裸の ESC
const ANSI_OSC_RE = /\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g;
const ANSI_CSI_RE = /\x1b\[[0-?]*[ -/]*[@-~]/g;
const ANSI_ESC_SINGLE_RE = /\x1b[@-Z\\-_]/g;
const ANSI_ESC_BARE_RE = /\x1b/g;

export function stripAnsiFromString(s: string): string {
  return s
    .replace(ANSI_OSC_RE, '')
    .replace(ANSI_CSI_RE, '')
    .replace(ANSI_ESC_SINGLE_RE, '')
    .replace(ANSI_ESC_BARE_RE, '');
}

export function bytesStartWith(bytes: Uint8Array, offset: number, pattern: Uint8Array): boolean {
  if (offset + pattern.length > bytes.length) return false;
  for (let i = 0; i < pattern.length; i++) {
    if (bytes[offset + i] !== pattern[i]) return false;
  }
  return true;
}

export function isPossiblePrefix(bytes: Uint8Array, offset: number, patterns: Uint8Array[]): boolean {
  const remaining = bytes.length - offset;
  return patterns.some((pattern) => {
    if (remaining >= pattern.length) return false;
    for (let i = 0; i < remaining; i++) {
      if (bytes[offset + i] !== pattern[i]) return false;
    }
    return true;
  });
}

export function isPossibleMarkerPrefix(bytes: Uint8Array, offset: number): boolean {
  return isPossiblePrefix(bytes, offset, hubMarkerBytePatterns) ||
    isPossiblePrefix(bytes, offset, [hubDoneMarkerOpen]);
}

export type HubMarkerFilterState = {
  carry: Uint8Array;
  inDone: boolean;
  inMarker: boolean;
  markerBuf: Uint8Array;
  doneBuf: Uint8Array;
  // 行頭ゲート（2026-07-04）: OPEN マーカーは「行頭（空白のみ先行）」でのみラッチする。
  // AI が地の文でマーカーをリテラル引用した場合（例:「…はすべて [MANY-AI-CLI] マーカー形式で…」）、
  // close が来ないまま inMarker にラッチし、後続の画面再描画 32KB を飲み込む事故の再発防止
  // （2026-07-04 orchestration 指揮者セッションで実測）。正規マーカーは Claude Code の描画上
  // 必ず「行境界（\r\n または CUP）＋インデント空白」の直後に現れることをログで確認済み。
  lineStart?: boolean;
  // lineStart 判定用の ESC シーケンス解析フェーズ（0=通常 / 1=ESC 直後 / 2=CSI 中 / 3=OSC 中）
  escPhase?: number;
};

function flushBufToOut(buf: number[], out: number[]): void {
  if (buf.length === 0) return;
  const bytes = new Uint8Array(buf);
  const text = utf8Decoder.decode(bytes);
  const stripped = stripAnsiFromString(text);
  const encoded = utf8Encoder.encode(stripped);
  for (const b of encoded) out.push(b);
}

export function filterHubMarkersPure(bytes: Uint8Array, state: HubMarkerFilterState): {
  out: Uint8Array;
  state: HubMarkerFilterState;
} {
  const carry = state.carry || new Uint8Array(0);
  const combined = new Uint8Array(carry.length + bytes.length);
  combined.set(carry, 0);
  combined.set(bytes, carry.length);

  const out: number[] = [];
  let i = 0;
  let inDone = state.inDone;
  let inMarker = state.inMarker;
  const markerBufArr: number[] = Array.from(state.markerBuf || new Uint8Array(0));
  const doneBufArr: number[] = Array.from(state.doneBuf || new Uint8Array(0));
  let lineStart = state.lineStart ?? true;
  let escPhase = state.escPhase ?? 0;

  // 行頭ゲート用の per-byte 状態機械。\r・\n・CUP/HVP（CSI ... H / f）で行頭に復帰し、
  // 空白・制御文字・ESC シーケンスは中立、印字文字（マルチバイト含む）で行頭を解除する。
  // Ink はインデントを空白または CUP の列指定で描くため、これで
  // 「行境界＋空白のみ先行」= 正規マーカー位置 と「文中」= prose リテラル を区別できる。
  const trackByte = (b: number): void => {
    switch (escPhase) {
      case 1: // ESC 直後
        if (b === 0x5b) escPhase = 2;        // CSI
        else if (b === 0x5d) escPhase = 3;   // OSC
        else escPhase = 0;                    // ESC+単一バイト（中立）
        return;
      case 2: // CSI 中
        if (b >= 0x40 && b <= 0x7e) {
          escPhase = 0;
          if (b === 0x48 || b === 0x66) lineStart = true; // CUP 'H' / HVP 'f'
        }
        return;
      case 3: // OSC 中
        if (b === 0x07) escPhase = 0;
        else if (b === 0x1b) escPhase = 1;
        return;
      default:
        if (b === 0x1b) { escPhase = 1; return; }
        if (b === 0x0a || b === 0x0d) { lineStart = true; return; }
        if (b === 0x20 || b === 0x09 || b < 0x20) return; // 空白・制御文字は中立
        lineStart = false;
    }
  };

  while (i < combined.length) {
    if (inDone) {
      if (bytesStartWith(combined, i, hubDoneMarkerClose)) {
        i += hubDoneMarkerClose.length;
        inDone = false;
        lineStart = false;
        // 案 E: 貯めた DONE 本文を ANSI 剥離してから out へ
        flushBufToOut(doneBufArr, out);
        doneBufArr.length = 0;
        for (const b of eraseDisplayBelowBytes) out.push(b);
        continue;
      }
      if (isPossiblePrefix(combined, i, [hubDoneMarkerClose])) break;
      // 案 E: DONE 本文は buf に貯める（次チャンク跨ぎでも累積）
      doneBufArr.push(combined[i]);
      i++;
      if (doneBufArr.length > MAX_MARKER_BUFFER_BYTES) {
        // typo/欠落/切断で close が来ない: 本文ではなく再描画の堆積とみなし破棄＋状態リセット
        inDone = false;
        lineStart = false;
        doneBufArr.length = 0;
      }
      continue;
    }

    if (bytesStartWith(combined, i, hubDoneMarkerOpen)) {
      if (lineStart) {
        i += hubDoneMarkerOpen.length;
        inDone = true;
        lineStart = false;
        continue;
      }
      // 文中の prose リテラルはマーカー扱いせず素通し（'[' 1 バイトだけ進めて再走査）
      trackByte(combined[i]);
      out.push(combined[i]);
      i++;
      continue;
    }

    const marker = hubMarkerBytePatterns.find(pattern => bytesStartWith(combined, i, pattern));
    if (marker) {
      const isClose = marker === hubMarkerEndBytes;
      // 行頭ゲート: OPEN は行頭のみラッチ。close は inMarker 中なら位置を問わず受理し、
      // stray close（開きなし）は行頭のみマーカー扱い（文中の prose リテラルは素通し）。
      if (inMarker || lineStart) {
        i += marker.length;
        lineStart = false;
        if (isClose) {
          // close: 貯めた本文を ANSI 剥離して out へ
          inMarker = false;
          flushBufToOut(markerBufArr, out);
          markerBufArr.length = 0;
          for (const b of eraseDisplayBelowBytes) out.push(b);
        } else {
          // open: 以降の close までの本文は markerBufArr に貯める
          inMarker = true;
        }
        continue;
      }
      if (!inMarker) {
        trackByte(combined[i]);
        out.push(combined[i]);
        i++;
        continue;
      }
    }
    // マーカー prefix の carry は「ラッチし得る文脈」のときだけ行う（文中は素通しでよい）
    if ((inMarker || lineStart) && isPossibleMarkerPrefix(combined, i)) break;
    // 案 E: in-marker / in-done 中は本文を buf へ、外は out へ
    if (inMarker) {
      markerBufArr.push(combined[i]);
      i++;
      if (markerBufArr.length > MAX_MARKER_BUFFER_BYTES) {
        // typo/欠落/切断で close が来ない: 本文ではなく再描画の堆積とみなし破棄＋状態リセット
        inMarker = false;
        lineStart = false;
        markerBufArr.length = 0;
      }
    } else {
      trackByte(combined[i]);
      out.push(combined[i]);
      i++;
    }
  }

  return {
    out: new Uint8Array(out),
    state: {
      carry: combined.slice(i),
      inDone,
      inMarker,
      markerBuf: new Uint8Array(markerBufArr),
      doneBuf: new Uint8Array(doneBufArr),
      lineStart,
      escPhase,
    },
  };
}
