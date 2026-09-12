// 端末バッファ上のパス検出。DOM / xterm に依存しない純関数だけを置く。
// リンクポップアップや API 呼び出しは path-links.ts。

export type PathCandidate = {
  start: number;
  end: number;
  text: string;
};

export type PathWrapRow = {
  text: string;
  isWrapped: boolean;
  contentWidth: number;
};

// CLI が自分で入れた改行によるパス分断は、xterm の isWrapped では繋がらない。
// 1 本のパスが何行に割れても結合はするが、誤結合を抑える上限。
export const MAX_HARD_WRAP_EXTRA = 6;

export function trimTerminalPathCandidate(path: string): string {
  let text = String(path || '').trim().replace(/(?:\s*[,;:'"`<>\])}]+)+$/, '');
  // 拡張子の直後に全角/日本語が続く場合はそこで切る（相対パス・Unix 絶対パスにも適用）。
  // Windows 絶対パスは下の trimWindowsPathCandidate で同等処理を行う。
  text = text.replace(/(\.[a-zA-Z0-9]{1,15})\s*[぀-ヿ㐀-鿿＀-￯一-鿿].*$/u, '$1');
  if (/^[A-Za-z]:[\\/]/.test(text)) text = trimWindowsPathCandidate(text);
  text = stripTerminalLineSuffix(text);
  return text;
}

export function trimWindowsPathCandidate(path: string): string {
  let text = String(path || '');
  text = text.replace(/([\\/])\s+.*$/, '$1');
  text = text.replace(/(\.[a-zA-Z0-9]{1,15})\s*[぀-ヿ㐀-鿿＀-￯一-鿿].*$/u, '$1');
  text = text.replace(/\s+[぀-ヿ㐀-鿿＀-￯].*$/u, '');
  text = text.replace(/\s+[A-Za-z]$/, '');
  return text.replace(/(?:\s*[,;:'"`<>\])}]+)+$/, '');
}

export function stripTerminalLineSuffix(path: string): string {
  const text = String(path || '');
  return text.replace(/([^\s:]):\d+(?::\d+)?$/, '$1');
}

export function isAbsolutePath(path: string): boolean {
  return /^[A-Za-z]:[\\/]/.test(path) || path.startsWith('/');
}

// Windows drive paths can appear with either backslashes or forward slashes
// in terminal output, e.g. D:\src\app.go or C:/example/project/README.md.
// バッククォートは Unix / 相対パスと同じく除外する（Markdown のコードスパン閉じをパスに含めない）。
export const ABS_WIN_PATH_RE = /([A-Za-z]:[\\/](?:(?!\s+[A-Za-z]:[\\/])[^\x00-\x1f<>:"|?*(`])+)/g;
// 空白を挟んだ説明文中の区切り（例: "hljs / highlight / prism"）を
// Unix 絶対パスとして誤検出しないよう、セグメント内の空白は許可しない。
export const ABS_UNIX_PATH_RE = /(\/[^\s\/\x00-\x1f"'<>`|(]+(?:\/[^\s\/\x00-\x1f"'<>`|(]*)*)/g;
export const REL_PATH_RE = /(^|[\s([{"'`])((?:\.{1,2}[\\/]|[A-Za-z0-9_.-]+[\\/])(?:[^\s\x00-\x1f"'<>`|(]+[\\/])*[^\s\x00-\x1f"'<>`|(]+)/g;

// `Y/N` / `1/2` / `bash/zsh` 等を誤検出しないための post-filter。
// 受理条件: `./` `../` 始まり、またはセパレータ 2 個以上、または末尾拡張子あり。
export function isLikelyRelPath(path: string): boolean {
  if (!path) return false;
  if (/^\.{1,2}[\\/]/.test(path)) return true;
  const sepCount = (path.match(/[\\/]/g) || []).length;
  if (sepCount >= 2) return true;
  if (/\.[a-zA-Z0-9]{1,15}$/.test(path)) return true;
  return false;
}

export function isTerminalPathStartBoundary(text: string, start: number): boolean {
  if (start <= 0) return true;
  return /[\s([{"'`]/.test(text[start - 1] || '');
}

export function findPathCandidates(text: string): PathCandidate[] {
  const candidates: PathCandidate[] = [];
  for (const re of [ABS_WIN_PATH_RE, ABS_UNIX_PATH_RE]) {
    re.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(text)) !== null) {
      if (re === ABS_UNIX_PATH_RE && !isTerminalPathStartBoundary(text, m.index)) continue;
      const pathStr = trimTerminalPathCandidate(m[1]);
      if (pathStr.length >= 3) candidates.push({ start: m.index, end: m.index + pathStr.length, text: pathStr });
    }
  }
  REL_PATH_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = REL_PATH_RE.exec(text)) !== null) {
    const pathStr = trimTerminalPathCandidate(m[2]);
    if (pathStr.length < 3) continue;
    if (!isLikelyRelPath(pathStr)) continue;
    candidates.push({ start: m.index + m[1].length, end: m.index + m[1].length + pathStr.length, text: pathStr });
  }
  candidates.sort((a, b) => a.start - b.start || b.end - a.end);
  const out: PathCandidate[] = [];
  for (const c of candidates) {
    if (out.some((x) => c.start < x.end && c.end > x.start)) continue;
    out.push(c);
  }
  return out;
}

const PATH_CONT_LINE_RE = /^[A-Za-z0-9._+\-]+(?:[\\/][A-Za-z0-9._+\-]+)*(?:\.[A-Za-z0-9]{1,15})?$/;
const INCOMPLETE_PATH_END_RE = /[-_.\\/]$/;

function trimWrapLine(text: string): string {
  return String(text || '').replace(/^[ \t]+/, '').replace(/[ \t]+$/g, '');
}

function stripTrailingWrapPunct(text: string): string {
  return String(text || '').replace(/(?:[\s,;:'"`<>\])}])+$/g, '');
}

export function isPathContinuationText(text: string): boolean {
  const t = stripTrailingWrapPunct(trimWrapLine(text));
  if (!t) return false;
  if (/^[A-Za-z]:[\\/]/.test(t)) return false;
  if (t.startsWith('/')) return false;
  if (/^https?:/i.test(t)) return false;
  return PATH_CONT_LINE_RE.test(t);
}

export function looksLikePathWrapContinuation(
  prev: PathWrapRow,
  next: PathWrapRow,
  cols?: number,
): boolean {
  const nextText = trimWrapLine(next.text);
  if (!isPathContinuationText(nextText)) return false;
  const prevCore = stripTrailingWrapPunct(String(prev.text || '').replace(/[ \t]+$/g, ''));
  if (!prevCore) return false;
  const incomplete = INCOMPLETE_PATH_END_RE.test(prevCore);
  const fullWidth = cols != null && cols > 0 && prev.contentWidth >= cols;
  if (!incomplete && !fullWidth) return false;

  const candidates = findPathCandidates(prevCore);
  if (candidates.length === 0) {
    // 3 行以上に割れたときの中間行（例: "09-"）。パス先頭は持たない。
    return isPathContinuationText(prevCore);
  }
  const last = candidates[candidates.length - 1];
  return last.end >= prevCore.length;
}

export function expandLogicalPathLine(
  getRow: (index: number) => PathWrapRow | null,
  index: number,
  cols?: number,
): { start: number; end: number } {
  if (index < 0 || !getRow(index)) return { start: Math.max(0, index), end: Math.max(0, index) };

  let start = index;
  let hardBack = 0;
  while (start > 0) {
    const cur = getRow(start);
    const prev = getRow(start - 1);
    if (!cur || !prev) break;
    if (cur.isWrapped) {
      start -= 1;
      continue;
    }
    if (hardBack >= MAX_HARD_WRAP_EXTRA) break;
    if (looksLikePathWrapContinuation(prev, cur, cols)) {
      start -= 1;
      hardBack += 1;
      continue;
    }
    break;
  }

  let end = index;
  let hardFwd = 0;
  while (true) {
    const cur = getRow(end);
    const next = getRow(end + 1);
    if (!cur || !next) break;
    if (next.isWrapped) {
      end += 1;
      continue;
    }
    if (hardFwd >= MAX_HARD_WRAP_EXTRA) break;
    if (looksLikePathWrapContinuation(cur, next, cols)) {
      end += 1;
      hardFwd += 1;
      continue;
    }
    break;
  }
  return { start, end };
}

export function joinPathWrapRowTexts(rows: PathWrapRow[]): string {
  if (rows.length === 0) return '';
  let out = String(rows[0].text || '');
  for (let i = 1; i < rows.length; i++) {
    const row = rows[i];
    const text = row.isWrapped
      ? String(row.text || '')
      : String(row.text || '').replace(/^[ \t]+/, '');
    out += text;
  }
  return out;
}
