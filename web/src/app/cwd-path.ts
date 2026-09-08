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
