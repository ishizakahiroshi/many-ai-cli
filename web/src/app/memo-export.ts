// 作業メモの「書き出し／取り込み」の純粋ロジック。DOM に触れない。
// 整形（Markdown / テキスト / JSON）・書き出す対象の選別・取り込みの解析・重複判定だけをここに置き、
// memo-panel.ts がボタンやダイアログから呼ぶ。
// import してよいのは memo-model.js だけ（path-links.ts / util.ts は window に触れるので、
// bun:test から読むと落ちる。memo-model.ts の冒頭コメント参照）。
import { groupMemosByProject, sortDoneMemos, sortOpenMemos } from './memo-model.js';
import type { Memo } from './memo-model.js';

export type MemoExportFormat = 'markdown' | 'text' | 'json';
export const MEMO_EXPORT_FORMATS: readonly MemoExportFormat[] = ['markdown', 'text', 'json'];

/** 出力に埋める表記。画面の言語に合わせるため呼び出し側（t()）から渡す。 */
export interface MemoExportLabels {
  /** 文書の見出し。例: 作業メモ（2026-10-20 出力・3 件） */
  title: (count: number, date: string) => string;
  /** project が空のグループ名。 */
  unclassified: string;
  /** 例: （画像 2 枚） */
  images: (count: number) => string;
  /** text 形式で完了メモに付ける印。例: 完了 */
  doneMark: string;
}

/**
 * 書き出すメモを選ぶ。selectedIds が空でなければ、その id のメモだけ（完了済みでも含める）。
 * 空なら includeDone ? 全件 : 未完了のみ。元の配列の順を保つ。
 */
export function selectMemosForExport(
  memos: readonly Memo[],
  selectedIds: ReadonlySet<string>,
  includeDone: boolean,
): Memo[] {
  if (selectedIds.size > 0) return memos.filter((m) => selectedIds.has(m.id));
  return memos.filter((m) => includeDone || !m.done);
}

export interface MemoExportResult {
  content: string;
  /** 書き出したメモの件数。 */
  count: number;
  mime: string;
  extension: 'md' | 'txt' | 'json';
}

const FORMAT_META: Record<MemoExportFormat, { mime: string; extension: MemoExportResult['extension'] }> = {
  markdown: { mime: 'text/markdown;charset=utf-8', extension: 'md' },
  text: { mime: 'text/plain;charset=utf-8', extension: 'txt' },
  json: { mime: 'application/json;charset=utf-8', extension: 'json' },
};

const pad2 = (n: number): string => String(n).padStart(2, '0');

// toLocaleString() はブラウザ依存なので使わず、年月日・時刻は自前で 2 桁詰めする。
function formatDate(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

/** ローカルオフセット付きの ISO 8601（例: 2026-10-20T12:00:00+09:00）。 */
function formatLocalIso(d: Date): string {
  const offset = -d.getTimezoneOffset();
  const sign = offset >= 0 ? '+' : '-';
  const abs = Math.abs(offset);
  return (
    `${formatDate(d)}T${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}` +
    `${sign}${pad2(Math.floor(abs / 60))}:${pad2(abs % 60)}`
  );
}

/** project 末尾のフォルダ名。\ と / の両方で分割し、末尾の区切りは無視する。 */
function folderName(project: string): string {
  const parts = project.split(/[\\/]/).filter((p) => p !== '');
  return parts.length > 0 ? parts[parts.length - 1] : project;
}

/** 1 件のメモを行に展開する。2 行目以降は 2 スペースで字下げ（空行は字下げなし）。 */
function memoLines(memo: Memo, marker: string, labels: MemoExportLabels): string[] {
  const body = memo.text.replace(/\s+$/, '');
  const imageCount = memo.images?.length ?? 0;
  const lines: string[] = [];
  if (body === '') {
    // 本文が空で画像だけのメモは、1 行目が画像の表記になる。
    lines.push(marker + (imageCount > 0 ? labels.images(imageCount) : ''));
    return lines;
  }
  body.split(/\r?\n/).forEach((line, i) => {
    if (i === 0) lines.push(marker + line);
    else lines.push(line === '' ? '' : `  ${line}`);
  });
  if (imageCount > 0) lines.push(`  ${labels.images(imageCount)}`);
  return lines;
}

function buildReadable(
  memos: readonly Memo[],
  format: 'markdown' | 'text',
  labels: MemoExportLabels,
  activeProject: string,
  title: string,
): string {
  const blocks: string[] = [format === 'markdown' ? `# ${title}` : title];
  for (const group of groupMemosByProject(memos, activeProject)) {
    const name = group.project === '' ? labels.unclassified : folderName(group.project);
    const lines: string[] = [format === 'markdown' ? `## ${name}` : `■ ${name}`];
    const ordered = [...sortOpenMemos(group.open), ...sortDoneMemos(group.done)];
    for (const memo of ordered) {
      let marker: string;
      if (format === 'markdown') marker = memo.done ? '- [x] ' : '- [ ] ';
      else marker = memo.done ? `・[${labels.doneMark}] ` : '・';
      lines.push(...memoLines(memo, marker, labels));
    }
    blocks.push(lines.join('\n'));
  }
  return blocks.join('\n\n') + '\n';
}

function buildJson(memos: readonly Memo[], now: Date): string {
  // 取り込み直したとき作成順が保たれるよう、作成日時の昇順（同時刻は元の順）。
  const sorted = memos
    .map((m, i) => ({ m, i }))
    .sort((a, b) => a.m.created_at.localeCompare(b.m.created_at) || a.i - b.i)
    .map((x) => x.m);
  const out = sorted.map((m) => {
    const item: Record<string, unknown> = { project: m.project, text: m.text, done: m.done };
    if (m.done && m.done_at) item.done_at = m.done_at;
    item.created_at = m.created_at;
    item.updated_at = m.updated_at;
    // 画像の実体は出さない。名前だけ。id も出さない（取り込み側で採番し直す）。
    item.images = [...(m.images ?? [])];
    return item;
  });
  return JSON.stringify({ version: 1, exported_at: formatLocalIso(now), memos: out }, null, 2) + '\n';
}

export function buildMemoExport(
  memos: readonly Memo[],
  format: MemoExportFormat,
  labels: MemoExportLabels,
  opts?: { activeProject?: string; now?: Date },
): MemoExportResult {
  const now = opts?.now ?? new Date();
  const meta = FORMAT_META[format];
  const content =
    format === 'json'
      ? buildJson(memos, now)
      : buildReadable(memos, format, labels, opts?.activeProject ?? '', labels.title(memos.length, formatDate(now)));
  return { content, count: memos.length, mime: meta.mime, extension: meta.extension };
}

/** 例: memos_20261020_121530.md */
export function memoExportFileName(format: MemoExportFormat, now: Date = new Date()): string {
  const stamp =
    `${now.getFullYear()}${pad2(now.getMonth() + 1)}${pad2(now.getDate())}` +
    `_${pad2(now.getHours())}${pad2(now.getMinutes())}${pad2(now.getSeconds())}`;
  return `memos_${stamp}.${FORMAT_META[format].extension}`;
}

// ── 取り込み ───────────────────────────────────────────────────────────

export interface ImportItem {
  text: string;
  project: string;
  done: boolean;
}

export type MemoImportParse =
  | { ok: true; items: ImportItem[]; skippedEmpty: number }
  | { ok: false; reason: 'empty' | 'invalid_json' | 'wrong_shape' };

/**
 * 書き出した JSON（{ memos: [...] }、または配列そのもの）を取り込み候補に解析する。
 * 本文が空の要素（object でない・text が文字列でないものを含む）は skippedEmpty に数えて捨てる。
 * メモを 1 件も取れなくても ok:true（items=[]）で返す。
 */
export function parseMemoImport(raw: string): MemoImportParse {
  const trimmed = raw.trim();
  if (trimmed === '') return { ok: false, reason: 'empty' };
  let data: unknown;
  try {
    data = JSON.parse(trimmed);
  } catch {
    return { ok: false, reason: 'invalid_json' };
  }
  let list: unknown;
  if (Array.isArray(data)) list = data;
  else if (data !== null && typeof data === 'object') list = (data as { memos?: unknown }).memos;
  if (!Array.isArray(list)) return { ok: false, reason: 'wrong_shape' };

  const items: ImportItem[] = [];
  let skippedEmpty = 0;
  for (const entry of list as unknown[]) {
    if (entry === null || typeof entry !== 'object' || Array.isArray(entry)) {
      skippedEmpty++;
      continue;
    }
    const e = entry as { text?: unknown; project?: unknown; done?: unknown };
    const text = typeof e.text === 'string' ? e.text.trim() : '';
    if (text === '') {
      skippedEmpty++;
      continue;
    }
    items.push({
      text,
      project: typeof e.project === 'string' ? e.project : '',
      done: typeof e.done === 'boolean' ? e.done : false,
    });
  }
  return { ok: true, items, skippedEmpty };
}

export interface ImportTarget {
  /** 取り込み先のセッション（Hub が project を決める）。無ければ null = 未分類。 */
  sessionId: number | null;
  /** 既存メモの project が、この取り込み先に属するか。 */
  belongs: (memoProject: string) => boolean;
}

export interface ImportPlan {
  create: { item: ImportItem; sessionId: number | null }[];
  duplicates: number;
}

/**
 * 取り込み候補から、実際に作るものと重複（作らないもの）を決める。
 * 重複 = 本文（trim 後）が同じで、既存メモが同じ取り込み先に属する（target.belongs）もの。
 * 今回の候補同士も、本文と sessionId が同じなら最初の 1 件だけ作る。完了済みの既存メモも対象。
 */
export function planMemoImport(
  items: readonly ImportItem[],
  existing: readonly Memo[],
  resolveTarget: (item: ImportItem) => ImportTarget,
): ImportPlan {
  const create: ImportPlan['create'] = [];
  const planned: { text: string; sessionId: number | null }[] = [];
  let duplicates = 0;
  for (const item of items) {
    const text = item.text.trim();
    const target = resolveTarget(item);
    const dupExisting = existing.some((m) => m.text.trim() === text && target.belongs(m.project));
    const dupPlanned = planned.some((p) => p.text === text && p.sessionId === target.sessionId);
    if (dupExisting || dupPlanned) {
      duplicates++;
      continue;
    }
    planned.push({ text, sessionId: target.sessionId });
    create.push({ item, sessionId: target.sessionId });
  }
  return { create, duplicates };
}
