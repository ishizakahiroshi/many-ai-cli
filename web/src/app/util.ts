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
const TOKEN_STORAGE_KEY = 'many-ai-cli-token';

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

// --- ESM window-interop publish (generated; preserves dynamic window.* lookups) ---
window.showToast = showToast;
