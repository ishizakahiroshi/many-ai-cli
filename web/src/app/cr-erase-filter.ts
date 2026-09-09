// 単独 \r（CR）の直後へ \x1b[K（EL: Erase Line from cursor to right）を挿入する
// 表示用フィルタの純関数版。terminal.ts の filterBareCarriageReturnForDisplay から
// 状態の橋渡しのみ受けて呼ばれる。cursor-hide-filter.ts と同じ方式で、DOM/xterm 依存
// なしに node:test から検証できるようにしている。
//
// 挿入の目的:
// \r で行頭へ戻ったあと行末を消去しないと、短い上書きテキストの後ろに旧テキストの
// 末尾が残り、scrollback 上で混在して見える。\r\n（正常な改行ペア）の \r には挿入
// しない（不要かつ \n 前の空白消去になる）。
//
// alternate screen buffer（\x1b[?1049h 〜 \x1b[?1049l）中は挿入しない:
// この防御は「main buffer の scrollback に残留が化石化する」ことへの対策で、
// scrollback を持たない alt buffer では防御自体が不要。そのうえ Claude Code は
// alt buffer 上で「1 行描く → \r で行頭へ戻る → カーソル下移動で次の行」という
// 描き方をするため、\r の直後へ EL を入れると描いたばかりの行を毎回消してしまい、
// 画面がほぼ空になる（bugfix: Linux Hub の Claude ペイン黒画面・2026-09-01）。
// Linux の実測 PTY 出力で単独 CR 38 個 / LF 0 個。@xterm/headless 173x22 で再生した
// ところ、素の出力 674 文字に対し EL 挿入後は 73 文字（最下段のステータス行のみ）
// まで落ちた。alt 中の挿入をやめると 674 文字へ戻り、Codex（単独 CR 0 個）は
// 658 文字で不変だった。
// Windows Hub で露見しなかったのは ConPTY が画面を作り直して出すためで、実測した
// Windows のセッションログ 6 本はいずれも単独 CR が 0 個だった。
//
// alt 判定は xterm の buffer.active.type ではなく本ストリーム内の ?1049h/l を自前
// 追跡する（write キューの遅延でストリーム位置とずれるため。cursor-hide-filter.ts
// と同じ理由）。

import { bytesStartWith, isPossiblePrefix } from './hub-marker-filter.js';
import { altScreenEnterSeq, altScreenExitSeq } from './cursor-hide-filter.js';

const eraseLineSeq = new TextEncoder().encode('\x1b[K');

export interface CarriageReturnFilterState {
  carry: Uint8Array;
  altScreen: boolean;
}

export function filterBareCarriageReturnPure(
  bytes: Uint8Array,
  state: CarriageReturnFilterState,
): { out: Uint8Array; state: CarriageReturnFilterState } {
  const combined = new Uint8Array(state.carry.length + bytes.length);
  combined.set(state.carry, 0);
  combined.set(bytes, state.carry.length);

  const out: number[] = [];
  let altScreen = state.altScreen;
  let i = 0;

  while (i < combined.length) {
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
    if (combined[i] === 0x0d) {
      if (i + 1 >= combined.length) {
        // チャンク末尾の \r は次チャンクの先頭が \n かどうか未確定のため carry に残す。
        return { out: new Uint8Array(out), state: { carry: combined.slice(i), altScreen } };
      }
      out.push(combined[i]);
      if (combined[i + 1] !== 0x0a && !altScreen) {
        for (const b of eraseLineSeq) out.push(b);
      }
      i++;
      continue;
    }
    if (isPossiblePrefix(combined, i, [altScreenEnterSeq, altScreenExitSeq])) {
      // ?1049h/l がチャンク跨ぎで分割された可能性がある。alt 判定を取り違えると
      // 行が消える／残るが反転するため、確定するまで carry へ送る。
      return { out: new Uint8Array(out), state: { carry: combined.slice(i), altScreen } };
    }
    out.push(combined[i]);
    i++;
  }

  return { out: new Uint8Array(out), state: { carry: new Uint8Array(0), altScreen } };
}
