// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { token, showToast } from './util.js';

// ---- 🌐 外部公開（Tailscale serve）トグル — plan_tailscale-serve-host-toggle.md C1 ----
//
// 「この PC を tailnet から受け入れ可能にする＝tailscale serve を開く」操作は
// ホスト全体の永続状態。📱モバイル接続ウィザード配下に埋もれていたものを、
// ツールバー直下の第一級トグル＋状態ドットへ切り出す。
//
// バックエンドは既存エンドポイントをそのまま再利用（再設計しない）:
//   - GET    /api/mobile-connect/tailscale         … 状態取得（handleTailscaleStatus）
//   - POST   /api/mobile-connect/tailscale/serve   … 有効化（handleTailscaleServeEnable）
//   - DELETE /api/mobile-connect/tailscale/serve   … 停止（handleTailscaleServeDisable）
//
// 状態は共有キャッシュに保持し、トグルと 📱ウィザードの双方が同じ関数経由で
// 読み・操作する（C2 で二重管理を解消）。片方の操作後はリスナー通知で他方を更新。

// Tailscale 自己診断レスポンス。フィールド名は internal/hub/tailscale_handlers.go と厳密一致させる。
export type TailscaleStateName =
  | 'not_installed'
  | 'not_logged_in'
  | 'serve_disabled_on_tailnet'
  | 'serve_inactive'
  | 'ready';

export interface TailscaleStatus {
  state: TailscaleStateName;
  dns_name: string;
  online: boolean;
  hub_port: number;
  serve_command: string;
  serve_off_command: string;
  admin_url?: string;
  https_url?: string;
}

// serve 有効化（POST）レスポンス。
export interface TailscaleServeResult {
  ok: boolean;
  state: TailscaleStateName;
  dns_name: string;
  hub_port: number;
  admin_url?: string;
  https_url?: string;
  allowed_host_added?: boolean;
  allowed_host_hint?: string;
}

// 取得結果。tsconfig は strictNullChecks:false のため discriminated union の
// 真偽値 discriminant では narrowing が効かない。optional フィールド付きの
// フラットな形にして ok を見て分岐する（既存コードベースの方針に合わせる）。
export interface FetchResult {
  ok: boolean;
  status?: TailscaleStatus;
  httpStatus?: number | 'network';
}

export interface ServeResult {
  ok: boolean;
  result?: TailscaleServeResult;
  httpStatus?: number | 'network';
}

// ── 共有状態（トグル ⇄ ウィザード）───────────────────────────────────────────
let cached: TailscaleStatus | null = null;
let inflight: Promise<FetchResult> | null = null;
const listeners = new Set<() => void>();

export function getExposeStatus(): TailscaleStatus | null {
  return cached;
}

// 状態変化（取得成功・有効化・停止）を購読する。返り値で解除。
export function onExposeChange(fn: () => void): () => void {
  listeners.add(fn);
  return () => { listeners.delete(fn); };
}
function notify(): void {
  for (const fn of listeners) { try { fn(); } catch (_) { /* リスナー側の例外は握り潰す */ } }
}

function api(path: string, init?: RequestInit): Promise<Response> {
  const sep = path.includes('?') ? '&' : '?';
  return fetch(`${path}${sep}token=${encodeURIComponent(token || '')}`, { cache: 'no-store', ...init });
}

// GET 状態取得。成功時はキャッシュ更新＋リスナー通知。
export function fetchExposeStatus(force: boolean): Promise<FetchResult> {
  if (cached && !force) return Promise.resolve({ ok: true, status: cached });
  if (inflight && !force) return inflight;
  inflight = (async (): Promise<FetchResult> => {
    try {
      const res = await api('/api/mobile-connect/tailscale');
      if (!res.ok) return { ok: false, httpStatus: res.status };
      cached = await res.json() as TailscaleStatus;
      notify();
      return { ok: true, status: cached };
    } catch (_) {
      return { ok: false, httpStatus: 'network' };
    } finally {
      inflight = null;
    }
  })();
  return inflight;
}

// POST 有効化。成功後に状態を取り直してキャッシュ更新。
export async function enableExpose(): Promise<ServeResult> {
  try {
    const res = await api('/api/mobile-connect/tailscale/serve', { method: 'POST' });
    if (!res.ok) return { ok: false, httpStatus: res.status };
    const result = await res.json() as TailscaleServeResult;
    await fetchExposeStatus(true);
    return { ok: true, result };
  } catch (_) {
    return { ok: false, httpStatus: 'network' };
  }
}

// DELETE 停止。成功後に状態を取り直してキャッシュ更新。
export async function disableExpose(): Promise<{ ok: boolean; httpStatus?: number | 'network' }> {
  try {
    const res = await api('/api/mobile-connect/tailscale/serve', { method: 'DELETE' });
    if (!res.ok) return { ok: false, httpStatus: res.status };
    await fetchExposeStatus(true);
    return { ok: true };
  } catch (_) {
    return { ok: false, httpStatus: 'network' };
  }
}

function httpErrText(httpStatus: number | 'network'): string {
  if (httpStatus === 'network') return t('mobile_connect_error_network');
  if (httpStatus === 401) return t('mobile_connect_error_unauthorized');
  return t('mobile_connect_error_http', { status: String(httpStatus) });
}

// ── ツールバー トグル UI ─────────────────────────────────────────────────────
let busy = false;
let popover: HTMLElement | null = null;
let popDownHandler: ((e: MouseEvent | TouchEvent) => void) | null = null;
let popKeyHandler: ((e: KeyboardEvent) => void) | null = null;

function btnEl(): HTMLButtonElement | null {
  return document.getElementById('expose-btn') as HTMLButtonElement | null;
}
function dotEl(): HTMLElement | null {
  return document.getElementById('expose-dot');
}

// ドット色＋ツールチップ（状態の常時可視化）。
function reflectDot(): void {
  const btn = btnEl();
  const dot = dotEl();
  if (!btn || !dot) return;
  const st = cached?.state;
  let cls = 'expose-dot';
  let label = t('expose_state_loading');
  if (busy) {
    cls += ' expose-dot-busy';
  } else if (st === 'ready') {
    cls += ' expose-dot-ready'; label = t('expose_state_ready');
  } else if (st === 'serve_inactive') {
    cls += ' expose-dot-inactive'; label = t('expose_state_inactive');
  } else if (st === 'serve_disabled_on_tailnet') {
    cls += ' expose-dot-warn'; label = t('expose_state_disabled');
  } else if (st === 'not_logged_in') {
    cls += ' expose-dot-off'; label = t('expose_state_not_logged_in');
  } else if (st === 'not_installed') {
    cls += ' expose-dot-off'; label = t('expose_state_not_installed');
  } else {
    cls += ' expose-dot-off';
  }
  dot.className = cls;
  btn.classList.toggle('expose-on', st === 'ready');
  btn.setAttribute('aria-pressed', st === 'ready' ? 'true' : 'false');
  btn.setAttribute('title', `${t('expose_tooltip')} — ${label}`);
}

function setBusy(v: boolean): void {
  busy = v;
  const btn = btnEl();
  if (btn) btn.disabled = v;
  reflectDot();
}

// inactive→有効化 / ready→停止 をその場で実行。
async function toggleServe(): Promise<void> {
  setBusy(true);
  try {
    if (cached?.state === 'ready') {
      const r = await disableExpose();
      if (r.ok) showToast(t('mobile_connect_ts_stopped_toast'), btnEl() || undefined);
      else showToast(httpErrText(r.httpStatus ?? 'network'), btnEl() || undefined);
    } else {
      const r = await enableExpose();
      if (!r.ok) {
        showToast(httpErrText(r.httpStatus), btnEl() || undefined);
      } else if (!r.result.ok) {
        // serve_disabled_on_tailnet 等。状態を反映し案内ポップオーバーへ。
        if (r.result.state === 'serve_disabled_on_tailnet') showToast(t('mobile_connect_ts_serve_disabled_toast'), btnEl() || undefined);
        else showToast(t('mobile_connect_ts_enable_failed'), btnEl() || undefined);
        openPopover();
      } else {
        showToast(t('expose_enabled_toast'), btnEl() || undefined);
      }
    }
  } finally {
    setBusy(false);
  }
}

// クリック挙動: inactive/ready は直接トグル、それ以外（要対応/未設定）は案内ポップオーバー。
async function onClick(): Promise<void> {
  if (busy) return;
  if (popover) { closePopover(); return; }
  if (!cached) {
    setBusy(true);
    const r = await fetchExposeStatus(false);
    setBusy(false);
    if (!r.ok) { showToast(httpErrText(r.httpStatus), btnEl() || undefined); return; }
  }
  const st = cached?.state;
  if (st === 'serve_inactive' || st === 'ready') {
    void toggleServe();
  } else {
    openPopover();
  }
}

function el(tag: string, opts?: { class?: string; text?: string; html?: string; attrs?: Record<string, string> }): HTMLElement {
  const node = document.createElement(tag);
  if (opts?.class) node.className = opts.class;
  if (opts?.text != null) node.textContent = opts.text;
  if (opts?.html != null) node.innerHTML = opts.html;
  if (opts?.attrs) for (const [k, v] of Object.entries(opts.attrs)) node.setAttribute(k, v);
  return node;
}

// 要ログイン / serve 未有効 / 未導入 時の案内ポップオーバー（admin_url / アプリ導線を再利用）。
function openPopover(): void {
  closePopover();
  const btn = btnEl();
  if (!btn) return;
  const st = cached?.state;
  const pop = el('div', { class: 'expose-pop', attrs: { role: 'dialog', 'aria-label': t('expose_popover_title') } });
  pop.appendChild(el('div', { class: 'expose-pop-title', text: t('expose_popover_title') }));

  if (st === 'not_installed') {
    pop.appendChild(el('div', { class: 'expose-pop-msg', text: t('mobile_connect_ts_not_installed') }));
    pop.appendChild(el('div', { class: 'expose-pop-msg expose-pop-warn', text: t('mobile_connect_ts_degrade_ssh') }));
    appendAppLink(pop);
  } else if (st === 'not_logged_in') {
    pop.appendChild(el('div', { class: 'expose-pop-msg', text: t('mobile_connect_ts_not_logged_in') }));
    appendAppLink(pop);
  } else if (st === 'serve_disabled_on_tailnet') {
    pop.appendChild(el('div', { class: 'expose-pop-msg', text: t('mobile_connect_ts_serve_disabled') }));
    pop.appendChild(el('div', { class: 'expose-pop-msg expose-pop-warn', html: t('mobile_connect_ts_funnel_warn') }));
    if (cached?.admin_url) appendLink(pop, cached.admin_url, '🔧', t('mobile_connect_ts_admin_open'));
  } else {
    pop.appendChild(el('div', { class: 'expose-pop-msg', text: t('expose_state_loading') }));
  }

  const refresh = el('button', { class: 'expose-pop-refresh', text: t('mobile_connect_ts_refresh'), attrs: { type: 'button' } });
  refresh.addEventListener('click', async () => {
    const r = await fetchExposeStatus(true);
    if (!r.ok) { showToast(httpErrText(r.httpStatus), btn); return; }
    // 状態が変わったらポップオーバーを開き直す（toggle 可能になったら閉じてトグルへ）。
    if (cached?.state === 'serve_inactive' || cached?.state === 'ready') closePopover();
    else openPopover();
  });
  pop.appendChild(refresh);

  document.body.appendChild(pop);
  popover = pop;
  positionPopover();

  popDownHandler = (e: MouseEvent | TouchEvent) => {
    const target = e.target as Node;
    if (pop.contains(target)) return;
    if ((target as HTMLElement).closest?.('#expose-btn')) return;
    closePopover();
  };
  popKeyHandler = (e: KeyboardEvent) => { if (e.key === 'Escape') closePopover(); };
  setTimeout(() => {
    if (popDownHandler) {
      document.addEventListener('mousedown', popDownHandler, true);
      document.addEventListener('touchstart', popDownHandler, true);
    }
  }, 0);
  document.addEventListener('keydown', popKeyHandler, true);
}

function positionPopover(): void {
  const btn = btnEl();
  if (!btn || !popover) return;
  const r = btn.getBoundingClientRect();
  popover.style.top = `${Math.round(r.bottom + 6)}px`;
  // 右端はみ出しを避けてボタン左端基準で配置（最大幅は CSS 側）。
  const left = Math.max(8, Math.min(r.left, window.innerWidth - popover.offsetWidth - 8));
  popover.style.left = `${Math.round(left)}px`;
}

function closePopover(): void {
  if (popKeyHandler) { document.removeEventListener('keydown', popKeyHandler, true); popKeyHandler = null; }
  if (popDownHandler) {
    document.removeEventListener('mousedown', popDownHandler, true);
    document.removeEventListener('touchstart', popDownHandler, true);
    popDownHandler = null;
  }
  if (popover) { popover.remove(); popover = null; }
}

function appendAppLink(parent: HTMLElement): void {
  appendLink(parent, 'https://tailscale.com/download', '📲', 'Tailscale tailscale.com/download');
}
function appendLink(parent: HTMLElement, href: string, icon: string, label: string): void {
  const a = el('a', { class: 'expose-pop-link', attrs: { href, target: '_blank', rel: 'noopener noreferrer' } });
  a.appendChild(el('span', { class: 'expose-pop-link-ico', text: icon }));
  a.appendChild(el('span', { text: label }));
  parent.appendChild(a);
}

// ── 初期化（app-entry から呼ぶ）────────────────────────────────────────────────
export function initHostExpose(): void {
  const btn = btnEl();
  if (!btn) return;
  reflectDot();
  btn.addEventListener('click', (e) => { e.stopPropagation(); void onClick(); });
  // 他コンポーネント（📱ウィザード）の操作に追従してドットを更新。
  onExposeChange(() => reflectDot());
  // 起動時に現在状態を 1 回取得してドットへ反映（永続副作用の可視化）。
  void fetchExposeStatus(false).then(() => reflectDot());
}
