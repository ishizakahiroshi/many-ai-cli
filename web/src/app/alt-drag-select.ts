// alt-drag-select.ts — 代替画面の CLI（全画面モードの Claude Code 等）で、ドラッグ選択が
// 端末の上下端を越えたときに CLI の画面を送りながら選択を続けるための純関数。
//
// 代替画面には xterm の履歴（scrollback）が無いので、xterm 本来の「ドラッグ中に端を越えたら
// 送る」処理（SelectionService._dragScroll）は送る先が無く、画面が動かない。画面を送れるのは
// CLI 自身だけで、xterm の選択は画面座標のまま残る。そこで CLI へ 1 段送るたびに送る前と
// 後の画面を突き合わせ、
//   (1) 本文が何行ずれたか
//   (2) 選択の起点から画面の外へ出た行はどれか
// を求める。画面の外へ出た行の文字は DOM 側（alt-drag-select-view.ts）が覚えておき、
// コピー時に見えている選択へつなげる。
//
// 行の比較は translateToString(true) 済み（行末空白なし）の文字列どうしで行う。
// 突き合わせられなかったときは推測で埋めず 'lost' を返す（コピーした文字に黙って
// 抜けや重複が出るより、選択を解いて知らせるほうがよいため）。

export interface AltDragAnchor {
  col: number;
  row: number;
}

export type AltDragShift =
  | { kind: 'moved'; lines: number }
  | { kind: 'still' }
  | { kind: 'lost' };

export interface AltDragStep {
  /** 画面の外へ出た、選択に入る行（上から下の順）。 */
  leftLines: string[];
  /** 送った後の画面での選択の起点。 */
  anchor: AltDragAnchor;
}

// 1 文字の繰り返し（罫線・区切り線）や短すぎる行は、どのずれ幅でも一致してしまうので
// ずれ幅の根拠に数えない。全角を含む行は 2 文字でも情報量がある。
function isMeaningfulLine(line: string): boolean {
  const trimmed = line.trim();
  if (trimmed.length < 2) return false;
  if (new Set(trimmed).size < 2) return false;
  return trimmed.length >= 4 || /[^\x00-\x7F]/.test(trimmed);
}

// 同じ行か。疑似レールの確定判定（terminal-history-strategy.ts の areAltLinesMatching）と
// 同じく、行末 1〜2 文字の揺らぎ（CLI が行末に描くスクロール位置の記号など）は同じ行と
// みなす。ただし短い行・空行は完全一致だけにする（数文字の行に揺らぎを認めると、
// 無関係な行とも一致してしまう）。
function isSameLine(a: string, b: string): boolean {
  if (a === b) return true;
  if (Math.min(a.trim().length, b.trim().length) < 4) return false;
  return Math.abs(a.length - b.length) <= 2 && (a.startsWith(b) || b.startsWith(a));
}

// direction > 0 は「下へ送った」（本文は上へずれる: after[i] === before[i + k]）。
// direction < 0 は「上へ送った」（本文は下へずれる: after[i + k] === before[i]）。
function countMatches(before: string[], after: string[], direction: number, k: number): number {
  let matches = 0;
  const rows = Math.min(before.length, after.length);
  for (let i = 0; i + k < rows; i++) {
    const b = direction > 0 ? before[i + k] : before[i];
    const a = direction > 0 ? after[i] : after[i + k];
    if (isMeaningfulLine(b) && isSameLine(b, a)) matches++;
  }
  return matches;
}

/**
 * 1 段送った前後の画面から、本文のずれ行数を測る。
 * - 'moved': 本文が送った方向へ lines 行ずれた
 * - 'still': 動いていない（CLI の履歴の端・入力がまだ描かれていない）。
 *   ステータス行などが書き換わっていても、本文の大半が同じ位置にあれば still
 * - 'lost': 画面は変わったが、ずれ幅を 1 つに決められない
 */
export function measureAltDragShift(before: string[], after: string[], direction: number): AltDragShift {
  if (before.length !== after.length || before.length === 0 || direction === 0) return { kind: 'lost' };
  const rows = before.length;
  if (before.every((line, i) => line === after[i])) return { kind: 'still' };
  const stay = countMatches(before, after, direction, 0);
  let best = 0;
  let bestMatches = 0;
  let tied = false;
  for (let k = 1; k < rows; k++) {
    const matches = countMatches(before, after, direction, k);
    if (matches > bestMatches) {
      best = k;
      bestMatches = matches;
      tied = false;
    } else if (matches === bestMatches && matches > 0) {
      tied = true;
    }
  }
  const required = rows >= 4 ? 2 : 1;
  if (best > 0 && !tied && bestMatches >= required && bestMatches > stay) {
    return { kind: 'moved', lines: best };
  }
  // 固定の入力欄・ステータス行と本文の大半が同じ位置に残っている＝送りは効いていない。
  const meaningful = before.filter(isMeaningfulLine).length;
  if (stay > 0 && stay * 2 >= meaningful) return { kind: 'still' };
  return { kind: 'lost' };
}

/**
 * 本文が direction 方向へ shift 行ずれたとき、選択の起点から画面の外へ出た行と、
 * 送った後の画面での起点を求める。
 *
 * 下へ送った場合は起点から下へ、上へ送った場合は起点から上へ 1 行ずつ見て、
 * 送った後の画面の「ずれた先の行」に同じ文字列が残っていれば、そこから先は
 * まだ見えている。残っていない行が画面の外へ出た行になる。画面の上端（下端）に
 * 固定の見出し・入力欄がある CLI でも、ずれた先がその固定行に当たるので同じ判定で済む。
 *
 * anchorPart は起点の行のうち選択に入る部分（下へ送る場合は起点から行末、上へ送る
 * 場合は行頭から起点まで）。起点の行が画面の外へ出たとき、その行はこの文字列で覚える。
 * 起点から見て画面の端まで全部が外へ出た（＝突き合わせの前提が崩れている）ときは null。
 */
export function stepAltDragAnchor(
  before: string[],
  after: string[],
  direction: number,
  shift: number,
  anchor: AltDragAnchor,
  anchorPart: string,
  cols: number,
): AltDragStep | null {
  const rows = Math.min(before.length, after.length);
  if (shift <= 0 || anchor.row < 0 || anchor.row >= rows) return null;
  const leftLines: string[] = [];
  if (direction > 0) {
    let j = anchor.row;
    for (; j < rows; j++) {
      const target = j - shift;
      if (target >= 0 && isSameLine(after[target], before[j])) break;
      leftLines.push(j === anchor.row ? anchorPart : before[j]);
    }
    if (j >= rows) return null;
    return {
      leftLines,
      anchor: leftLines.length === 0 ? { col: anchor.col, row: anchor.row - shift } : { col: 0, row: j - shift },
    };
  }
  let j = anchor.row;
  for (; j >= 0; j--) {
    const target = j + shift;
    if (target < rows && isSameLine(after[target], before[j])) break;
    leftLines.unshift(j === anchor.row ? anchorPart : before[j]);
  }
  if (j < 0) return null;
  return {
    leftLines,
    anchor: leftLines.length === 0 ? { col: anchor.col, row: anchor.row + shift } : { col: cols, row: j + shift },
  };
}
