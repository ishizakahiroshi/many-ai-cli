// Hub が送る「provider ごとの宣言」の画面側の写し。
//
// 表示名・頭文字・並び順・使用量ページの URL・許可モードの選択肢・入力の癖など、
// これまで画面のあちこちに provider ごとの固定表で持っていた値の置き場。
// 同梱 7 本も、登録した AI CLI も、同じ経路（GET /api/providers/declarations）で届く。
// 画面は起動時に同梱 7 本の種（provider-declarations-seed.ts）を置き、Hub の答えで入れ替える。
// 何も入っていないとき（単体のテストなど）は、各関数は id から作れる控えめな既定を返す。
//
// 色は CSS の変数（--prov-<id>）と、利用者が設定した色（provider-icon.ts）の持ち分。ここには持たない。

export type ProviderDeclaration = {
  id: string;
  display_name?: string;
  enabled?: boolean;
  // 同梱の 7 本か（登録した AI CLI は false）。
  builtin?: boolean;
  // 'pty'（既定）か 'acp'。acp は人が直接操作する接続で、自動の経路からは使えない。
  connection?: string;
  presentation?: {
    icon_text?: string;
    color?: string;
    letter?: string;
    order?: number;
    handoff_order?: number;
    label?: string;
    long_label?: string;
  };
  slash_commands?: { source?: string; skill_prefix?: string; items?: { cmd: string; desc?: string }[] };
  permissions?: { modes?: Record<string, string[]>; allowed_tools_args?: string[]; choices?: string[] };
  links?: { usage_url?: string };
  input?: {
    slow_paste?: boolean;
    interrupt_key?: string;
    attach_format?: string;
    slash_submit?: string;
    full_frame_cursor?: boolean;
    no_line_clear?: boolean;
  };
  routines?: { enabled?: boolean; completion_mode?: string };
  context_limit?: number;
  // 宣言から Hub が計算した「できること／できないこと」（provider-manager-view.ts の capabilityView）。
  capability_report?: { key: string; state: string; detail?: string }[];
};

const declarations = new Map<string, ProviderDeclaration>();
let listeners: Array<() => void> = [];

function isDeclaration(value: unknown): value is ProviderDeclaration {
  return !!value && typeof value === 'object' && typeof (value as { id?: unknown }).id === 'string' && !!(value as { id: string }).id;
}

// Hub の答え（declarations の配列）で丸ごと入れ替える。
export function cacheProviderDeclarations(list: unknown): void {
  declarations.clear();
  if (Array.isArray(list)) {
    for (const entry of list) {
      if (isDeclaration(entry)) declarations.set(entry.id, entry);
    }
  }
  for (const listener of listeners) listener();
}

export function clearProviderDeclarations(): void {
  cacheProviderDeclarations([]);
}

// 宣言が入れ替わったあとに、画面の再描画などを呼ぶ。戻り値は解除の関数。
export function onProviderDeclarationsChanged(listener: () => void): () => void {
  listeners.push(listener);
  return () => {
    listeners = listeners.filter((entry) => entry !== listener);
  };
}

export function providerDeclaration(id: unknown): ProviderDeclaration | undefined {
  return typeof id === 'string' ? declarations.get(id) : undefined;
}

export function hasProviderDeclarations(): boolean {
  return declarations.size > 0;
}

// 並び順が小さい順（0 や未指定は最後）。同じ順位では Hub が送った順のまま。
export function declaredProviderIds(options?: {
  includeDisabled?: boolean;
  includeAcp?: boolean;
  // 'hub'（Hub が送った順のまま）か 'order'（presentation.order の順・既定）。
  sort?: 'hub' | 'order';
}): string[] {
  const list = Array.from(declarations.values()).filter((entry) => (
    (options?.includeDisabled || entry.enabled !== false)
    && (options?.includeAcp !== false || entry.connection !== 'acp')
  ));
  const rank = (entry: ProviderDeclaration): number => {
    const order = entry.presentation?.order;
    return typeof order === 'number' && order > 0 ? order : Number.MAX_SAFE_INTEGER;
  };
  if (options?.sort === 'hub') return list.map((entry) => entry.id);
  return list
    .map((entry, index) => ({ id: entry.id, rank: rank(entry), index }))
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .map((entry) => entry.id);
}

// 一覧・バッジ・カードに出す短い名前。宣言が無ければ id。
export function providerShortLabel(id: unknown): string {
  const declaration = providerDeclaration(id);
  return declaration?.presentation?.label || declaration?.display_name || String(id || '');
}

// 起動の選択肢・役割の一覧に出す正式な名前。宣言が無ければ id。
export function providerLongLabel(id: unknown): string {
  const declaration = providerDeclaration(id);
  return declaration?.presentation?.long_label
    || declaration?.display_name
    || declaration?.presentation?.label
    || String(id || '');
}

// バッジの頭文字。宣言が無ければ id の先頭 1 文字（大文字）。
export function providerBadgeLetter(id: unknown): string {
  const declared = providerDeclaration(id)?.presentation?.letter;
  if (declared) return declared;
  const source = String(id || '?').trim();
  return (Array.from(source)[0] || '?').toUpperCase();
}

// 並び順（小さい順）。宣言が無い・指定が無いものは 99。
export function providerSortOrder(id: unknown): number {
  const order = providerDeclaration(id)?.presentation?.order;
  return typeof order === 'number' && order > 0 ? order : 99;
}

export function providerUsageUrl(id: unknown): string {
  return providerDeclaration(id)?.links?.usage_url || '';
}

// 起動パネルが出す許可モードの選択肢（順番どおり）。空なら選択欄を出さない。
export function providerPermissionChoices(id: unknown): readonly string[] {
  return providerDeclaration(id)?.permissions?.choices ?? [];
}

// 貼り付けた本文が落ちるので Enter を遅らせる CLI か。
export function providerSlowPaste(id: unknown): boolean {
  return providerDeclaration(id)?.input?.slow_paste === true;
}

// 実行中の応答を止めるキー。'esc'（既定）か 'ctrl-c'。
export function providerInterruptKey(id: unknown): 'esc' | 'ctrl-c' {
  return providerDeclaration(id)?.input?.interrupt_key === 'ctrl-c' ? 'ctrl-c' : 'esc';
}

// 本文やフルフレームを絶対カーソル移動で描く CLI か（画面はそれをステータスバー更新として捨てない）。
export function providerFullFrameCursor(id: unknown): boolean {
  return providerDeclaration(id)?.input?.full_frame_cursor === true;
}

// Ctrl+U を「行クリア」と読まない CLI か（送ると続く送信が壊れる）。
export function providerIgnoresLineClear(id: unknown): boolean {
  return providerDeclaration(id)?.input?.no_line_clear === true;
}

// 同梱の 7 本か。登録した AI CLI は false。宣言が何も入っていなければ false。
export function providerIsBuiltin(id: unknown): boolean {
  return providerDeclaration(id)?.builtin === true;
}

// 起動パネルの許可モード「auto」「bypassPermissions」の文言の訳名。
// 訳の名前は spawn_permission_<種類>_<id（- は _）>。provider 専用の訳を探すのは同梱の
// CLI（宣言の builtin）だけにする。登録した AI CLI は、id が同梱の CLI に似ていても
// （例: cursor で始まる id）その CLI 専用の文言を出さず、共通の文言にする。
// 同梱の cursor-agent だけは、訳を cursor の名前で持っている。
// 先頭から順に、訳があるものを使う。最後は必ず共通の文言。
export type PermissionLabelKind = 'auto' | 'bypass';

export function permissionLabelKeys(kind: PermissionLabelKind, provider: string): string[] {
  const base = `spawn_permission_${kind}`;
  if (!providerIsBuiltin(provider)) return [base];
  const suffix = provider.replace(/-/g, '_');
  const keys = [`${base}_${suffix}`];
  if (suffix.startsWith('cursor_')) keys.push(`${base}_cursor`);
  keys.push(base);
  return keys;
}

// translate は訳が無いとき名前をそのまま返す（i18n の t と同じ約束）。
export function permissionLabelText(
  kind: PermissionLabelKind,
  provider: string,
  translate: (key: string) => string,
): string {
  const keys = permissionLabelKeys(kind, provider);
  for (const key of keys.slice(0, -1)) {
    const text = translate(key);
    if (text && text !== key) return text;
  }
  return translate(keys[keys.length - 1]);
}

// モデル名の先頭が合うときのコンテキスト上限の既定。宣言が無ければ 0（不明）。
export function providerContextLimit(id: unknown): number {
  const limit = providerDeclaration(id)?.context_limit;
  return typeof limit === 'number' && limit > 0 ? limit : 0;
}

// 'pty' の AI CLI か。acp は人が直接操作する別の接続（自動の経路からは使えない）。
export function providerIsAcp(id: unknown): boolean {
  return providerDeclaration(id)?.connection === 'acp';
}
