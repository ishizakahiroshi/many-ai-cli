// token-statusbar.ts — 画面最下部に固定表示するトークン/コスト ステータスバー。
//
// 表示対象: アクティブセッション 1 件分（+ 全セッション横断バッジ）。
// セグメント構成（左→右）:
//   #N / 状態pill / provider(アイコン+ラベル)+モデル / 作業ラベル /
//   📁project ⎇branch ±git / ctxゲージ / tok / cacheゲージ / compact残量 / cost(+today) /
//   burn / elapsed(+turn) / 接続 / 横断バッジ(▶⏸⚠)
// 未取得セグメントは DOM から非表示にしてレイアウト崩れを防ぐ。
// コスト不明（cost_known=false）時は "$ —" を表示し誤金額を出さない。

import type { Message } from '../types/proto.js';
import { activeSessionId, sessions, chatHistory } from './state.js';
import { token, escapeHtml } from './util.js';
import { t } from '../i18n.js';
import { providerIconHtml, providerDisplayName, safeClassToken, stateLabel, activateSession } from './session-list.js';
import { wsConnectionState } from './ws-client.js';
import { FilesTabManager } from './files-view.js';
import { showPathPopup } from './path-links.js';

// セッション単位の usage データキャッシュ。
interface UsageCacheEntry {
  provider: string;
  costUSD: number;
  costKnown: boolean;
  tokensIn: number;
  tokensOut: number;
  tokensCache: number;
  tokensTotal: number;
  ctxWindow: number;
  usageModel: string;
  usageStartedAt: string;
}

const usageCache = new Map<number, UsageCacheEntry>();

// セッションごとの「このターン」開始時刻フォールバック。
// 基本は chatHistory の直近 user/approval 時刻を使う。履歴がまだ無いセッションでは
// state==running になった時点を暫定起点として使う。
const turnStartAt = new Map<number, number>();

// 毎秒の経過時間更新用 timer。
let _tickInterval: ReturnType<typeof setInterval> | null = null;
// バー全体の有効/無効フラグ（settings から制御）。
let _barEnabled = true;
// クリックハンドラを 1 度だけ結線するためのフラグ。
let _clickWired = false;

// ── モデル別コンテキスト上限テーブル ────────────────────────────────────────
// feedback_no_hardcoded_model_names: UI の分岐にモデル名文字列を使うのは禁止だが、
// ここは「ID→数値上限」の純粋なデータマップなので許容範囲。前方一致で解決する。
// ヒットしないモデルは null を返し、ctx 率セグメントを非表示にする（誤分母回避）。
const CTX_LIMIT_TABLE: Array<{ prefix: string; limit: number }> = [
  { prefix: 'claude-opus',   limit: 200_000 },
  { prefix: 'claude-sonnet', limit: 200_000 },
  { prefix: 'claude-haiku',  limit: 200_000 },
  { prefix: 'claude-3',      limit: 200_000 },
  { prefix: 'claude',        limit: 200_000 },
  { prefix: 'gpt-4.1',       limit: 1_000_000 },
  { prefix: 'gpt-5',         limit: 400_000 },
  { prefix: 'gpt-4o',        limit: 128_000 },
  { prefix: 'gpt-4',         limit: 128_000 },
  { prefix: 'o4',            limit: 200_000 },
  { prefix: 'o3',            limit: 200_000 },
  { prefix: 'o1',            limit: 200_000 },
  { prefix: 'codex',         limit: 400_000 },
  // --- Ollama / ローカルモデル（route="ollama" で claude/codex CLI から利用）---
  // 値は各モデルのネイティブ最大コンテキスト長。タグ（:7b 等）は前方一致で吸収。
  // 注意: 実際に使える窓は Ollama 側の num_ctx 設定に依存し、ここより小さい場合がある。
  // より具体的なプレフィックスを先に置く（startsWith 先勝ちのため）。
  { prefix: 'qwen2.5-coder',     limit: 32_768 },
  { prefix: 'qwen3-coder',       limit: 262_144 },
  { prefix: 'qwen2.5',           limit: 32_768 },
  { prefix: 'qwen3',             limit: 131_072 },
  { prefix: 'qwen',              limit: 32_768 },
  { prefix: 'deepseek-coder-v2', limit: 131_072 },
  { prefix: 'deepseek-r1',       limit: 131_072 },
  { prefix: 'deepseek',          limit: 32_768 },
  { prefix: 'llama3.3',          limit: 131_072 },
  { prefix: 'llama3.2',          limit: 131_072 },
  { prefix: 'llama3.1',          limit: 131_072 },
  { prefix: 'llama3',            limit: 8_192 },
  { prefix: 'codellama',         limit: 16_384 },
  { prefix: 'codestral',         limit: 32_768 },
  { prefix: 'devstral',          limit: 131_072 },
  { prefix: 'mixtral',           limit: 32_768 },
  { prefix: 'mistral',           limit: 32_768 },
  { prefix: 'gemma3',            limit: 131_072 },
  { prefix: 'gemma2',            limit: 8_192 },
  { prefix: 'phi4',              limit: 16_384 },
  { prefix: 'phi3',              limit: 131_072 },
  { prefix: 'starcoder2',        limit: 16_384 },
  { prefix: 'gpt-oss',           limit: 131_072 },
];

function resolveCtxLimit(model: string): number | null {
  const id = String(model || '').toLowerCase().trim();
  if (!id) return null;
  // 1M コンテキスト版（"[1m]" / "1m" / "1-million" 等のマーカー）は上限を底上げ。
  const oneM = /\[1m\]|(^|[^0-9a-z])1m([^0-9a-z]|$)|1-?million/.test(id);
  for (const { prefix, limit } of CTX_LIMIT_TABLE) {
    if (id.startsWith(prefix)) return oneM ? Math.max(limit, 1_000_000) : limit;
  }
  return null;
}

// ── DOM 要素参照 ──────────────────────────────────────────────────────────────

function getBar(): HTMLElement | null {
  return document.getElementById('token-statusbar') as HTMLElement | null;
}

// ── フォーマッタ ──────────────────────────────────────────────────────────────
// startedAt は ISO 8601 (RFC 3339) 文字列。Date.toLocaleString() は使わない。

function formatElapsed(startedAt: string): string {
  if (!startedAt) return '';
  const start = Date.parse(startedAt);
  if (isNaN(start)) return '';
  const elapsedSec = Math.max(0, Math.floor((Date.now() - start) / 1000));
  return formatDurSec(elapsedSec);
}

function formatDurSec(elapsedSec: number): string {
  const h = Math.floor(elapsedSec / 3600);
  const m = Math.floor((elapsedSec % 3600) / 60);
  const s = elapsedSec % 60;
  if (h > 0) return `${h}h ${m}m ${s}s`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

function parseEpochMs(ts: unknown): number {
  if (typeof ts === 'number' && Number.isFinite(ts)) return ts;
  if (ts instanceof Date) {
    const n = ts.getTime();
    return Number.isFinite(n) ? n : 0;
  }
  if (typeof ts === 'string' && ts.trim() !== '') {
    const n = Date.parse(ts);
    return Number.isFinite(n) ? n : 0;
  }
  return 0;
}

function latestInteractionAt(sessionId: number): number | null {
  const arr = chatHistory.get(sessionId) || [];
  for (let i = arr.length - 1; i >= 0; i--) {
    const msg = arr[i];
    if (!msg) continue;
    const role = String(msg.role || '');
    const kind = String(msg.kind || '');
    if (role !== 'user' && kind !== 'approval') continue;
    const text = String(msg.normalizedText || msg.rawText || '').trim();
    const hasAttachment = Array.isArray(msg.attachments) && msg.attachments.length > 0;
    if (!text && !hasAttachment) continue;
    const ts = parseEpochMs(msg.ts);
    if (ts > 0) return ts;
  }
  return null;
}

function formatCost(costUSD: number, costKnown: boolean): string {
  if (!costKnown) return '$ —';
  if (costUSD === 0) return '$0.0000';
  if (costUSD < 0.0001) return '$<0.0001';
  return '$' + costUSD.toFixed(4);
}

function formatTok(n: number): string {
  if (n === 0) return '0';
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k';
  return String(n);
}

// 0..100 に丸めた塗りゲージ HTML。fillClass: ok/warn/crit/cache。
function gaugeHtml(pct: number, fillClass: string): string {
  const w = Math.max(0, Math.min(100, pct));
  return `<span class="tsb-gauge"><span class="tsb-fill ${fillClass}" style="width:${w}%"></span></span>`;
}

// ── プロジェクト名取得 ──────────────────────────────────────────────────────

function getProject(sessionId: number): string {
  const s = sessions.get(sessionId);
  if (!s) return '';
  const p = (s as any).project || '';
  if (p) return p;
  const cwd = s.cwd || '';
  if (!cwd) return '';
  const parts = cwd.replace(/\\/g, '/').split('/').filter(Boolean);
  return parts[parts.length - 1] || '';
}

// 状態 → pill クラス（リストの色定義に合わせる）。
function pillClassFor(state: string): string {
  if (state === 'running') return 'running';
  if (state === 'waiting') return 'waiting';
  if (state === 'error' || state === 'disconnected') return 'error';
  return 'standby';
}

// ── 横断集計（全セッション）────────────────────────────────────────────────

function fleetCounts(): { running: number; standby: number; waiting: number } {
  const c = { running: 0, standby: 0, waiting: 0 };
  sessions.forEach(s => {
    const st = (s.state as string) || 'standby';
    if (st === 'running') c.running++;
    else if (st === 'waiting') c.waiting++;
    else c.standby++;
  });
  return c;
}

// 本日累計コスト（全セッションのライブコスト合算）。
// 集計対象は現在生存しているセッションのみ＝実質「本日アクティブ分」。
function todayCostSum(): { sum: number; known: boolean } {
  let sum = 0;
  let known = false;
  usageCache.forEach(e => {
    if (e.costKnown) { sum += e.costUSD; known = true; }
  });
  return { sum, known };
}

// セグメント表示ユーティリティ。
function setSeg(bar: HTMLElement, cls: string, show: boolean): HTMLElement | null {
  const el = bar.querySelector<HTMLElement>('.' + cls);
  if (!el) return null;
  el.style.display = show ? '' : 'none';
  return show ? el : null;
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
  const sesData = sessions.get(sid);
  const provider: string = entry?.provider || sesData?.provider || '';

  if (!entry && !sesData) {
    bar.style.display = 'none';
    return;
  }

  bar.style.display = 'flex';
  wireClicks(bar);

  const isTokenProvider = provider === 'claude' || provider === 'codex';
  const modelName = entry?.usageModel || sesData?.model || '';

  // ---- #N セッション番号 ----
  const idEl = setSeg(bar, 'tsb-seg-id', true);
  if (idEl) idEl.textContent = `#${sid}`;

  // ---- 状態 pill ----
  const stateKey = (sesData?.state as string) || 'standby';
  const statusEl = setSeg(bar, 'tsb-seg-status', true);
  if (statusEl) {
    statusEl.innerHTML = `<span class="tsb-pill ${pillClassFor(stateKey)}"><span class="tsb-pdot"></span>${escapeHtml(stateLabel(stateKey))}</span>`;
  }

  // ---- provider アイコン+ラベル+モデル ----
  const agentEl = setSeg(bar, 'tsb-seg-agent', !!provider);
  if (agentEl) {
    const chip = `<span class="card-provider-chip ${safeClassToken(provider)}">${escapeHtml(providerDisplayName(provider))}</span>`;
    const model = modelName ? `<span class="tsb-model" title="${escapeHtml(modelName)}">${escapeHtml(modelName)}</span>` : '';
    agentEl.innerHTML = `${providerIconHtml(provider, 13)}${chip}${model}`;
  }

  // ---- 作業ラベル（E）----
  const labelText = (sesData?.label && String(sesData.label).trim())
    || (sesData?.last_message ? String(sesData.last_message).trim() : '');
  const labelEl = setSeg(bar, 'tsb-seg-label', !!labelText);
  if (labelEl) {
    labelEl.textContent = `“${labelText}”`;
    // クリックで送信履歴モーダルを開く導線。本文＋操作ヒントを tooltip に出す。
    labelEl.title = `${labelText}\n${t('tsb_sent_history_hint')}`;
  }

  // ---- project ⎇branch ±git ----
  const project = getProject(sid);
  const branch = sesData?.branch || '';
  const gf = Number((sesData as any)?.git_files || 0);
  const ga = Number((sesData as any)?.git_added || 0);
  const gd = Number((sesData as any)?.git_deleted || 0);
  const projectEl = setSeg(bar, 'tsb-seg-project', !!project);
  if (projectEl) {
    let html = `📁 ${escapeHtml(project)}`;
    if (branch) html += ` <span class="tsb-branch">⎇ ${escapeHtml(branch)}</span>`;
    if (gf > 0 || ga > 0 || gd > 0) {
      const title = `+${ga} -${gd} (${gf} files)`;
      html += ` <span class="tsb-git" title="${escapeHtml(title)}">±${gf} ~${ga + gd}</span>`;
    }
    projectEl.innerHTML = html;
    // クリックで Files タブを開く導線（旧プロジェクトグループ header の「📁 Files」ボタンの代替）。
    projectEl.title = t('files_group_btn_tooltip');
  }

  // ---- ctx 使用率（塗りゲージ + %）----
  // 上限は relay 経由の実値（Claude statusline の context_window_size）を最優先し、
  // 取得できないプロバイダのみモデル名テーブルにフォールバックする。
  const ctxLimit = (entry && entry.ctxWindow > 0) ? entry.ctxWindow : resolveCtxLimit(modelName);
  const showCtx = !!(isTokenProvider && entry && ctxLimit && ctxLimit > 0);
  const ctxEl = setSeg(bar, 'tsb-seg-ctx', showCtx);
  if (ctxEl && entry && ctxLimit) {
    const used = entry.tokensIn;
    const pct = Math.max(0, Math.min(100, Math.round((used / ctxLimit) * 100)));
    const fill = pct >= 90 ? 'crit' : pct >= 80 ? 'warn' : 'ok';
    const pctCls = pct >= 90 ? 'tsb-pct crit' : 'tsb-pct';
    ctxEl.innerHTML = `ctx ${gaugeHtml(pct, fill)}<span class="${pctCls}">${pct}%</span>`;
    ctxEl.title = `${formatTok(used)} / ${formatTok(ctxLimit)} tokens`;
    ctxEl.dataset.copy = `${used}/${ctxLimit}`;
  }

  // ---- tok ↑in ↓out ----
  const showTok = !!(isTokenProvider && entry);
  const tokEl = setSeg(bar, 'tsb-seg-tok', showTok);
  if (tokEl && entry) {
    tokEl.textContent = `tok ↑${formatTok(entry.tokensIn)} ↓${formatTok(entry.tokensOut)}`;
    tokEl.dataset.copy = `in=${entry.tokensIn} out=${entry.tokensOut}`;
  }

  // ---- cache 率（塗りゲージ + %、情報色）----
  const showCache = !!(entry && entry.tokensCache > 0 && entry.tokensIn > 0);
  const cacheEl = setSeg(bar, 'tsb-seg-cache', showCache);
  if (cacheEl && entry) {
    const pct = Math.max(0, Math.min(100, Math.round((entry.tokensCache / entry.tokensIn) * 100)));
    cacheEl.innerHTML = `⛁ ${gaugeHtml(pct, 'cache')}<span class="tsb-pct">${pct}%</span>`;
    cacheEl.title = `cache read ${formatTok(entry.tokensCache)} / ${formatTok(entry.tokensIn)} tokens`;
  }

  // ---- compact 残量（auto-compact 発動目安までの残りトークン）----
  // しきい値は Claude Code が auto-compact を始めるおおよその位置（公式未公開の近似値）。
  const COMPACT_TRIGGER_RATIO = 0.92;
  const showCompact = !!(provider === 'claude' && entry && ctxLimit && ctxLimit > 0 && entry.tokensIn > 0);
  const compactEl = setSeg(bar, 'tsb-seg-compact', showCompact);
  if (compactEl && entry && ctxLimit) {
    const threshold = Math.round(ctxLimit * COMPACT_TRIGGER_RATIO);
    const left = threshold - entry.tokensIn;
    // 到達率 = 現在 tokensIn が「しきい値（=発火点）」のどこまで来たか。100% で auto-compact 発動。
    // バー全体は 0→threshold を表し、満タン＝発火。ctx ゲージ（0→ctxLimit）とは分母が違う点に注意。
    const reachPct = Math.max(0, Math.min(100, Math.round((entry.tokensIn / threshold) * 100)));
    const fill = reachPct >= 90 ? 'crit' : reachPct >= 75 ? 'warn' : 'ok';
    if (left <= 0) {
      compactEl.innerHTML = `⌛ ${gaugeHtml(100, 'crit')}<span class="tsb-pct crit">compact間近</span>`;
    } else {
      const pctCls = reachPct >= 90 ? 'tsb-pct crit' : 'tsb-pct';
      compactEl.innerHTML = `⌛ ${gaugeHtml(reachPct, fill)}<span class="${pctCls}">${reachPct}%</span>`;
    }
    compactEl.title = `auto-compact しきい値到達率 ${reachPct}%（100% で発動）`
      + `\n残り ${formatTok(Math.max(0, left))} tokens`
      + `\nしきい値 ~${Math.round(COMPACT_TRIGGER_RATIO * 100)}% = ${formatTok(threshold)} / ${formatTok(ctxLimit)}、現在 ${formatTok(entry.tokensIn)}`;
    compactEl.dataset.copy = `${Math.max(0, left)}`;
  }

  // ---- cost（+ 本日累計）----
  const showCost = !!(isTokenProvider && entry);
  const costEl = setSeg(bar, 'tsb-seg-cost', showCost);
  if (costEl && entry) {
    let html = escapeHtml(formatCost(entry.costUSD, entry.costKnown));
    const today = todayCostSum();
    if (today.known && today.sum > 0) {
      html += ` <span class="tsb-today">· today ${escapeHtml(formatCost(today.sum, true))}</span>`;
    }
    costEl.innerHTML = html;
  }

  // ---- burn rate ----
  const startedAt = sesData?.started_at || entry?.usageStartedAt || '';
  let showBurn = false;
  if (isTokenProvider && entry && startedAt) {
    const start = Date.parse(startedAt);
    if (!isNaN(start)) {
      const elapsedSec = (Date.now() - start) / 1000;
      if (elapsedSec >= 10) {
        const burnEl = setSeg(bar, 'tsb-seg-burn', true);
        if (burnEl) {
          if (entry.costKnown && entry.costUSD > 0) {
            const perH = entry.costUSD / (elapsedSec / 3600);
            burnEl.textContent = `~$${perH.toFixed(perH >= 1 ? 1 : 2)}/h`;
            showBurn = true;
          } else if (entry.tokensTotal > 0) {
            const perMin = entry.tokensTotal / (elapsedSec / 60);
            burnEl.textContent = `~${formatTok(Math.round(perMin))} tok/min`;
            showBurn = true;
          }
        }
      }
    }
  }
  if (!showBurn) setSeg(bar, 'tsb-seg-burn', false);

  // ---- elapsed（+ 直近の送信/承認から AI が動いている時間）----
  if (stateKey === 'running') {
    if (!turnStartAt.has(sid)) turnStartAt.set(sid, Date.now());
  } else {
    turnStartAt.delete(sid);
  }
  const elapsedEl = setSeg(bar, 'tsb-seg-elapsed', !!startedAt);
  if (elapsedEl) {
    let html = `⏱ ${escapeHtml(formatElapsed(startedAt))}`;
    const ts = latestInteractionAt(sid) || turnStartAt.get(sid);
    if (ts) {
      const turnSec = Math.max(0, Math.floor((Date.now() - ts) / 1000));
      html += ` <span class="tsb-turn" title="${escapeHtml(t('tsb_turn_elapsed_title'))}">· AI ${escapeHtml(formatDurSec(turnSec))}</span>`;
    }
    elapsedEl.innerHTML = html;
  }

  // ---- 接続状態 ----
  const connEl = setSeg(bar, 'tsb-seg-conn', true);
  if (connEl) {
    const st = wsConnectionState();
    const icon = st === 'open' ? '🟢' : st === 'connecting' ? '🟡' : '🔴';
    const label = st === 'open' ? t('tsb_conn_open') : st === 'connecting' ? t('tsb_conn_connecting') : t('tsb_conn_closed');
    connEl.innerHTML = `<span class="tsb-dot">${icon}</span>`;
    connEl.title = label;
  }

  // ---- 横断バッジ（▶run ⏸idle ⚠wait）----
  const fc = fleetCounts();
  const showFleet = fc.running > 0 || fc.standby > 0 || fc.waiting > 0;
  const fleetEl = setSeg(bar, 'tsb-seg-fleet', showFleet);
  if (fleetEl) {
    let html = '';
    if (fc.running > 0) html += `<span class="tsb-run">▶${fc.running}</span>`;
    if (fc.standby > 0) html += `<span class="tsb-idle">⏸${fc.standby}</span>`;
    if (fc.waiting > 0) html += `<span class="tsb-wait" title="${escapeHtml(t('tsb_fleet_jump'))}">⚠${fc.waiting}</span>`;
    fleetEl.innerHTML = html;
  }
}

// ── クリック操作（D）─────────────────────────────────────────────────────────

function wireClicks(bar: HTMLElement): void {
  if (_clickWired) return;
  _clickWired = true;
  bar.addEventListener('click', (ev) => {
    const target = ev.target as HTMLElement;
    // ⚠ → 承認待ちセッションへジャンプ
    if (target.closest('.tsb-wait')) {
      let jumpTo: number | null = null;
      sessions.forEach((s, id) => {
        if (jumpTo === null && (s.state as string) === 'waiting') jumpTo = id;
      });
      if (jumpTo !== null) activateSession(jumpTo);
      return;
    }
    // 📁project → Files タブを開く（アクティブセッションの cwd 起点）
    if (target.closest('.tsb-seg-project')) {
      const sid = activeSessionId;
      if (sid === null) return;
      const cwd = sessions.get(sid)?.cwd;
      if (cwd) FilesTabManager.openFilesTab(sid, getProject(sid), cwd, cwd);
      return;
    }
    // 作業ラベル → 送信履歴モーダル（アクティブセッションの user 送信のみを時系列表示）
    if (target.closest('.tsb-seg-label')) {
      openSentHistoryModal();
      return;
    }
    // cost → 内訳ポップ
    const costSeg = target.closest('.tsb-seg-cost') as HTMLElement | null;
    if (costSeg) {
      toggleCostPopover(costSeg);
      return;
    }
    // ctx / tok → 値をコピー
    const copySeg = target.closest('.tsb-seg-ctx, .tsb-seg-tok') as HTMLElement | null;
    if (copySeg && copySeg.dataset.copy) {
      copyText(copySeg.dataset.copy, copySeg);
      return;
    }
    // … → モバイル時の詳細モーダル
    if (target.closest('.tsb-seg-more')) {
      openDetailModal();
      return;
    }
  });
}

// ── 詳細モーダル（モバイル時 … タップで開く）─────────────────────────────────
// CSS @media でモバイル時に折り畳まれるセグメントを、垂直リストとして再表示する。
// バーの現在 DOM をそのままクローンしてラベルを添える方式で、表示ロジックを二重化しない。

const DETAIL_SEGMENTS: Array<{ cls: string; label: string }> = [
  { cls: 'tsb-seg-id',      label: 'ID' },
  { cls: 'tsb-seg-status',  label: '状態' },
  { cls: 'tsb-seg-agent',   label: 'AI' },
  { cls: 'tsb-seg-label',   label: '作業' },
  { cls: 'tsb-seg-project', label: 'プロジェクト' },
  { cls: 'tsb-seg-ctx',     label: 'ctx' },
  { cls: 'tsb-seg-tok',     label: 'tok' },
  { cls: 'tsb-seg-cache',   label: 'cache' },
  { cls: 'tsb-seg-compact', label: 'compact' },
  { cls: 'tsb-seg-cost',    label: 'cost' },
  { cls: 'tsb-seg-burn',    label: 'burn' },
  { cls: 'tsb-seg-elapsed', label: '経過' },
  { cls: 'tsb-seg-conn',    label: '接続' },
  { cls: 'tsb-seg-fleet',   label: '横断' },
];

let _detailDownHandler: ((e: MouseEvent | TouchEvent) => void) | null = null;
let _detailKeyHandler: ((e: KeyboardEvent) => void) | null = null;

function closeDetailModal(): void {
  const existing = document.getElementById('tsb-detail-modal');
  if (existing) existing.remove();
  if (_detailDownHandler) {
    document.removeEventListener('mousedown', _detailDownHandler, true);
    document.removeEventListener('touchstart', _detailDownHandler, true);
    _detailDownHandler = null;
  }
  if (_detailKeyHandler) {
    document.removeEventListener('keydown', _detailKeyHandler, true);
    _detailKeyHandler = null;
  }
}

function openDetailModal(): void {
  if (document.getElementById('tsb-detail-modal')) { closeDetailModal(); return; }
  const bar = getBar();
  if (!bar) return;

  const overlay = document.createElement('div');
  overlay.id = 'tsb-detail-modal';
  const box = document.createElement('div');
  box.className = 'tsb-detail-box';

  const header = document.createElement('div');
  header.className = 'tsb-detail-header';
  const title = document.createElement('span');
  title.className = 'tsb-detail-title';
  title.textContent = 'セッション詳細';
  header.appendChild(title);
  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'tsb-detail-close';
  closeBtn.textContent = '✕';
  closeBtn.addEventListener('click', closeDetailModal);
  header.appendChild(closeBtn);
  box.appendChild(header);

  const body = document.createElement('div');
  body.className = 'tsb-detail-body';
  for (const { cls, label } of DETAIL_SEGMENTS) {
    const src = bar.querySelector<HTMLElement>('.' + cls);
    if (!src) continue;
    const html = src.innerHTML.trim();
    if (!html || src.style.display === 'none') continue;
    const row = document.createElement('div');
    row.className = 'tsb-detail-row';
    const lab = document.createElement('span');
    lab.className = 'tsb-detail-label';
    lab.textContent = label;
    const val = document.createElement('span');
    val.className = 'tsb-detail-value';
    val.innerHTML = html;
    row.appendChild(lab);
    row.appendChild(val);
    body.appendChild(row);
  }
  box.appendChild(body);
  overlay.appendChild(box);
  document.body.appendChild(overlay);

  _detailDownHandler = (e: MouseEvent | TouchEvent) => {
    const target = e.target as Node;
    if (box.contains(target)) return;
    if ((target as HTMLElement).closest?.('.tsb-seg-more')) return;
    closeDetailModal();
  };
  _detailKeyHandler = (e: KeyboardEvent) => {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    e.stopPropagation();
    closeDetailModal();
  };
  setTimeout(() => {
    if (_detailDownHandler) {
      document.addEventListener('mousedown', _detailDownHandler, true);
      document.addEventListener('touchstart', _detailDownHandler, true);
    }
  }, 0);
  document.addEventListener('keydown', _detailKeyHandler, true);
}

function copyText(text: string, flashEl: HTMLElement): void {
  try {
    navigator.clipboard?.writeText(text);
    flashEl.classList.add('tsb-copied');
    setTimeout(() => flashEl.classList.remove('tsb-copied'), 600);
  } catch (_) { /* clipboard 不可環境は黙ってスキップ */ }
}

let _costPopDocHandler: ((e: MouseEvent) => void) | null = null;
function closeCostPopover(): void {
  const existing = document.getElementById('tsb-cost-pop');
  if (existing) existing.remove();
  if (_costPopDocHandler) {
    document.removeEventListener('mousedown', _costPopDocHandler, true);
    _costPopDocHandler = null;
  }
}

function positionCostPopover(pop: HTMLElement, anchor: HTMLElement): void {
  const r = anchor.getBoundingClientRect();
  const margin = 8;
  const width = pop.offsetWidth || 220;
  const left = Math.max(margin, Math.min(r.right - width, window.innerWidth - width - margin));
  pop.style.left = `${Math.round(left)}px`;
  pop.style.right = 'auto';
  pop.style.bottom = `${Math.max(26, Math.round(window.innerHeight - r.top + 4))}px`;
}

function toggleCostPopover(anchor: HTMLElement): void {
  if (document.getElementById('tsb-cost-pop')) { closeCostPopover(); return; }
  const pop = document.createElement('div');
  pop.id = 'tsb-cost-pop';
  const rows: string[] = [];
  let total = 0;
  usageCache.forEach((e, id) => {
    if (!e.costKnown) return;
    total += e.costUSD;
    const s = sessions.get(id);
    const name = s?.label || s?.model || e.provider || `#${id}`;
    rows.push(`<div class="tsb-pop-row"><span>#${id} ${escapeHtml(String(name))}</span><span>${escapeHtml(formatCost(e.costUSD, true))}</span></div>`);
  });
  if (!rows.length) rows.push(`<div class="tsb-pop-row"><span>${escapeHtml(t('tsb_cost_none'))}</span><span></span></div>`);
  pop.innerHTML =
    `<div class="tsb-pop-title">${escapeHtml(t('tsb_cost_breakdown_title'))}</div>${rows.join('')}` +
    `<div class="tsb-pop-row tsb-pop-total"><span>${escapeHtml(t('tsb_cost_total'))}</span><span>${escapeHtml(formatCost(total, true))}</span></div>`;
  // fixed のステータスバー配下に置くと stacking context の都合で入力欄やライブステータスに
  // 潜ることがあるため、body 直下に出してクリック位置の上へ配置する。
  document.body.appendChild(pop);
  positionCostPopover(pop, anchor);
  // バー外クリックで閉じる。cost セグメント上のクリックは toggle 側に委ねる
  // （ここで閉じると click が再度開いてトグルが効かなくなるため除外）。
  setTimeout(() => {
    _costPopDocHandler = (e: MouseEvent) => {
      const target = e.target as Node;
      if (pop.contains(target)) return;
      if ((target as HTMLElement).closest?.('.tsb-seg-cost')) return;
      closeCostPopover();
    };
    document.addEventListener('mousedown', _costPopDocHandler, true);
  }, 0);
}

// ── 送信履歴モーダル ──────────────────────────────────────────────────────────
// 作業ラベルセグメントのクリックで開く。アクティブセッションのチャット履歴から
// role==='user'（＝ AI への送信内容）だけを抜き出し、日時付きで時系列表示する。
// チャットタブと同じ chatHistory（state）を参照するが、送信内容のみを対象にする。

// ts（epoch ms / ISO 文字列 / Date）を「YYYY-MM-DD HH:MM:SS」のローカル日時に整形。
function formatSentDateTime(ts: any): string {
  const d = (ts instanceof Date) ? ts : new Date(ts);
  if (isNaN(d.getTime())) return '';
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} `
    + `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

// アクティブセッションの送信メッセージ（user role）を時系列（古い→新しい）で返す。
// 本文ありのテキスト送信と、添付のみ（kind==='attach'）の送信の両方を対象にする。
// 添付は saved_path（絶対パス）を持つものだけを拾う（クリックで開けないものは除外）。
function collectSentMessages(sid: number): Array<{ ts: any; text: string; attachments: any[] }> {
  const arr = chatHistory.get(sid) || [];
  const out: Array<{ ts: any; text: string; attachments: any[] }> = [];
  for (const m of arr) {
    if (!m || m.role !== 'user') continue;
    const text = String(m.normalizedText || m.rawText || '').trim();
    const attachments = Array.isArray(m.attachments)
      ? m.attachments.filter((a: any) => a && a.path)
      : [];
    if (!text && attachments.length === 0) continue;
    out.push({ ts: m.ts, text, attachments });
  }
  return out;
}

let _sentModalKeyHandler: ((e: KeyboardEvent) => void) | null = null;
let _sentModalDownHandler: ((e: MouseEvent | TouchEvent) => void) | null = null;

function closeSentHistoryModal(): void {
  const existing = document.getElementById('tsb-sent-modal');
  if (existing) existing.remove();
  if (_sentModalKeyHandler) {
    document.removeEventListener('keydown', _sentModalKeyHandler, true);
    _sentModalKeyHandler = null;
  }
  if (_sentModalDownHandler) {
    document.removeEventListener('mousedown', _sentModalDownHandler, true);
    document.removeEventListener('touchstart', _sentModalDownHandler, true);
    _sentModalDownHandler = null;
  }
}

function openSentHistoryModal(): void {
  // 既に開いていればトグルで閉じる。
  if (document.getElementById('tsb-sent-modal')) { closeSentHistoryModal(); return; }
  const sid = activeSessionId;
  if (sid === null) return;

  const items = collectSentMessages(sid);

  const overlay = document.createElement('div');
  overlay.id = 'tsb-sent-modal';

  const box = document.createElement('div');
  box.className = 'tsb-sent-box';

  // ---- ヘッダ（タイトル + 件数 + 閉じる）----
  const header = document.createElement('div');
  header.className = 'tsb-sent-header';
  const title = document.createElement('span');
  title.className = 'tsb-sent-title';
  title.textContent = t('tsb_sent_history_title');
  header.appendChild(title);
  const count = document.createElement('span');
  count.className = 'tsb-sent-count';
  count.textContent = t('tsb_sent_history_count', { n: items.length });
  header.appendChild(count);
  const closeBtn = document.createElement('button');
  closeBtn.type = 'button';
  closeBtn.className = 'tsb-sent-close';
  closeBtn.textContent = '✕';
  closeBtn.setAttribute('aria-label', t('settings_close'));
  closeBtn.addEventListener('click', closeSentHistoryModal);
  header.appendChild(closeBtn);
  box.appendChild(header);

  // ---- 本文（時系列リスト）----
  const body = document.createElement('div');
  body.className = 'tsb-sent-body';
  if (items.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'tsb-sent-empty';
    empty.textContent = t('tsb_sent_history_empty');
    body.appendChild(empty);
  } else {
    // 直近の送信を上に出すため新しい順（降順）で並べる。# は実際の送信順（古い=#1）を維持する。
    items.slice().reverse().forEach((it, i) => {
      const seq = items.length - i;
      const row = document.createElement('div');
      row.className = 'tsb-sent-row';
      const meta = document.createElement('div');
      meta.className = 'tsb-sent-meta';
      const idx = document.createElement('span');
      idx.className = 'tsb-sent-idx';
      idx.textContent = `#${seq}`;
      const time = document.createElement('span');
      time.className = 'tsb-sent-time';
      time.textContent = formatSentDateTime(it.ts);
      meta.appendChild(idx);
      meta.appendChild(time);
      row.appendChild(meta);
      // 本文（テキスト送信のみ）。クリックでその送信内容をコピー。
      if (it.text) {
        const text = document.createElement('div');
        text.className = 'tsb-sent-text';
        text.textContent = it.text;
        row.title = t('tsb_sent_history_copy_hint');
        // 添付チップのクリックがここまでバブルした場合はコピーしない。
        row.addEventListener('click', (e) => {
          if ((e.target as HTMLElement).closest?.('.tsb-sent-attach-chip')) return;
          copyText(it.text, row);
        });
        row.appendChild(text);
      }
      // 添付（画像・CSV・テキスト等）。クリックで Files と同じ右クリックメニューを開く。
      if (it.attachments.length > 0) {
        const ats = document.createElement('div');
        ats.className = 'tsb-sent-attachments';
        for (const a of it.attachments) {
          const name = a.filename || String(a.path).split(/[\\/]/).pop() || a.path;
          const chip = document.createElement('button');
          chip.type = 'button';
          chip.className = 'tsb-sent-attach-chip';
          chip.textContent = `${a.kind === 'image' ? '🖼' : '📄'} ${name}`;
          chip.title = t('tsb_sent_history_attach_hint');
          chip.addEventListener('click', (e) => {
            e.stopPropagation();
            showPathPopup(a.path, e.clientX, e.clientY, sid, 'file');
          });
          ats.appendChild(chip);
        }
        row.appendChild(ats);
      }
      body.appendChild(row);
    });
  }
  box.appendChild(body);
  overlay.appendChild(box);
  document.body.appendChild(overlay);
  // 新しい順（降順）で並べているので、最新の送信が見える先頭へスクロール。
  body.scrollTop = 0;

  // 外側クリック / Esc で閉じる。
  _sentModalDownHandler = (e: MouseEvent | TouchEvent) => {
    const target = e.target as Node;
    if (box.contains(target)) return;
    if ((target as HTMLElement).closest?.('.tsb-seg-label')) return; // ラベル再クリックは toggle に委ねる
    closeSentHistoryModal();
  };
  _sentModalKeyHandler = (e: KeyboardEvent) => {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    e.stopPropagation();
    closeSentHistoryModal();
  };
  setTimeout(() => {
    if (_sentModalDownHandler) {
      document.addEventListener('mousedown', _sentModalDownHandler, true);
      document.addEventListener('touchstart', _sentModalDownHandler, true);
    }
  }, 0);
  document.addEventListener('keydown', _sentModalKeyHandler, true);
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
    ctxWindow:      m.ctx_window     ?? 0,
    usageModel:     m.usage_model    || '',
    usageStartedAt: m.usage_started_at || '',
  });
  if (sid === activeSessionId) {
    renderStatusbar();
  }
}

/** セッション削除時にキャッシュをクリアする。 */
export function removeUsageCacheEntry(sessionId: number): void {
  usageCache.delete(sessionId);
  turnStartAt.delete(sessionId);
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

// ── 毎秒 tick（経過時間・バーンレート・接続状態の更新）────────────────────────

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
    const tsb = data?.token_statusbar;
    const enabled = tsb == null || tsb.enabled == null ? true : !!tsb.enabled;
    setStatusbarEnabled(enabled);
  } catch (_) {
    setStatusbarEnabled(true);
  }
}
