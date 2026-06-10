// token-statusbar.ts — 画面最下部に固定表示するトークン/コスト ステータスバー（案1 レイアウト）。
//
// 表示対象: アクティブセッション 1 件分のみ。
// provider 差分:
//   - claude: tok ↑in ↓out / $ cost / model / ⏱ 経過時間
//     （tok は現在のコンテキストウィンドウ使用量。セッション累積ではない）
//   - codex:  tok ↑in ↓out / $ cost / model / ⏱ 経過時間
//   - copilot / cursor-agent: model / ⏱ 経過時間 / project のみ
// 未取得セグメントは DOM から非表示にしてレイアウト崩れを防ぐ。
// コスト不明（cost_known=false）時は "$ —" を表示し誤金額を出さない。

import type { Message } from '../types/proto.js';
import { activeSessionId, sessions } from './state.js';
import { token } from './util.js';

// セッション単位の usage データキャッシュ。
interface UsageCacheEntry {
  provider: string;
  costUSD: number;
  costKnown: boolean;
  tokensIn: number;
  tokensOut: number;
  tokensCache: number;
  tokensTotal: number;
  usageModel: string;
  usageStartedAt: string;
}

const usageCache = new Map<number, UsageCacheEntry>();

// 毎秒の経過時間更新用 timer。
let _tickInterval: ReturnType<typeof setInterval> | null = null;
// バー全体の有効/無効フラグ（settings から制御）。
let _barEnabled = true;

// ── DOM 要素参照 ──────────────────────────────────────────────────────────────

function getBar(): HTMLElement | null {
  return document.getElementById('token-statusbar') as HTMLElement | null;
}

// ── 経過時間フォーマット ──────────────────────────────────────────────────────
// startedAt は ISO 8601 (RFC 3339) 文字列。Date.toLocaleString() は使わない。

function formatElapsed(startedAt: string): string {
  if (!startedAt) return '';
  const start = Date.parse(startedAt);
  if (isNaN(start)) return '';
  const elapsedSec = Math.max(0, Math.floor((Date.now() - start) / 1000));
  const h = Math.floor(elapsedSec / 3600);
  const m = Math.floor((elapsedSec % 3600) / 60);
  const s = elapsedSec % 60;
  if (h > 0) return `${h}h ${m}m ${s}s`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

// ── コスト表示フォーマット ──────────────────────────────────────────────────

function formatCost(costUSD: number, costKnown: boolean): string {
  if (!costKnown) return '$ —';
  if (costUSD === 0) return '$0.0000';
  if (costUSD < 0.0001) return '$<0.0001';
  return '$' + costUSD.toFixed(4);
}

// ── トークン表示フォーマット ──────────────────────────────────────────────────

function formatTok(n: number): string {
  if (n === 0) return '0';
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k';
  return String(n);
}

// ── プロジェクト名取得 ──────────────────────────────────────────────────────

function getProject(sessionId: number): string {
  const s = sessions.get(sessionId);
  if (!s) return '';
  // project キーは deriveProjectKeyFromCwd で設定される（state.js で管理）
  const p = (s as any).project || '';
  if (p) return p;
  // cwd から末尾ディレクトリ名を取る
  const cwd = s.cwd || '';
  if (!cwd) return '';
  const parts = cwd.replace(/\\/g, '/').split('/').filter(Boolean);
  return parts[parts.length - 1] || '';
}

// ── ステータスバー描画 ────────────────────────────────────────────────────────

export function renderStatusbar(): void {
  const bar = getBar();
  if (!bar) return;

  if (!_barEnabled) {
    bar.style.display = 'none';
    return;
  }

  const sid = activeSessionId;
  if (sid === null) {
    bar.style.display = 'none';
    return;
  }

  const entry = usageCache.get(sid);
  const sesInfo = sessions.get(sid);
  const provider: string = entry?.provider || sesInfo?.provider || '';

  // provider が不明な場合でも最低限表示するため、完全非表示はしない。
  // ただし usage データが全くない場合はバーを隠す。
  if (!entry && !sesInfo) {
    bar.style.display = 'none';
    return;
  }

  bar.style.display = 'flex';

  // ---- project セグメント ----
  const projectEl = bar.querySelector<HTMLElement>('.tsb-seg-project');
  const project = getProject(sid);
  const sesData = sessions.get(sid);
  const branch = sesData?.branch || '';
  if (projectEl) {
    if (project) {
      projectEl.textContent = branch ? `📁 ${project}(${branch})` : `📁 ${project}`;
      projectEl.style.display = '';
    } else {
      projectEl.style.display = 'none';
    }
  }

  // ---- model セグメント ----
  const modelEl = bar.querySelector<HTMLElement>('.tsb-seg-model');
  const modelName = entry?.usageModel || sesData?.model || '';
  if (modelEl) {
    if (modelName) {
      modelEl.textContent = `🤖 ${modelName}`;
      modelEl.style.display = '';
    } else {
      modelEl.style.display = 'none';
    }
  }

  // ---- tok セグメント（Claude / Codex）----
  const tokEl = bar.querySelector<HTMLElement>('.tsb-seg-tok');
  if (tokEl) {
    // provider 値で判定（モデル名文字列には依存しない）
    if ((provider === 'codex' || provider === 'claude') && entry) {
      const inStr  = formatTok(entry.tokensIn);
      const outStr = formatTok(entry.tokensOut);
      tokEl.textContent = `tok ↑${inStr} ↓${outStr}`;
      tokEl.style.display = '';
    } else {
      tokEl.style.display = 'none';
    }
  }

  // ---- cost セグメント（claude / codex のみ）----
  const costEl = bar.querySelector<HTMLElement>('.tsb-seg-cost');
  if (costEl) {
    if ((provider === 'claude' || provider === 'codex') && entry) {
      costEl.textContent = formatCost(entry.costUSD, entry.costKnown);
      costEl.style.display = '';
    } else {
      costEl.style.display = 'none';
    }
  }

  // ---- elapsed セグメント ----
  const elapsedEl = bar.querySelector<HTMLElement>('.tsb-seg-elapsed');
  if (elapsedEl) {
    // startedAt: entry から取るか session の started_at を使う
    const startedAt = entry?.usageStartedAt || sesData?.started_at || '';
    if (startedAt) {
      elapsedEl.textContent = `⏱ ${formatElapsed(startedAt)}`;
      elapsedEl.style.display = '';
    } else {
      elapsedEl.style.display = 'none';
    }
  }
}

// ── 外部 API ─────────────────────────────────────────────────────────────────

/** WS usage_stat メッセージを受信したときに呼ぶ。 */
export function handleUsageStatMessage(m: Message): void {
  const sid = m.session_id;
  if (!sid) return;
  usageCache.set(sid, {
    provider:       m.provider       || '',
    costUSD:        m.cost_usd       ?? 0,
    costKnown:      m.cost_known     ?? false,
    tokensIn:       m.tokens_in      ?? 0,
    tokensOut:      m.tokens_out     ?? 0,
    tokensCache:    m.tokens_cache   ?? 0,
    tokensTotal:    m.tokens_total   ?? 0,
    usageModel:     m.usage_model    || '',
    usageStartedAt: m.usage_started_at || '',
  });
  // アクティブセッションの更新なら即座に再描画
  if (sid === activeSessionId) {
    renderStatusbar();
  }
}

/** セッション削除時にキャッシュをクリアする。 */
export function removeUsageCacheEntry(sessionId: number): void {
  usageCache.delete(sessionId);
  if (sessionId === activeSessionId) {
    renderStatusbar();
  }
}

/** アクティブセッションが変わった時に呼ぶ（セッション切替時の snapshot 更新）。 */
export function onActiveSessionChanged(): void {
  renderStatusbar();
}

/** 設定の enabled 値を適用する（起動時 + トグル変更時）。 */
export function setStatusbarEnabled(enabled: boolean): void {
  _barEnabled = enabled;
  const bar = getBar();
  if (bar) {
    bar.style.display = enabled ? '' : 'none';
  }
  // attach-panel が fixed statusbar に隠れないよう padding-bottom を同期する
  document.body.style.setProperty('--tsb-bottom-offset', enabled ? '22px' : '0px');
  if (enabled) {
    renderStatusbar();
    startTick();
  } else {
    stopTick();
  }
}

/** ステータスバーの有効/無効を返す。 */
export function isStatusbarEnabled(): boolean {
  return _barEnabled;
}

// ── 毎秒 tick（経過時間更新）──────────────────────────────────────────────────

function startTick(): void {
  if (_tickInterval) return;
  _tickInterval = setInterval(() => {
    if (!_barEnabled) return;
    renderStatusbar();
  }, 1000);
}

function stopTick(): void {
  if (_tickInterval) {
    clearInterval(_tickInterval);
    _tickInterval = null;
  }
}

// ── 初期化 ────────────────────────────────────────────────────────────────────

/** DOM 準備完了後に呼ぶ初期化関数。/api/user-prefs から enabled を読んで初期表示を決める。 */
export async function initTokenStatusbar(): Promise<void> {
  startTick();
  try {
    const res = await fetch(`/api/user-prefs?token=${encodeURIComponent(token || '')}`);
    if (!res.ok) return;
    const data = await res.json();
    // token_statusbar.enabled が null/undefined（未設定）または true のとき ON（既定 ON）
    const tsb = data?.token_statusbar;
    const enabled = tsb == null || tsb.enabled == null ? true : !!tsb.enabled;
    setStatusbarEnabled(enabled);
  } catch (_) {
    // fetch 失敗時は既定 ON を維持
    setStatusbarEnabled(true);
  }
}
