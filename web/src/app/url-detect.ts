// テキスト中の URL 検出。DOM / xterm に依存しない純関数だけを置く。
// クリック時のメニューや DOM へのリンク化は url-links.ts。
// 端末・ファイルプレビュー・チャット履歴・Grok 履歴・展開ポップアップは、URL の判定をすべてここへ寄せる
// （画面ごとに別の正規表現を持つと、同じ URL が画面によって違う範囲でリンクになる）。

import { looksLikePathWrapContinuation, type PathWrapRow } from './path-detect.js';

export type UrlCandidate = {
  start: number;
  // 検出した URL の直後の位置（text.slice(start, end) === url）
  end: number;
  url: string;
};

// URL として受け取る文字は ASCII に限る。日本語の文中では「https://example.com。次の文」
// 「（https://example.com）」のように URL の直後へ空白なしで日本語や全角記号が続くことが多く、
// 非 ASCII まで URL に含めると開いた先が 404 になる。代わりに、パスへ日本語をそのまま含む
// URL（例: ja.wikipedia.org/wiki/日本）は日本語の手前で切れる。
// ' は RFC 上は使えるが、'https://…' の囲みを URL に含めないため外す。
const URL_BODY_CHARS = "A-Za-z0-9\\-._~:/?#\\[\\]@!$&()*+,;=%";
const URL_RE = new RegExp(`https?:\\/\\/[${URL_BODY_CHARS}]+`, 'gi');
const URL_ONLY_RE = new RegExp(`^[${URL_BODY_CHARS}]+$`);
const URL_AT_END_RE = new RegExp(`https?:\\/\\/[${URL_BODY_CHARS}]+$`, 'i');

// 文末の句読点・強調記号は URL に含めない。
const TRAILING_PUNCT_RE = /[.,;:!?*]$/;
// CLI が折り返した行末に来やすく、URL がそこで終わるとは考えにくい文字。
const INCOMPLETE_URL_END_RE = /[-_=&]$/;

function countChar(text: string, ch: string): number {
  let n = 0;
  for (const c of text) if (c === ch) n++;
  return n;
}

// 末尾の句読点と、対応の取れない閉じ括弧を落とす。
// 「(https://example.com)」の ) は落とし、「https://en.wikipedia.org/wiki/Foo_(bar)」の ) は残す。
export function trimUrlCandidate(raw: string): string {
  let url = String(raw || '');
  for (;;) {
    if (TRAILING_PUNCT_RE.test(url)) {
      url = url.slice(0, -1);
      continue;
    }
    if (url.endsWith(')') && countChar(url, '(') < countChar(url, ')')) {
      url = url.slice(0, -1);
      continue;
    }
    if (url.endsWith(']') && countChar(url, '[') < countChar(url, ']')) {
      url = url.slice(0, -1);
      continue;
    }
    return url;
  }
}

export function isHttpUrl(url: string): boolean {
  return /^https?:\/\/[A-Za-z0-9[]/i.test(String(url || ''));
}

export function findUrlCandidates(text: string): UrlCandidate[] {
  const src = String(text || '');
  const out: UrlCandidate[] = [];
  URL_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = URL_RE.exec(src)) !== null) {
    // 「xhttps://」のような語の途中からは拾わない
    if (m.index > 0 && /[A-Za-z0-9]/.test(src[m.index - 1] || '')) continue;
    const url = trimUrlCandidate(m[0]);
    if (!isHttpUrl(url)) continue;
    out.push({ start: m.index, end: m.index + url.length, url });
  }
  return out;
}

export type UrlPiece = { from: number; to: number; url: string };

// 連続した文字列が複数の断片（ハイライトで割れたテキストノードなど）に分かれているとき、
// 断片をつないだ 1 本の文字列で URL を探し、断片ごとに「どこからどこまでがどの URL か」を返す。
// 戻り値の i 番目は values[i] の中の位置。断片をまたぐ URL は、またいだ断片それぞれに 1 つずつ出る。
export function splitUrlPieces(values: string[]): UrlPiece[][] {
  const starts: number[] = [];
  let combined = '';
  for (const v of values) {
    starts.push(combined.length);
    combined += v;
  }
  const out: UrlPiece[][] = values.map(() => []);
  const candidates = findUrlCandidates(combined);
  let ci = 0;
  for (let i = 0; i < values.length && ci < candidates.length; i++) {
    const nodeStart = starts[i];
    const nodeEnd = nodeStart + values[i].length;
    for (let k = ci; k < candidates.length; k++) {
      const c = candidates[k];
      if (c.start >= nodeEnd) break;
      if (c.end <= nodeStart) continue;
      out[i].push({ from: Math.max(c.start, nodeStart) - nodeStart, to: Math.min(c.end, nodeEnd) - nodeStart, url: c.url });
    }
    while (ci < candidates.length && candidates[ci].end <= nodeEnd) ci++;
  }
  return out;
}

// CLI が自分で入れた改行で URL が割れたかどうか。xterm の isWrapped では繋がらない分を拾う。
// 条件はパスの looksLikePathWrapContinuation と同じ形で、前の行が URL（または URL の途中）で終わり、
// 行幅いっぱいか途中で切れた形をしていて、次の行が URL の文字だけでできているとき。
export function looksLikeUrlWrapContinuation(
  prev: PathWrapRow,
  next: PathWrapRow,
  cols?: number,
): boolean {
  const nextText = String(next.text || '').trim();
  if (!nextText || !URL_ONLY_RE.test(nextText)) return false;
  if (/^https?:/i.test(nextText)) return false;
  const prevCore = String(prev.text || '').replace(/\s+$/, '');
  if (!prevCore) return false;
  const incomplete = INCOMPLETE_URL_END_RE.test(prevCore);
  const fullWidth = cols != null && cols > 0 && prev.contentWidth >= cols;
  if (!incomplete && !fullWidth) return false;
  if (URL_AT_END_RE.test(prevCore)) return true;
  // 3 行以上に割れたときの中間行。URL の先頭は持たない。
  return URL_ONLY_RE.test(prevCore.trim()) && !/^https?:/i.test(prevCore.trim());
}

// 端末のリンク検出で使う、パスと URL の両方の折り返し判定。
export function looksLikeLinkWrapContinuation(
  prev: PathWrapRow,
  next: PathWrapRow,
  cols?: number,
): boolean {
  return looksLikePathWrapContinuation(prev, next, cols) || looksLikeUrlWrapContinuation(prev, next, cols);
}
