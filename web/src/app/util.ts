// --- ESM imports (generated) ---
import { t } from '../i18n.js';

// --- ESM shared token (generated; moved from settings.js) ---
// token は初回ロード時に URL クエリから取得し、localStorage へ退避したうえで
// history.replaceState によりアドレスバーから除去する（ブラウザ履歴・スクショ・
// 画面共有への露出対策）。
//
// localStorage を使うのは「QR/token 付き URL を一度開けば、以降は token 無しの
// 保存 URL（ブックマーク・ホーム画面ショートカット）でも再アクセスできる」運用を
// 成立させるため。sessionStorage はタブ/アプリを閉じると消えるため、スマホで
// 保存 URL を開き直すと token が失われ unauthorized になっていた。
// 端末に token が永続保存される点はトレードオフ（自分専用端末・VPN 網内アクセス前提）。
// Hub の token がローテートしたら保存値は無効になり、再度 token 付き URL が必要。
// SW 側の token キャッシュ（pwa.ts → sw.ts postMessage）は従来通り別系統で保持。
//
// token は cookie に置かない（監査 U05）。ブラウザは 127.0.0.1 の cookie を別のポートの
// サーバーにも送るので、同じ PC の別のプログラムが受け取れてしまうため。API は apiFetch が
// Authorization header で、WebSocket は最初のメッセージで token を送る。header を付けられない
// <img>・<video>・<audio>・ダウンロードのリンクは withAssetTicket() で読み取り専用の券を付ける。
// localStorage が使えないとき（private mode 等）はサインインの画面が sessionStorage に置く。
const TOKEN_STORAGE_KEY = 'many-ai-cli-token';
const ASSET_TICKET_STORAGE_KEY = 'many-ai-cli-asset-ticket';

function initToken(): string | null {
  let tk: string | null = null;
  try {
    const params = new URLSearchParams(location.search);
    tk = params.get('token');
    if (tk) {
      try { localStorage.setItem(TOKEN_STORAGE_KEY, tk); } catch (_) { /* private mode 等は無視 */ }
      params.delete('token');
      const qs = params.toString();
      try { history.replaceState(history.state, '', location.pathname + (qs ? `?${qs}` : '') + location.hash); } catch (_) { /* 失敗しても従来動作 */ }
    } else {
      try { tk = localStorage.getItem(TOKEN_STORAGE_KEY); } catch (_) { /* 取得不可なら null のまま */ }
      if (!tk) {
        try { tk = sessionStorage.getItem(TOKEN_STORAGE_KEY); } catch (_) { /* 取得不可なら null のまま */ }
      }
    }
  } catch (_) { /* URL 解析失敗時は null */ }
  return tk;
}

// token は null を返さない（initToken が見つけられなければ空文字）。
// 生の `?token=${token}` 埋め込みが `?token=null` という不正トークン文字列を
// 送ってしまうと、サーバーの requestToken が cookie 認証へフォールバックできず
// unauthorized になる（特にスマホ/PWA でタブを開き直し sessionStorage が消えた場合）。
// 空文字なら `?token=` となりサーバーが認証 cookie へ正しくフォールバックする。
export const token = initToken() ?? '';

// 未認証のページ遷移に対して Hub が返す再認証ページ（internal/hub/server.go の
// writeReauthPage）は、往復ループを防ぐため sessionStorage に 1 回きりのガードを張る。
// このモジュールが評価された ＝ 本体アプリの HTML が返った ＝ 認証は通っているので、
// ここでガードを外して次の cold launch でまた 1 回試せるようにする。
// キー名は writeReauthPage 側の reauthGuardKey と同じ値。片方だけ変えないこと。
try { sessionStorage.removeItem('many-ai-cli-reauth'); } catch (_) { /* private mode 等は無視 */ }

// apiFetch は Hub の `/api/*` を Authorization header 付きで呼ぶ共通ヘルパ。
// credentials 'same-origin' は遠隔の PIN の cookie を送るために残す。
// URL に `?token=` を付けないので、ブラウザ history / server access log / crash
// report へ token が出ない。
export function apiFetch(path: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers);
  if (token && !headers.has('Authorization')) {
    headers.set('Authorization', `Bearer ${token}`);
  }
  const merged: RequestInit = {
    ...init,
    headers,
    credentials: init.credentials ?? 'same-origin',
  };
  return fetch(path, merged);
}

// ---- 読み取り専用の券（asset ticket）----
// <img>・<video>・<audio>・ダウンロードのリンクは header を付けられないので、Hub の
// 画像・ファイルの GET にだけ通る券を URL に付ける。券は token から作られ 12 時間で切れ、
// token を作り直すと全部使えなくなる。読み込みのたびに待たないよう保存しておき、
// 起動時と 1 時間ごとに取り直す。
const ASSET_ROUTE_PREFIXES = [
  '/api/files-asset?', '/api/files-download?', '/api/memo-images/', '/api/provider-icons/',
  '/api/user-prefs/notify-sound-custom', '/api/avatar',
];

function storedAssetTicket(): string {
  try {
    const stored = localStorage.getItem(ASSET_TICKET_STORAGE_KEY) || '';
    const expiry = Number(stored.split('.')[0]);
    // 期限の 1 分前からは使わない。
    if (stored && Number.isFinite(expiry) && expiry * 1000 > Date.now() + 60_000) return stored;
  } catch (_) { /* private mode 等 */ }
  return '';
}

let assetTicket = token ? storedAssetTicket() : '';

export function withAssetTicket(url: string): string {
  if (!assetTicket || typeof url !== 'string') return url;
  if (!ASSET_ROUTE_PREFIXES.some((prefix) => url.startsWith(prefix))) return url;
  const separator = url.includes('?') ? '&' : '?';
  return `${url}${separator}ticket=${encodeURIComponent(assetTicket)}`;
}

// WebSocket も header を付けられない。券を URL に載せると、Hub は認証済みの待ち枠へ入れる
// （待ち枠を選ぶだけで、接続の認証は最初のメッセージの token で行う）。
export function webSocketTicketQuery(): string {
  return assetTicket ? `?ticket=${encodeURIComponent(assetTicket)}` : '';
}

export async function refreshAssetTicket(): Promise<void> {
  if (!token) return;
  try {
    const res = await apiFetch('/api/asset-ticket');
    if (!res.ok) return;
    const data = await res.json();
    if (typeof data?.ticket !== 'string' || !data.ticket) return;
    assetTicket = data.ticket;
    try { localStorage.setItem(ASSET_TICKET_STORAGE_KEY, data.ticket); } catch (_) { /* private mode 等 */ }
  } catch (_) { /* 次の取り直しで再試行する */ }
}

// token を持たずに開いたとき（保存が消えた・別のブラウザ）は、Hub が token 無しを許して
// いない限り、何もできない画面のまま黙らず、token 付きの URL を開くよう伝える。
function showSignedOut(): void {
  const box = document.createElement('div');
  box.className = 'signed-out-notice';
  box.setAttribute('role', 'alert');
  // 言語のファイルがまだ読めていなければ英語の文を出す。
  const text = (key: string, fallback: string) => {
    const value = t(key);
    return value && value !== key ? value : fallback;
  };
  const title = document.createElement('p');
  title.textContent = text('signed_out_title', 'This browser is not signed in to the Hub.');
  const body = document.createElement('p');
  body.textContent = text(
    'signed_out_body',
    'Open the dashboard from many-ai-cli (or the tray), or open the Hub URL that includes ?token= once. Settings > Mobile connect shows a QR code for it.',
  );
  box.append(title, body);
  document.body.replaceChildren(box);
}

if (token) {
  void refreshAssetTicket();
  setInterval(() => { void refreshAssetTicket(); }, 60 * 60 * 1000);
} else {
  fetch('/api/auth/status', { credentials: 'same-origin' })
    .then((res) => { if (res.status === 401) showSignedOut(); })
    .catch(() => { /* Hub に届かないときは接続の表示に任せる */ });
}

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- トースト通知 ----
export let _toastTimer: ReturnType<typeof setTimeout> | null = null;

export type ToastAnchor = Element | MouseEvent | { clientX: number; clientY: number } | null | undefined;

export function getToastAnchorRect(anchor: ToastAnchor): DOMRect | { left: number; right: number; top: number; height: number } | null {
  if (!anchor) return null;
  if (anchor instanceof Element && typeof anchor.getBoundingClientRect === 'function') {
    // 非表示（display:none）やデタッチ済みの要素は getClientRects() が空になる。
    // その場合は全 0 の矩形が返り、トーストが左上に貼り付くため、
    // アンカー無し扱いにして既定の下中央表示へフォールバックさせる。
    if (anchor.getClientRects().length === 0) return null;
    return anchor.getBoundingClientRect();
  }
  if ('clientX' in anchor && 'clientY' in anchor && typeof anchor.clientX === 'number' && typeof anchor.clientY === 'number') {
    return {
      left: anchor.clientX,
      right: anchor.clientX,
      top: anchor.clientY,
      height: 0,
    };
  }
  return null;
}

export function showToast(msg: string, anchor?: ToastAnchor, durationMs = 1800): void {
  let el = document.getElementById('toast');
  if (!el) {
    el = document.createElement('div');
    el.id = 'toast';
    document.body.appendChild(el);
  }
  el.textContent = msg;
  // トーストは常に画面下中央へ固定表示する。anchor 引数は呼び出し側の後方互換の
  // ため残すが、位置決めには使わない（ボタン隣表示はボタンごとに縦位置が変わり
  // 「バラバラ」に見えるため、全体で下中央に統一した）。
  void anchor;
  el.style.left = '';
  el.style.top = '';
  el.style.bottom = '';
  el.classList.remove('toast--anchored');
  el.classList.add('show');
  if (_toastTimer) clearTimeout(_toastTimer);
  _toastTimer = setTimeout(() => { el.classList.remove('show'); el.classList.remove('toast--anchored'); }, durationMs);
}

// i18n フォールバックヘルパ: t() がキー文字列をそのまま返した（未登録）場合に
// fallback を返す。t()自体は key を確実に返す仕様なので、簡易判定で問題ない。
export function ti18n(key: string, fallback?: string | null, vars?: Record<string, unknown>): string {
  const v = window.t ? window.t(key, vars) : key;
  return (v === key && fallback != null) ? fallback : v;
}

export function escapeHtml(str: unknown): string {
  return String(str).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;');
}

export function formatStartedAt(isoStr?: string | null): string {
  if (!isoStr) return '';
  const d = new Date(isoStr);
  if (isNaN(d.getTime())) return '';
  const elapsed = Math.max(0, Math.floor((Date.now() - d.getTime()) / 1000));
  const p = (n: number) => String(n).padStart(2, '0');
  const s = elapsed % 60;
  const m = Math.floor(elapsed / 60) % 60;
  const h = Math.floor(elapsed / 3600);
  return h > 0 ? `[${h}:${p(m)}:${p(s)}]` : `[${p(m)}:${p(s)}]`;
}

export function formatLastOutputAt(isoStr?: string | null): string {
  if (!isoStr) return '';
  const d = new Date(isoStr);
  if (isNaN(d.getTime())) return '';
  const p = (n: number) => String(n).padStart(2, '0');
  const now = new Date();
  const sameDay = d.getFullYear() === now.getFullYear() &&
                  d.getMonth() === now.getMonth() &&
                  d.getDate() === now.getDate();
  const time = `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  if (sameDay) return time;
  let DOW;
  try { DOW = JSON.parse(t('dow')); }
  catch { DOW = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']; }
  return `${d.getFullYear()}-${p(d.getMonth()+1)}-${p(d.getDate())}(${DOW[d.getDay()]}) ${time}`;
}

export function cleanCopiedText(linesOrText: string | string[] | null | undefined): string {
  const lines = Array.isArray(linesOrText)
    ? linesOrText.slice()
    : String(linesOrText || '').replace(/\r\n?/g, '\n').split('\n');
  const cleaned = lines.map(line => String(line || '').replace(/[ \t]+$/g, ''));

  while (cleaned.length > 0 && cleaned[0].trim() === '') cleaned.shift();
  while (cleaned.length > 0 && cleaned[cleaned.length - 1].trim() === '') cleaned.pop();

  const indents = cleaned
    .filter(line => line.trim() !== '')
    .map(line => (line.match(/^[ \t]*/) || [''])[0].length);
  const minIndent = indents.length > 0 ? Math.min(...indents) : 0;
  if (minIndent <= 0) return cleaned.join('\n');

  return cleaned
    .map(line => line.trim() === '' ? '' : line.slice(minIndent))
    .join('\n');
}

// コピー操作のアイコンはアプリ全体でこの 1 組に揃える（📋 → 押した直後だけ ✓）。
// 新しいコピーボタンは別の記号（⧉ ⎘ 等）や文字ラベルを使わず、これを使う。
export const COPY_ICON = '📋';
export const COPIED_ICON = '✓';

/** コピーボタンの表示を一瞬 ✓ にして、ms 後に 📋 へ戻す。 */
export function flashCopied(btn: HTMLElement, ms = 1000): void {
  btn.textContent = COPIED_ICON;
  btn.classList.add('copied');
  setTimeout(() => { btn.textContent = COPY_ICON; btn.classList.remove('copied'); }, ms);
}

export async function copyCleanText(linesOrText: string | string[] | null | undefined, anchor?: ToastAnchor): Promise<void> {
  const text = cleanCopiedText(linesOrText);
  if (!text) return;
  await navigator.clipboard.writeText(text);
  showToast(t('copied_to_clipboard'), anchor);
}

// TUI がハード改行で折り返した複数行コマンドを 1 行に戻す。
// 折り返しは単語境界（スペース位置）で起きるため、各行 trim → スペース 1 個で join する。
// xterm.js の soft wrap（isWrapped 行）は getSelection() が連結済みなのでここには来ない。
export function cleanOneLineText(text: string | null | undefined): string {
  return String(text || '')
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map(line => line.trim())
    .filter(line => line !== '')
    .join(' ');
}

export async function copyOneLineText(text: string | null | undefined, anchor?: ToastAnchor): Promise<void> {
  const oneLine = cleanOneLineText(text);
  if (!oneLine) return;
  await navigator.clipboard.writeText(oneLine);
  showToast(t('copied_to_clipboard'), anchor);
}

// ── DOM ヘルパ ────────────────────────────────────────────────────────────────
// `document.createElement` に class / text / html / attrs を一括で渡す薄いラッパー。
// mobile-connect.ts / host-expose.ts に同じ定義が並んでいたので集約した。
// generics 版で受けるとタグごとの正しい戻り型（HTMLDivElement 等）が伝播する。
export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  opts?: { class?: string; text?: string; html?: string; attrs?: Record<string, string> },
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (opts?.class) node.className = opts.class;
  if (opts?.text != null) node.textContent = opts.text;
  if (opts?.html != null) node.innerHTML = opts.html;
  if (opts?.attrs) for (const [k, v] of Object.entries(opts.attrs)) node.setAttribute(k, v);
  return node;
}

// --- ESM window-interop publish (generated; preserves dynamic window.* lookups) ---
if (typeof window !== 'undefined') window.showToast = showToast;
