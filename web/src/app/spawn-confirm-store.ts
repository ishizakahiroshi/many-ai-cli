// 子セッション起動の確認待ちを保持する、DOM に一切触れない純粋なストア。
//
// spawn-confirm.ts（ダイアログの markup 生成・イベント配線）から意図的に分離している。
// spawn-confirm.ts は session-list.js を import しており、それが terminal.ts /
// settings.ts / chat-history.ts 等の巨大な import グラフを引き込む。その中には
// `document.addEventListener(...)` のようなトップレベル副作用があるモジュールが含まれ、
// DOM の無い Bun テスト環境（web/tests/*.test.ts、bun:test）で import すると
// ReferenceError で落ちる。ここに置いた純粋関数だけは、その import グラフに触れずに
// 単体テストできる（web/tests/spawn-confirm-store.test.ts）。
//
// plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md C3。

// 子が実際に起動する承認設定。Hub 側で権限の段の表を通して作ったものが
// provider × 段ごとに入っている（internal/hub/child_permission.go と
// internal/hub/orchestration.go の childApprovalPreviewTiers）。
// 全項目が空なら「Hub は何も足さない」＝ CLI 既定のまま。
export interface ChildApproval {
  permissionMode: string;
  sandbox: string;
  askForApproval: string;
  // 段 2（bounded）で子に許す tool / コマンドパターン。他の段では空。
  allowedTools: string[];
  riskConfirmed: boolean;
  // 実際に適用される段（attended / bounded / full）。
  tier: string;
  // 要求された段がこの provider に無くて別の段へ落ちた場合、その要求された段。
  // 今日は "bounded" のみ（grok / cursor-agent には範囲を限る設定が無い）。
  fallbackFrom: string;
}

// Hub と wrapper の間だけで使う permission mode の内部マーカー。copilot は
// --allow-tool の列挙へ、opencode は opencode.json の規則へ変換されるので、
// この値は実 CLI のフラグ値としては存在しない（正本: config.PermissionModeBounded）。
// 画面でフラグ名として見せると、実在しない起動コマンドを読ませることになる。
export const INTERNAL_BOUNDED_PERMISSION_MODE = 'bounded';

export interface SpawnConfirmationRecord {
  id: string;
  parentId: number;
  role: string;
  provider: string;
  model: string;
  cwd: string;
  initialPrompt: string;
  // 起動要求の共通 3 項目（要求された値）。空文字は「指定なし」。
  // ダイアログはこれを初期表示し、承認時は select の実効値をそのまま決定 body に載せる。
  effort: string;
  executionMode: string;
  permissionPreset: string;
  // 「この役割では次回もこの段を使う」チェックボックスの初期状態。Hub がその役割の
  // 段を既に覚えていれば true で開く。欄を持たない古い Hub からは false になり、
  // そのときは「覚えていない」と同じ見え方になる（承認しても記憶は増えない）。
  rememberPermission: boolean;
  requestedAtMs: number;
  // provider → 段 → 実効権限。2 段のキーにするのは、ダイアログが承認前に
  // provider と段のどちらも差し替えられるため。選び直した瞬間に表示を切り替えられる
  // よう、組み合わせを先に全部受け取っておく（Hub への往復を挟まない）。
  // 内側の "" は「要求どおり」＝要求に載っていた permission_preset での実効値。
  approval: Record<string, Record<string, ChildApproval>>;
}

// 起動要求の共通 3 項目の候補値。正本は Hub（internal/config/effort.go）で、
// /api/info が provider ごとの effort 候補とこの build で選べる実行モード・権限段を
// 返す。ここに置いてあるのは「まだ /api/info を読んでいないとき」の保守的な既定で、
// 画面の描画にしか使わない（受理の判断は常に Hub 側が行う）。
let effortLevelsByProvider: Record<string, string[]> = {};
let availableExecutionModes: string[] = ['auto', 'interactive'];
let availablePermissionPresets: string[] = ['attended', 'full'];

// headless の定義がある provider（/api/info の headless_providers）。実行モードの
// 選択肢そのものは全 provider で出るが、定義が無い CLI へ headless を明示すると
// Hub は 400 で返す。この一覧はその refusal を押す前に 1 行で言うためだけにあり、
// 判定の正本ではない（正本は起動時の解決）。空のままなら「分からない」なので
// 注意書きを出さない＝古い Hub では今までどおり何も出ない。
let headlessProviders: string[] = [];

// 画面から立てる子（origin: "ui"）の実効権限。provider → 段 → 実効フラグで、
// 形は spawn 確認ダイアログが WS で受け取るものと同じ（Hub 側は同じ
// childApprovalPreviewTiers が作る）。確認ダイアログには対になる要求があるが、
// 派生ダイアログには「これから作る要求」しか無いので /api/info から先に受け取る。
// 空のままなら呼び出し側は開示欄を出さない（古い Hub では欄ごと無い）。
let childPermissionPreview: Record<string, Record<string, ChildApproval>> = {};

// 役割 → 「次回もこの段を使う」で覚えた段（/api/info の role_permission）。派生ダイアログ
// が役割を選んだ時点の初期値に使う。**Hub は UI 起点の要求をこの記憶で埋め直さない**ので、
// 画面がここから入れた段を人が変えたら、変えた段がそのまま送られる。
// 空のままなら「覚えていない」＝今までどおり段の select は「指定なし」で開く。
let rolePermissionMemory: Record<string, string> = {};

// スキーマ（この build で選べない値も含む）。選べない値は disabled で見せる:
// 「まだ無い」ことが分かる方が、欄ごと消えて理由が分からないより良い。
export const EXECUTION_MODE_SCHEMA = ['auto', 'interactive', 'headless'] as const;
export const PERMISSION_PRESET_SCHEMA = ['attended', 'bounded', 'full'] as const;

// /api/info の応答から候補値を取り込む。欄が無い（古い Hub）ときは既定のまま。
export function setLaunchOptionChoices(info: any): void {
  const levels = info?.effort_levels;
  if (levels && typeof levels === 'object') {
    const next: Record<string, string[]> = {};
    for (const [provider, value] of Object.entries(levels as Record<string, any>)) {
      const key = String(provider || '').trim();
      if (!key || !Array.isArray(value)) continue;
      const list = value.map((v) => String(v)).filter((v) => v);
      if (list.length) next[key] = list;
    }
    effortLevelsByProvider = next;
  }
  if (Array.isArray(info?.execution_modes)) {
    const list = info.execution_modes.map((v: any) => String(v)).filter((v: string) => v);
    if (list.length) availableExecutionModes = list;
  }
  if (Array.isArray(info?.permission_presets)) {
    const list = info.permission_presets.map((v: any) => String(v)).filter((v: string) => v);
    if (list.length) availablePermissionPresets = list;
  }
  if (Array.isArray(info?.headless_providers)) {
    headlessProviders = info.headless_providers.map((v: any) => String(v).trim()).filter((v: string) => v);
  }
  if (info?.child_permission_preview && typeof info.child_permission_preview === 'object') {
    childPermissionPreview = approvalsFromMessage(info.child_permission_preview);
  }
  if (info?.role_permission && typeof info.role_permission === 'object') {
    const next: Record<string, string> = {};
    for (const [role, value] of Object.entries(info.role_permission as Record<string, any>)) {
      const key = String(role || '').trim();
      const tier = String(value ?? '').trim();
      if (key && tier) next[key] = tier;
    }
    rolePermissionMemory = next;
  }
}

// その役割で覚えている段。覚えていなければ空文字（＝指定なし）。
// この build で選べない段が入っていたら空を返す: 選択肢に無い値を select へ入れると、
// 見えている段と送る段が食い違う。
export function rememberedRolePermission(role: string): string {
  const tier = rolePermissionMemory[String(role || '').trim()] || '';
  return tier && isPermissionPresetAvailable(tier) ? tier : '';
}

// この provider を headless で起動できるか。
//
// 返り値は 3 値ではなく 2 値だが、「分からない」は false ではなく **true** 側へ倒す:
// 一覧が空（古い Hub・/api/info をまだ読んでいない）ときに「この CLI は headless に
// 対応していません」と書くと、対応している CLI について嘘をつくことになる。出せる
// 注意書きが 1 つ減るだけの側へ倒す。
export function isHeadlessCapable(provider: string): boolean {
  if (!headlessProviders.length) return true;
  return headlessProviders.includes(String(provider || '').trim());
}

// 実行モードの選択と provider の組み合わせが、押した瞬間に Hub で弾かれるか。
// headless を明示したときだけ true になる（auto は対応していなければ対話へ倒れるので
// 弾かれない・親 plan D2）。ダイアログはこれを見て注意書きを 1 行出す。
export function headlessUnsupportedForSelection(provider: string, executionMode: string): boolean {
  return String(executionMode || '').trim() === 'headless' && !isHeadlessCapable(provider);
}

// 派生ダイアログが「今の選択でこの子に何が渡るか」を引くための窓。返り値をそのまま
// spawn-confirm.ts の approvalDisplayHtml へ渡す（同じ表示を 2 通り書かない）。
// 表が空（古い Hub）なら undefined で、呼び出し側は開示欄を出さない。
export function childPermissionPreviewFor(provider: string, tier: string): ChildApproval | undefined {
  return approvalForSelection(childPermissionPreview, provider, tier);
}

// provider に effort の写像が無ければ空配列。呼び出し側は空なら欄ごと出さない
// （候補の無い select を見せない）。
export function effortLevelsFor(provider: string): string[] {
  const list = effortLevelsByProvider[String(provider || '').trim()];
  return Array.isArray(list) ? list.slice() : [];
}

// New Session フォームが送る effort を決める純関数。写像が無い provider（欄が
// 隠れている）と「指定なし」はどちらも空を返し、呼び出し側は空ならキーごと送らない。
// 前の provider で選んだ値が残っていても、新しい provider の候補に無ければ空にする
// （Hub はその組み合わせを 400 で弾くため、送ると起動できなくなる）。
export function effortForSpawnBody(provider: string, selected: string): string {
  const level = String(selected || '').trim();
  if (!level) return '';
  return effortLevelsFor(provider).includes(level) ? level : '';
}

export function isExecutionModeAvailable(mode: string): boolean {
  return availableExecutionModes.includes(mode);
}

export function isPermissionPresetAvailable(preset: string): boolean {
  return availablePermissionPresets.includes(preset);
}

export interface SpawnConfirmationClosedController {
  applyClosed: (m: any) => void;
}

// Hub がまだ持っている（決定が付いていない）確認の一覧。
// spawn_confirmation_requested（新規要求 / 再接続時の再送のどちらも同じ型）で追加し、
// spawn_confirmation_closed で取り除く。
const pendingSpawnConfirmations = new Map<string, SpawnConfirmationRecord>();

// 現在ダイアログとして開いている確認 ID → その結果反映コールバック。
const openDialogControllers = new Map<string, SpawnConfirmationClosedController>();

export function recordFromMessage(m: any): SpawnConfirmationRecord {
  return {
    id: String(m?.spawn_confirmation_id || ''),
    parentId: Number(m?.session_id || 0),
    role: String(m?.role || ''),
    provider: String(m?.provider || ''),
    model: String(m?.model || ''),
    cwd: String(m?.cwd || ''),
    initialPrompt: String(m?.initial_prompt || ''),
    effort: String(m?.effort || ''),
    executionMode: String(m?.execution_mode || ''),
    permissionPreset: String(m?.permission_preset || ''),
    rememberPermission: Boolean(m?.remember_permission),
    requestedAtMs: Number(m?.spawn_requested_at_ms || 0) || Date.now(),
    approval: approvalsFromMessage(m?.spawn_child_approval),
  };
}

// 旧 Hub から届いた（このフィールドを持たない）メッセージでも落ちないよう、
// 欠けていれば空の表として扱う。呼び出し側は「エントリが無い＝不明」ではなく
// 「何も足されない」と読める形に寄せている。
export function approvalsFromMessage(raw: any): Record<string, Record<string, ChildApproval>> {
  const out: Record<string, Record<string, ChildApproval>> = {};
  if (!raw || typeof raw !== 'object') return out;
  for (const [provider, tiers] of Object.entries(raw as Record<string, any>)) {
    const key = String(provider || '').trim();
    if (!key) continue;
    const byTier: Record<string, ChildApproval> = {};
    if (tiers && typeof tiers === 'object') {
      for (const [tier, value] of Object.entries(tiers as Record<string, any>)) {
        byTier[String(tier ?? '')] = {
          permissionMode: String(value?.permission_mode || ''),
          sandbox: String(value?.sandbox || ''),
          askForApproval: String(value?.ask_for_approval || ''),
          allowedTools: Array.isArray(value?.allowed_tools)
            ? value.allowed_tools.map((v: any) => String(v)).filter((v: string) => v)
            : [],
          riskConfirmed: Boolean(value?.risk_confirmed),
          tier: String(value?.tier || ''),
          fallbackFrom: String(value?.fallback_from || ''),
        };
      }
    }
    out[key] = byTier;
  }
  return out;
}

// ダイアログが今まさに見せるべき 1 件を選ぶ。provider と段の select の現在値で引き、
// その組み合わせが無ければ「要求どおり」（内側の ""）へ落とす。純関数にしてあるのは、
// 「選び直したら表示も追随する」が DOM 無しで検証できる唯一の形だから。
export function approvalForSelection(
  approval: Record<string, Record<string, ChildApproval>>,
  provider: string,
  tier: string,
): ChildApproval | undefined {
  const byTier = approval[String(provider || '').trim()];
  if (!byTier) return undefined;
  return byTier[String(tier ?? '')] ?? byTier[''];
}

// spawn_confirmation_requested を受けたらストアへ積む。同じ ID の再送（UI 再接続時の
// resend）では単に上書きする。
export function noteSpawnConfirmationRequested(m: any): void {
  const record = recordFromMessage(m);
  if (!record.id) return;
  pendingSpawnConfirmations.set(record.id, record);
}

// parentId の保留件数（サイドバーの印・× 無効化の両方から使う）。
export function pendingSpawnConfirmationCount(parentId: number): number {
  let n = 0;
  for (const rec of pendingSpawnConfirmations.values()) {
    if (rec.parentId === parentId) n++;
  }
  return n;
}

// parentId の保留のうち、isOpen(id) が false のものだけを対象に、最も古い
// （requestedAtMs が最小の）ものを 1 件返す。「開いている確認をもう一度開かない」
// 「古い順に開く」の 2 つのルールを、任意の Map を渡して検証できるようにしている。
export function selectOldestPendingConfirmation(
  records: Map<string, SpawnConfirmationRecord>,
  parentId: number,
  isOpen: (id: string) => boolean,
): SpawnConfirmationRecord | null {
  let best: SpawnConfirmationRecord | null = null;
  for (const rec of records.values()) {
    if (rec.parentId !== parentId) continue;
    if (isOpen(rec.id)) continue;
    if (!best || rec.requestedAtMs < best.requestedAtMs) best = rec;
  }
  return best;
}

// spawn-confirm.ts の openNextSpawnConfirmationFor から使う、モジュール内シングルトンの
// ストアに対する薄いラッパー。
export function selectOldestPendingConfirmationFor(parentId: number): SpawnConfirmationRecord | null {
  return selectOldestPendingConfirmation(pendingSpawnConfirmations, parentId, (id) => openDialogControllers.has(id));
}

export function registerDialogController(id: string, controller: SpawnConfirmationClosedController): void {
  if (!id) return;
  openDialogControllers.set(id, controller);
}

export function unregisterDialogController(id: string): void {
  openDialogControllers.delete(id);
}

export function isDialogControllerOpen(id: string): boolean {
  return openDialogControllers.has(id);
}

// spawn_confirmation_closed を受けたらストアから外し、開いているダイアログが
// あればそちらへ結果を渡す。開いていなければ（サイドバーの印を消すだけで）何もしない。
export function closeSpawnConfirmation(m: any): void {
  const id = String(m?.spawn_confirmation_id || '');
  if (!id) return;
  pendingSpawnConfirmations.delete(id);
  const controller = openDialogControllers.get(id);
  if (controller) controller.applyClosed(m);
}

// Hub 再起動検出時にローカル状態を破棄する purgeLocalStateForHubRestart から呼ぶ。
// 開いているダイアログも道連れで閉じる（保留自体が Hub 上で消えているため）。
export function clearAllSpawnConfirmationsForHubRestart(): void {
  pendingSpawnConfirmations.clear();
  for (const [id, controller] of Array.from(openDialogControllers.entries())) {
    controller.applyClosed({ spawn_confirmation_id: id, reason: 'parent_gone' });
  }
}

// After POST /spawn-confirm returns 2xx the Hub has accepted the decision, but
// the dialog still waits for spawn_confirmation_closed. That broadcast can be
// lost if the UI WebSocket is down or stale. HTTP 200 is written before
// performSpawn, so a missing close event must not leave the overlay stuck in
// deciding with Escape disabled and no Close button.
export const SPAWN_CONFIRM_CLOSED_FALLBACK_MS = 4000;

export type SpawnConfirmHttpDecision = {
  waitForCloseBroadcast: boolean;
  fallbackMs: number;
  terminalReason: '' | 'expired' | 'decided_elsewhere' | 'submit_failed';
};

export function spawnConfirmDecisionFromHttp(ok: boolean, status: number): SpawnConfirmHttpDecision {
  if (ok) {
    return { waitForCloseBroadcast: true, fallbackMs: SPAWN_CONFIRM_CLOSED_FALLBACK_MS, terminalReason: '' };
  }
  if (status === 404) {
    return { waitForCloseBroadcast: false, fallbackMs: 0, terminalReason: 'expired' };
  }
  if (status === 409) {
    return { waitForCloseBroadcast: false, fallbackMs: 0, terminalReason: 'decided_elsewhere' };
  }
  return { waitForCloseBroadcast: false, fallbackMs: 0, terminalReason: 'submit_failed' };
}

