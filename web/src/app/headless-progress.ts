// 画面なし（headless）実行の進み。Hub が持つ同じスナップショットと、受け取った時刻を
// 起点にした 1 本の時計で、サイドバーのカード・開いているセッション・ペインを描く。
// 端末の文字は読まない。値は Rust 側の許可した項目だけ（rust/src/orchestration/headless_progress.rs）。
// 文字列は Rust 側で HTML の 5 文字だけを escape して届くので、ここで 1 回だけ戻し、
// 描くときは必ず textContent へ入れる（innerHTML へは入れない）。
export interface HeadlessProgress {
  version: number; run_id: string; sequence: number;
  observed_turns: number; result_turns?: number | null;
  active_tools: { id: string; name: string }[];
  started_at_ms: number; elapsed_ms: number; ended_at_ms?: number | null;
  cost: { amount_usd?: number | null; currency: string; kind: string; coverage: string };
  last_assistant: string; state: string; data_incomplete: boolean; final_cost_missing: boolean;
}
const states = new Set(['running', 'finishing', 'completed', 'failed', 'canceled', 'disconnected']);
const nonnegative = (v: unknown): v is number => Number.isSafeInteger(v) && Number(v) >= 0;
const text = (v: unknown, max: number): v is string => typeof v === 'string' && v.length <= max;
const id = (v: unknown): v is string => text(v, 128) && /^[A-Za-z0-9_:.-]+$/.test(v);
const live = (state: string) => state === 'running' || state === 'finishing';

/** 形・範囲・状態の組み合わせが Rust の検査と合わないものは捨てる（null）。 */
export function normalizeHeadlessProgress(raw: unknown): HeadlessProgress | null {
  if (!raw || typeof raw !== 'object') return null;
  const p = raw as HeadlessProgress;
  if (p.version !== 1 || !id(p.run_id) || !nonnegative(p.sequence)
    || !nonnegative(p.observed_turns) || p.observed_turns > 1_000_000
    || (p.result_turns != null && (!nonnegative(p.result_turns) || p.result_turns > 1_000_000))
    || !nonnegative(p.elapsed_ms) || p.elapsed_ms > 366 * 24 * 60 * 60 * 1000 || !nonnegative(p.started_at_ms)
    || (p.ended_at_ms != null && (!nonnegative(p.ended_at_ms) || p.ended_at_ms < p.started_at_ms))
    || !states.has(p.state) || !text(p.last_assistant, 3072)
    || !Array.isArray(p.active_tools) || p.active_tools.length > 16
    || p.active_tools.some(tool => !tool || !id(tool.id) || !text(tool.name, 384) || !tool.name)
    || new Set(p.active_tools.map(tool => tool.id)).size !== p.active_tools.length
    || (!live(p.state) && (p.active_tools.length !== 0 || p.ended_at_ms == null))
    || !p.cost || p.cost.currency !== 'USD'
    || !['estimate', 'result', 'unavailable'].includes(p.cost.kind)
    || !['observed-main', 'partial', 'cli-result', 'unavailable'].includes(p.cost.coverage)
    || (p.cost.amount_usd != null && (!Number.isFinite(p.cost.amount_usd) || p.cost.amount_usd < 0 || p.cost.amount_usd > 1_000_000))
    || (p.cost.kind === 'unavailable') !== (p.cost.amount_usd == null)
    || (p.cost.kind === 'unavailable' && p.cost.coverage !== 'unavailable')
    || (p.cost.kind === 'result' && p.cost.coverage !== 'cli-result')
    || (p.cost.kind === 'estimate' && !['observed-main', 'partial'].includes(p.cost.coverage))) return null;
  return {
    version: 1, run_id: p.run_id, sequence: p.sequence,
    observed_turns: p.observed_turns, result_turns: p.result_turns ?? null,
    active_tools: p.active_tools.map(tool => ({ id: tool.id, name: tool.name })),
    started_at_ms: p.started_at_ms, elapsed_ms: p.elapsed_ms, ended_at_ms: p.ended_at_ms ?? null,
    cost: { amount_usd: p.cost.amount_usd ?? null, currency: 'USD', kind: p.cost.kind, coverage: p.cost.coverage },
    last_assistant: p.last_assistant, state: p.state,
    data_incomplete: p.data_incomplete === true, final_cost_missing: p.final_cost_missing === true,
  };
}

const received = new WeakMap<HeadlessProgress, number>();
let connected = true;
/** Hub との接続が切れている間は時計を止め、古い数字を進行中に見せない。 */
export function setHeadlessConnected(value: boolean): void { connected = value; }

/**
 * 届いたスナップショットを受け入れるか決める。別の実行・古い順番・巻き戻った時計は捨てる。
 * 同じ値の重複は足し込まず、受け入れたものが前の値を置き換える。
 */
export function receiveHeadlessProgress(raw: unknown, previous?: HeadlessProgress | null, now = performance.now()): HeadlessProgress | null {
  const next = normalizeHeadlessProgress(raw);
  if (!next || (previous && (next.run_id !== previous.run_id || next.sequence <= previous.sequence
    || next.elapsed_ms < previous.elapsed_ms
    || (previous.sequence > 0 && next.started_at_ms !== previous.started_at_ms)))) return null;
  received.set(next, now);
  return next;
}

/** 経過。実行中は受け取ってからの時間を足し、終わったら（または切断中は）止める。 */
export function elapsedHeadlessMs(p: HeadlessProgress, now = performance.now(), isConnected = connected): number {
  const start = received.get(p);
  return p.elapsed_ms + (isConnected && start != null && live(p.state) ? Math.max(0, now - start) : 0);
}

// Rust の投影が出す 5 つの実体参照だけを戻す。
export const decodeHeadlessText = (value: string): string => value.replace(/&(?:amp|lt|gt|quot|#39);/g,
  entity => ({ '&amp;': '&', '&lt;': '<', '&gt;': '>', '&quot;': '"', '&#39;': "'" })[entity] ?? entity);

export type HeadlessTranslate = (key: string, vars?: Record<string, unknown>) => string;

/**
 * 表示する 6 行（状態・ターン・道具・経過・費用・最後の発言）。
 * structured=false（Claude の stream 以外）は数えられない値を「未取得」のままにし、0 にしない。
 */
export function headlessProgressText(p: HeadlessProgress | null | undefined, translate: HeadlessTranslate, now = performance.now(), structured = true): string[] {
  const t = translate;
  if (!p) return [t('headless_waiting'), t('headless_turns_unknown'), t('headless_tool_unknown'), t('headless_elapsed_unknown'), t('headless_cost_unknown'), t('headless_last_unknown')];
  const seconds = Math.floor(elapsedHeadlessMs(p, now) / 1000);
  const elapsed = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
  const tools = p.active_tools;
  const tool = tools.length ? `${decodeHeadlessText(tools[tools.length - 1].name)}${tools.length > 1 ? ` (+${tools.length - 1})` : ''}` : t('headless_none');
  const amount = p.cost.amount_usd;
  const money = amount == null ? t('headless_unknown') : `$${amount.toFixed(4)}`;
  const cost = p.cost.kind === 'result' ? t('headless_cost_result', { amount: money })
    : p.cost.kind === 'estimate' ? t(p.cost.coverage === 'partial' ? 'headless_cost_partial' : 'headless_cost_estimate', { amount: money })
    : t('headless_cost_unknown');
  const state = !connected ? t('headless_disconnected') : p.sequence === 0 ? t('headless_waiting') : t(`headless_state_${p.state}`);
  const observed = structured && p.sequence > 0;
  return [
    `${state}${p.data_incomplete ? ` · ${t('headless_incomplete')}` : ''}`,
    observed ? t(p.result_turns == null ? 'headless_turns_observed' : 'headless_turns_result', { count: p.result_turns ?? p.observed_turns }) : t('headless_turns_unknown'),
    observed ? t('headless_tool', { tool }) : t('headless_tool_unknown'),
    p.sequence > 0 ? t('headless_elapsed', { elapsed }) : t('headless_elapsed_unknown'),
    `${cost}${amount != null ? ` · ${t('headless_reference')}` : ''}${!live(p.state) && p.final_cost_missing ? ` · ${t('headless_final_missing')}` : ''}`,
    observed ? t('headless_last', { text: decodeHeadlessText(p.last_assistant) || t('headless_unknown') }) : t('headless_last_unknown'),
  ];
}

/** サイドバー・開いているセッション・ペインで共有する描き先。常に平文で入れる。 */
export function renderHeadlessProgressValues(el: HTMLElement, values: string[]): void {
  while (el.children.length < values.length) el.appendChild(el.ownerDocument.createElement('span'));
  values.forEach((value, index) => {
    const child = el.children[index] as HTMLElement;
    child.className = index === values.length - 1 ? 'headless-progress-last' : 'headless-progress-metric';
    if (child.textContent !== value) child.textContent = value;
  });
}
