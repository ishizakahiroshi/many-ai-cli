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
// 強制 flush + 状態リセットして描画を復帰させる。閾値は正常運用の 10 倍以上のマージン。
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

  while (i < combined.length) {
    if (inDone) {
      if (bytesStartWith(combined, i, hubDoneMarkerClose)) {
        i += hubDoneMarkerClose.length;
        inDone = false;
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
        // typo/欠落/切断で close が来ない: 諦めて強制 flush＋状態リセット
        inDone = false;
        flushBufToOut(doneBufArr, out);
        doneBufArr.length = 0;
        for (const b of eraseDisplayBelowBytes) out.push(b);
      }
      continue;
    }

    if (bytesStartWith(combined, i, hubDoneMarkerOpen)) {
      i += hubDoneMarkerOpen.length;
      inDone = true;
      continue;
    }

    const marker = hubMarkerBytePatterns.find(pattern => bytesStartWith(combined, i, pattern));
    if (marker) {
      i += marker.length;
      if (marker === hubMarkerEndBytes) {
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
    if (isPossibleMarkerPrefix(combined, i)) break;
    // 案 E: in-marker / in-done 中は本文を buf へ、外は out へ
    if (inMarker) {
      markerBufArr.push(combined[i]);
      i++;
      if (markerBufArr.length > MAX_MARKER_BUFFER_BYTES) {
        // typo/欠落/切断で close が来ない: 諦めて強制 flush＋状態リセット
        inMarker = false;
        flushBufToOut(markerBufArr, out);
        markerBufArr.length = 0;
        for (const b of eraseDisplayBelowBytes) out.push(b);
      }
    } else {
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
    },
  };
}
