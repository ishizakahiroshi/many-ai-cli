// session-strip-branch.ts — セッション帯に出すブランチ名の省略規則。
//
// DOM に触らない純関数だけを置く（node:test から検証するため。sidebar-tree.ts /
// cwd-path.ts と同じ切り分け）。描画は session-strip.ts が担当する。
//
// 由来: docs/local/plan_project-box-open-and-session-strip_c2_session-strip.md

/** 帯に出すブランチ名の既定の上限文字数。超えたら真ん中を省く。 */
export const BRANCH_MAX_CHARS = 18;

/**
 * ブランチ名の真ん中を省いて頭と尻を残す。
 *
 * CSS の text-overflow では尻が落ちるだけなので、`feature/plan-alpha` と
 * `feature/plan-bravo` のように「頭が同じで尻だけ違う」ブランチが区別できなくなる
 * （relay の子を 2 本以上動かすとまさにこの形になる）。頭と尻の両方を残すのが要件。
 *
 * 返る文字列の長さは常に maxChars 以下。サロゲートペア（絵文字等）は割らない。
 * 空・null・undefined は空文字を返す（帯ではブランチの部分ごと出さないため）。
 */
export function abbreviateBranchName(branch: unknown, maxChars: number = BRANCH_MAX_CHARS): string {
  const text = String(branch ?? '');
  if (!text) return '';
  // 省略記号 1 文字 + 頭 1 文字 + 尻 1 文字 が成立する最小が 3。
  const limit = Math.max(3, Math.floor(Number(maxChars) || 0));
  const chars = Array.from(text);
  if (chars.length <= limit) return text;
  const keep = limit - 1;
  const head = Math.ceil(keep / 2);
  const tail = keep - head;
  return chars.slice(0, head).join('') + '…' + (tail > 0 ? chars.slice(chars.length - tail).join('') : '');
}
