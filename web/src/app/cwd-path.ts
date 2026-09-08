/** cwd 文字列を「親ディレクトリ」「末尾セグメント（basename）」に分割する。
 *  区切りは \ と / の両対応。末尾が区切り文字の場合は手前のセグメントを basename とする。 */
export function splitCwdPath(value: string): { parent: string; basename: string } {
  const v = String(value);
  // 末尾の区切り文字は無視して basename 境界を探す。
  let end = v.length;
  while (end > 0 && (v[end - 1] === '/' || v[end - 1] === '\\')) end--;
  let start = end;
  while (start > 0 && v[start - 1] !== '/' && v[start - 1] !== '\\') start--;
  return { parent: v.slice(0, start), basename: v.slice(start, end) };
}

/** お気に入り表示順: 最終フォルダ名（basename）昇順。同名はフルパスで安定化。 */
export function compareCwdByBasename(a: string, b: string): number {
  const ba = splitCwdPath(a).basename;
  const bb = splitCwdPath(b).basename;
  return ba.localeCompare(bb) || a.localeCompare(b);
}

/** サブフォルダの 2 段ソート: お気に入りを先頭に、各グループ内は basename 昇順。 */
export function sortCwdSubdirItems(paths: string[], favSet: Set<string>): string[] {
  return paths.slice().sort((a, b) => {
    const af = favSet.has(a);
    const bf = favSet.has(b);
    if (af !== bf) return af ? -1 : 1;
    return compareCwdByBasename(a, b);
  });
}

/** cwd 入力欄の値を「親ディレクトリ」「区切り文字」「打ちかけの末尾セグメント」に分割する。
 *  区切り（\ と / の両対応）を 1 つも含まない値は null を返す。
 *  値が区切り文字で終わるときは partial を空文字にし、末尾の区切り文字（連続していても全部）
 *  を落とした残りを parent にする。終わらないときは最後の区切りの位置で分け、手前を parent、
 *  後ろを partial にする。
 *  ドライブ直下（`C:` → `C:\`）と POSIX ルート（'' → `/`）は、切り詰めた結果だけでは
 *  サーバ側 filepath.IsAbs の絶対パス判定を満たさないため、区切り文字を 1 つ残して補う。 */
export function splitCwdTypeahead(value: string): { parent: string; sep: string; partial: string } | null {
  const v = String(value);
  const isSep = (ch: string) => ch === '/' || ch === '\\';
  if (!v.includes('/') && !v.includes('\\')) return null;
  const sep = v.includes('\\') ? '\\' : '/';

  let parent: string;
  let partial: string;
  if (v.length > 0 && isSep(v[v.length - 1])) {
    let end = v.length;
    while (end > 0 && isSep(v[end - 1])) end--;
    parent = v.slice(0, end);
    partial = '';
  } else {
    let idx = -1;
    for (let i = v.length - 1; i >= 0; i--) {
      if (isSep(v[i])) { idx = i; break; }
    }
    parent = v.slice(0, idx);
    partial = v.slice(idx + 1);
  }

  if (/^[A-Za-z]:$/.test(parent)) {
    parent = parent + '\\';
  } else if (parent === '') {
    parent = '/';
  }

  return { parent, sep, partial };
}

/** サブフォルダのフルパス配列を、打ちかけ文字（partial）で絞り込んで並べる。
 *  partial が空文字なら sortCwdSubdirItems と完全に同じ結果を返す（不変条件）。
 *  一致判定はフォルダ名（basename）の小文字化に対する includes。親パス側は判定に使わない。
 *  並びは ①basename が partial に前方一致するものを部分一致より前に ②同ランク内はお気に入り
 *  を先に ③同グループ内は compareCwdByBasename、の 3 段。 */
export function filterCwdSubdirItems(paths: string[], favSet: Set<string>, partial: string): string[] {
  if (partial === '') return sortCwdSubdirItems(paths, favSet);
  const needle = partial.toLowerCase();
  return paths
    .filter(p => splitCwdPath(p).basename.toLowerCase().includes(needle))
    .sort((a, b) => {
      const ap = splitCwdPath(a).basename.toLowerCase().startsWith(needle);
      const bp = splitCwdPath(b).basename.toLowerCase().startsWith(needle);
      if (ap !== bp) return ap ? -1 : 1;
      const af = favSet.has(a);
      const bf = favSet.has(b);
      if (af !== bf) return af ? -1 : 1;
      return compareCwdByBasename(a, b);
    });
}

/** 親パスと子フォルダ名を連結する。親がすでに区切り文字で終わっている（ドライブ直下 `C:\` や
 *  POSIX ルート `/`）ときに区切りを二重に置かないためだけの関数。素朴に parent + sep + name と
 *  書くと `C:\` + `\` + `x` が `C:\\x` になり、表示も、お気に入り判定の完全一致も崩れる。 */
export function joinCwdChild(parent: string, sep: string, name: string): string {
  const last = parent.slice(-1);
  if (last === '/' || last === '\\') return parent + name;
  return parent + sep + name;
}
