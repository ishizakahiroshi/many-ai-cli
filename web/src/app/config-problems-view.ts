// config-problems-view.ts — 設定ファイル（config.yaml）の誤りを画面に出すための「表示内容」を作る純関数。
//
// Hub は custom_providers の 1 件が誤っていても止まらず、その 1 件だけを外して起動する。
// 外したことは /api/info の config_problems で届く。ここではそれを、画面に出す文言の並びに
// 変換するだけ（DOM にも通信にも触らない。fixtures でそのまま検査する）。
//
// 文言は Hub が返した「構造化された値」（code / index / id / field …）から i18n で作る。
// Hub の英文（reason / effect）は、この画面が知らない code が来たとき（新しい Hub）の代わりにだけ使う。
// config.yaml の行の中身や token は届かない（Hub 側で載せていない）。

export interface ConfigProblemInfo {
  // 'custom_providers'（1 件ごと）か 'config'（ファイル全体）。Hub が返さない古い応答は custom_providers 扱い。
  section: string;
  index: number | null;
  id: string | null;
  field: string;
  code: string;
  dropped: boolean;
  related_index: number | null;
  reason: string;
  effect: string;
  example: string;
  fix_command: string | null;
}

// Hub の保存が config.yaml から外した項目（読み込めなかったもの）。元の中身はバックアップにある。
// 利用者が restore するか「確認した」と記録するまで、再起動しても出し続ける。
// kind: 'removed-by-save'（1 件）/ 'removed-more'（「ほか N 件」の要約）/ 'record-unreadable'
// （知らせの記録が読めない。読めない記録は「何も無い」とは扱わない）。
export type RemovedKind = 'removed-by-save' | 'removed-more' | 'record-unreadable';
export interface RemovedItemInfo extends ConfigProblemInfo {
  kind: RemovedKind;
  backup_id: string;
  more: number;
  record_file: string;
  restore_command: string;
  dismiss_command: string;
}

export interface ConfigProblemsInfo {
  status: 'ok' | 'problems' | 'unreadable' | 'unavailable';
  problems: ConfigProblemInfo[];
  removed: RemovedItemInfo[];
  restartNeeded: boolean;
  checkCommand: string;
  unreadable: { line: number | null; column: number | null; resetCommand: string } | null;
}

export type Translate = (key: string, vars?: Record<string, unknown>) => string;

export interface ProblemItemView {
  heading: string;
  field: string;
  reason: string;
  effect: string;
  example: string;
  fixCommand: string | null;
}

export interface ConfigBannerView {
  kind: 'problems' | 'fixed' | 'unreadable';
  title: string;
  summary: string;
  items: ProblemItemView[];
  notes: string[];
  checkCommand: string;
}

// 画面が文言を持つ code。Hub の ProblemCode（kebab-case）と同じ並び。
export const KNOWN_PROBLEM_CODES = [
  'not-a-list',
  'not-a-mapping',
  'incompatible-type',
  'missing-id',
  'missing-command',
  'id-too-long',
  'id-invalid-shape',
  'reserved-id',
  'duplicate-id',
  'command-unterminated-quote',
  'command-control-character',
  'headless-format',
  'headless-prompt-via',
  'headless-args',
  'extra-documents',
] as const;

const asString = (value: unknown): string => (typeof value === 'string' ? value : '');

// id は利用者が書いた文字列。向きを変える・見えない書式制御文字（U+202E など）は表示に出さない。
// Hub 側（safety.rs の display_id）でも除くが、画面でも除いて二重にする。
// 範囲は Unicode の Cf に行・段落区切りを足したもの（コードポイントの数値で持つ）。
const HIDDEN_RANGES: ReadonlyArray<readonly [number, number]> = [
  [0x0000, 0x001f], [0x007f, 0x009f], [0x00ad, 0x00ad], [0x0600, 0x0605], [0x061c, 0x061c],
  [0x06dd, 0x06dd], [0x070f, 0x070f], [0x0890, 0x0891], [0x08e2, 0x08e2], [0x180e, 0x180e],
  [0x200b, 0x200f], [0x2028, 0x202e], [0x2060, 0x206f], [0xfeff, 0xfeff], [0xfff9, 0xfffb],
  [0x110bd, 0x110bd], [0x110cd, 0x110cd], [0x13430, 0x1343f], [0x1bca0, 0x1bca3],
  [0x1d173, 0x1d17a], [0xe0001, 0xe0001], [0xe0020, 0xe007f],
];
export function visibleText(value: string): string {
  let out = '';
  for (const ch of value) {
    const code = ch.codePointAt(0) ?? 0;
    if (!HIDDEN_RANGES.some(([from, to]) => code >= from && code <= to)) out += ch;
  }
  return out;
}
const asIndex = (value: unknown): number | null =>
  typeof value === 'number' && Number.isInteger(value) && value > 0 ? value : null;

// /api/info の応答から config_problems を取り出す。欄が無い（古い Hub）・形が違うときは null
// （= 何も出さない）。
export function parseConfigProblems(raw: unknown): ConfigProblemsInfo | null {
  if (!raw || typeof raw !== 'object') return null;
  const value = raw as Record<string, unknown>;
  const status = value.status;
  if (status !== 'ok' && status !== 'problems' && status !== 'unreadable' && status !== 'unavailable') {
    return null;
  }
  const toProblem = (item: Record<string, unknown>): ConfigProblemInfo => ({
    section: asString(item.section) || 'custom_providers',
    index: asIndex(item.index),
    id: visibleText(asString(item.id)) || null,
    field: asString(item.field),
    code: asString(item.code),
    dropped: item.dropped === true,
    related_index: asIndex(item.related_index),
    reason: asString(item.reason),
    effect: asString(item.effect),
    example: asString(item.example),
    fix_command: asString(item.fix_command) || null,
  });
  const objects = (raw: unknown): Record<string, unknown>[] =>
    (Array.isArray(raw) ? raw : []).filter(
      (item): item is Record<string, unknown> => !!item && typeof item === 'object',
    );
  const problems: ConfigProblemInfo[] = objects(value.problems).map(toProblem);
  const removedKind = (raw: unknown): RemovedKind =>
    raw === 'removed-more' || raw === 'record-unreadable' ? raw : 'removed-by-save';
  const removed: RemovedItemInfo[] = objects(value.removed_by_save).map((item) => ({
    ...toProblem(item),
    dropped: true,
    kind: removedKind(item.kind),
    backup_id: asString(item.backup_id),
    more: typeof item.more === 'number' && item.more > 0 ? Math.floor(item.more) : 0,
    record_file: visibleText(asString(item.record_file)),
    restore_command: asString(item.restore_command),
    dismiss_command: asString(item.dismiss_command) || 'many-ai-cli config check --dismiss-removed',
  }));
  const rawUnreadable = value.unreadable;
  const unreadable =
    rawUnreadable && typeof rawUnreadable === 'object'
      ? {
          line: asIndex((rawUnreadable as Record<string, unknown>).line),
          column: asIndex((rawUnreadable as Record<string, unknown>).column),
          resetCommand: asString((rawUnreadable as Record<string, unknown>).reset_command),
        }
      : null;
  return {
    status,
    problems,
    removed,
    restartNeeded: value.restart_needed === true,
    checkCommand: asString(value.check_command) || 'many-ai-cli config check',
    unreadable,
  };
}

// 辞書に無い key は t() が key そのものを返す。その場合は fallback（英文）を使う。
function say(t: Translate, key: string, fallback: string, vars?: Record<string, unknown>): string {
  const text = t(key, vars);
  return text && text !== key ? text : fallback;
}

export function problemKey(code: string, part: 'reason' | 'effect'): string {
  return `config_problem_${part}_${code.replaceAll('-', '_')}`;
}

function entryLabel(problem: ConfigProblemInfo, t: Translate): string {
  if (problem.section !== 'custom_providers') return say(t, 'config_problem_entry_file', 'config.yaml (the whole file)');
  if (problem.index === null) return say(t, 'config_problem_entry_list', 'custom_providers (whole list)');
  const vars = { n: problem.index, id: problem.id ?? '' };
  return problem.id
    ? say(t, 'config_problem_entry_with_id', `custom_providers entry #${vars.n} (id: ${vars.id})`, vars)
    : say(t, 'config_problem_entry', `custom_providers entry #${vars.n}`, vars);
}

function effectText(problem: ConfigProblemInfo, t: Translate): string {
  const vars = {
    id: problem.id ?? '',
    n: problem.index ?? '',
    related: problem.related_index ?? '',
  };
  const known = (KNOWN_PROBLEM_CODES as readonly string[]).includes(problem.code);
  if (!known) return problem.effect;
  if (problem.code === 'extra-documents') {
    return say(t, 'config_problem_effect_ignored', problem.effect, vars);
  }
  if (!problem.dropped) {
    return say(t, 'config_problem_effect_reduced', problem.effect, vars);
  }
  if (problem.code === 'duplicate-id') {
    return say(t, 'config_problem_effect_duplicate', problem.effect, vars);
  }
  if (problem.code === 'not-a-list') {
    return say(t, 'config_problem_effect_none_loaded', problem.effect, vars);
  }
  // 同じ id の別の 1 件が使われているとき、どれが使われているかを言う。
  if (
    problem.related_index !== null &&
    (problem.code === 'command-unterminated-quote' || problem.code === 'command-control-character')
  ) {
    return say(t, 'config_problem_effect_replaced', problem.effect, vars);
  }
  return say(t, 'config_problem_effect_dropped', problem.effect, vars);
}

export function problemItem(problem: ConfigProblemInfo, t: Translate): ProblemItemView {
  const known = (KNOWN_PROBLEM_CODES as readonly string[]).includes(problem.code);
  const vars = {
    id: problem.id ?? '',
    field: problem.field,
    n: problem.index ?? '',
    related: problem.related_index ?? '',
  };
  return {
    heading: entryLabel(problem, t),
    field: problem.field,
    reason: known ? say(t, problemKey(problem.code, 'reason'), problem.reason, vars) : problem.reason,
    effect: effectText(problem, t),
    example: problem.example,
    fixCommand: problem.fix_command,
  };
}

// Hub の保存が外した項目: 何件目・なぜ読めなかったか・元はどのバックアップか・戻し方。
export function removedItem(item: RemovedItemInfo, t: Translate): ProblemItemView {
  if (item.kind === 'removed-more') {
    const vars = { count: item.more, backup: item.backup_id };
    return {
      heading: say(t, 'config_problem_removed_more_heading', `${item.more} more item(s) taken out by the same save`, vars),
      field: '',
      reason: say(
        t,
        'config_problem_removed_more_reason',
        `Only the first items are listed. The rest are in the backup ${item.backup_id} too.`,
        vars,
      ),
      effect: say(
        t,
        'config_problem_effect_removed',
        `A save by the Hub took this out of config.yaml because it could not be loaded. The original is in the backup ${item.backup_id}.`,
        vars,
      ),
      example: '',
      fixCommand: item.restore_command || null,
    };
  }
  if (item.kind === 'record-unreadable') {
    const vars = { file: item.record_file };
    return {
      heading: say(t, 'config_problem_record_unreadable_heading', `The notice record ${item.record_file} cannot be read`, vars),
      field: '',
      // 理由は Hub の固定の英文（ファイルの中身は含まない）。
      reason: item.reason,
      effect: say(
        t,
        'config_problem_record_unreadable_effect',
        'What a save by the Hub took out of config.yaml cannot be listed from this record. The record was not changed or deleted, and the backups are kept.',
        vars,
      ),
      example: '',
      fixCommand: item.dismiss_command || null,
    };
  }
  const view = problemItem(item, t);
  const vars = { backup: item.backup_id };
  return {
    ...view,
    effect: say(
      t,
      'config_problem_effect_removed',
      `A save by the Hub took this out of config.yaml because it could not be loaded. The original is in the backup ${item.backup_id}.`,
      vars,
    ),
    example: '',
    fixCommand: item.restore_command || null,
  };
}

function position(line: number | null, column: number | null, t: Translate): string {
  if (line !== null && column !== null) {
    return say(t, 'config_problems_position_line_column', `line ${line}, column ${column}`, { line, column });
  }
  if (line !== null) return say(t, 'config_problems_position_line', `line ${line}`, { line });
  return '';
}

// 画面に出す内容。何も出さないとき（誤りが無く、再起動待ちでもない）は null。
export function buildConfigBanner(info: ConfigProblemsInfo | null, t: Translate): ConfigBannerView | null {
  if (!info) return null;
  const checkCommand = info.checkCommand;
  if (info.status === 'unreadable' && info.unreadable) {
    const at = position(info.unreadable.line, info.unreadable.column, t);
    const title = at
      ? say(t, 'config_problems_unreadable_at', `The settings file cannot be read (${at}).`, { at })
      : say(t, 'config_problems_unreadable', 'The settings file cannot be read.');
    return {
      kind: 'unreadable',
      title,
      summary: say(
        t,
        'config_problems_unreadable_effect',
        'Because the settings file cannot be read, no new session can be started. Sessions that are already running are not affected. Fix the file, or start over from the defaults (the current file is kept as a backup).',
      ),
      items: [],
      notes: [
        say(t, 'config_problems_unreadable_reset', `To start over: ${info.unreadable.resetCommand}`, {
          command: info.unreadable.resetCommand,
        }),
      ],
      checkCommand,
    };
  }
  if (info.status === 'problems' && info.problems.length + info.removed.length > 0) {
    const count = info.problems.length + info.removed.length;
    const notes = [] as string[];
    if (info.removed.length > 0) {
      const command = info.removed[0].dismiss_command;
      notes.push(
        say(
          t,
          'config_problems_removed_note',
          `Items a save by the Hub took out of the file stay listed until you bring the backup back or say you have seen them: ${command}`,
          { command },
        ),
      );
    }
    // 知らせが立っている間は「ファイルで直した分は再起動後に使われる」と言わない
    // （Hub の保存が外したものは、ファイルで直したものではない）。
    if (info.restartNeeded && info.removed.length === 0) {
      notes.push(
        say(t, 'config_problems_restart_note', 'What you fixed in the file is used after the Hub is restarted.'),
      );
    }
    return {
      kind: 'problems',
      title: say(t, 'config_problems_title', `${count} mistake(s) in the settings`, { count }),
      summary: say(
        t,
        'config_problems_summary',
        'Only the affected part was left out and the Hub started. Open this to see which one and how to fix it.',
      ),
      items: [
        ...info.problems.map((problem) => problemItem(problem, t)),
        ...info.removed.map((item) => removedItem(item, t)),
      ],
      notes,
      checkCommand,
    };
  }
  // 直したが、動いている Hub はまだ起動時の設定のまま。
  if (info.status === 'ok' && info.restartNeeded && info.removed.length === 0) {
    return {
      kind: 'fixed',
      title: say(t, 'config_problems_fixed_title', 'The settings mistake is fixed in the file.'),
      summary: say(t, 'config_problems_fixed_restart', 'Restart the Hub to use the fixed settings.'),
      items: [],
      notes: [],
      checkCommand,
    };
  }
  return null;
}
