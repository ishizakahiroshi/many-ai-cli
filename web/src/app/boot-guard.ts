// 画面の初期化（モジュールの評価）が途中の例外で止まったとき、「読み込み中...」のまま黙って
// 止めずに、読み込めなかったことと再読み込みの案内を画面に出す。
//
// なぜ要るか:
//   web/src は循環 import を含む。循環の中のモジュールが読み込みの瞬間に同期で相手の let を読むと、
//   TDZ の ReferenceError でモジュールグラフ全体の評価が止まる（2c25878 の app.ts、d6ef87b の
//   approval.ts）。以前はこの例外を受け取る所が無く、画面は「読み込み中...」のまま、ボタンも
//   ツアーも反応しなかった。
//
// 決まり（scripts/check-web-module-init.mjs が CI で配線を確かめる）:
//   - app-entry.ts の最初の import をこのモジュールにする。評価の順番が最初になり、ほかの
//     モジュールの評価中に出た例外も受け取れる。そのため、このモジュールは何も import しない
//   - app-entry.ts の最後の文で markAppEntryEvaluated() を呼ぶ。そこまで評価が届いたら初期化は
//     完了とみなし、以後の例外では何も出さない（普段の操作中のエラーで「読み込めなかった」と出さない）
//   - 例外の中身（スタック）は画面に出さず、コンソールへ出す
//
// 受け取れないもの: モジュールの取得の失敗と構文エラー。このモジュールも同じグラフの一部なので、
// そのときは評価されない（構文と import 先の欠けは、tsc と build が CI で止める）。

interface BootFailureText {
  message: string;
  detail: string;
  reload: string;
}

// 辞書が読めなかったときだけ使う。文言の正本は i18n の 3 言語の辞書（boot_failed_*）。
const FALLBACK_TEXT: BootFailureText = {
  message: 'The page failed to load. Please reload it.',
  detail: 'The cause has been written to the browser console.',
  reload: 'Reload',
};

// 辞書が届かないまま待ち続けないための上限。
const I18N_WAIT_MS = 3000;

let appEntryEvaluated = false;
let decisionPending = false;
let failureShown = false;
let i18nReady = false;
const reportedErrors: unknown[] = [];

function onWindowError(ev: ErrorEvent): void {
  noteError(ev.error ?? ev.message);
}

function onUnhandledRejection(ev: PromiseRejectionEvent): void {
  noteError(ev.reason);
}

function noteError(reason: unknown): void {
  if (appEntryEvaluated || failureShown) return;
  reportedErrors.push(reason);
  if (decisionPending) return;
  decisionPending = true;
  // 評価が止まったかどうかは、次のタスクで決める。モジュールグラフの評価は同期で一気に進む
  // （トップレベルの await が無い前提）ので、評価の途中で報告された例外でも、評価が最後まで
  // 届いていれば、その時点で markAppEntryEvaluated() が呼ばれている。
  setTimeout(decide, 0);
}

function decide(): void {
  decisionPending = false;
  if (appEntryEvaluated) {
    reportedErrors.length = 0;
    return;
  }
  failureShown = true;
  detach();
  console.error('[boot] The page stopped initializing before it finished. Errors reported while loading, in order:');
  reportedErrors.forEach((reason, i) => console.error(`[boot] #${i + 1}`, reason));
  showFailure();
}

function detach(): void {
  window.removeEventListener('error', onWindowError);
  window.removeEventListener('unhandledrejection', onUnhandledRejection);
}

function translate(key: string, fallback: string): string {
  const text = typeof window.t === 'function' ? window.t(key) : '';
  return text && text !== key ? text : fallback;
}

function showFailure(): void {
  if (i18nReady) {
    render();
    return;
  }
  // 辞書が届いてから出す（届く前に出すと、日本語の画面でも一瞬英語が出る）。i18n-ready の
  // ほかの受け手（ヘッダーの文言を書き換えるものがある）より後に書くため、マイクロタスクへ遅らせる。
  let rendered = false;
  const renderOnce = () => {
    if (rendered) return;
    rendered = true;
    render();
  };
  document.addEventListener('i18n-ready', () => queueMicrotask(renderOnce), { once: true });
  setTimeout(renderOnce, I18N_WAIT_MS);
}

function render(): void {
  const text: BootFailureText = {
    message: translate('boot_failed_message', FALLBACK_TEXT.message),
    detail: translate('boot_failed_detail', FALLBACK_TEXT.detail),
    reload: translate('boot_failed_reload', FALLBACK_TEXT.reload),
  };

  // ヘッダーの「読み込み中...」を置き換える。data-i18n を外すのは、辞書の適用で戻されないため。
  const summary = document.getElementById('summary');
  if (summary) {
    summary.removeAttribute('data-i18n');
    summary.textContent = text.message;
  }

  const bar = document.createElement('div');
  bar.id = 'boot-failure';
  bar.setAttribute('role', 'alert');
  Object.assign(bar.style, {
    position: 'fixed',
    top: '0',
    left: '0',
    right: '0',
    zIndex: '2147483647',
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '6px 12px',
    boxSizing: 'border-box',
    padding: '10px 14px',
    paddingTop: 'max(10px, env(safe-area-inset-top))',
    background: '#7f1d1d',
    color: '#fff',
    borderBottom: '1px solid #ef4444',
    font: '14px/1.5 system-ui, sans-serif',
  });

  const message = document.createElement('span');
  message.textContent = text.message;
  message.style.fontWeight = '600';

  const detail = document.createElement('span');
  detail.textContent = text.detail;
  detail.style.opacity = '0.85';

  const reload = document.createElement('button');
  reload.type = 'button';
  reload.textContent = text.reload;
  Object.assign(reload.style, {
    padding: '4px 14px',
    border: '1px solid rgba(255, 255, 255, 0.8)',
    borderRadius: '4px',
    background: 'transparent',
    color: 'inherit',
    font: 'inherit',
    cursor: 'pointer',
  });
  reload.addEventListener('click', () => location.reload());

  bar.append(message, detail, reload);
  (document.body ?? document.documentElement).appendChild(bar);
}

/** app-entry.ts の最後の文から呼ぶ。ここまで評価が届いたら、初期化は完了。 */
export function markAppEntryEvaluated(): void {
  appEntryEvaluated = true;
  detach();
}

window.addEventListener('error', onWindowError);
window.addEventListener('unhandledrejection', onUnhandledRejection);
document.addEventListener('i18n-ready', () => { i18nReady = true; }, { once: true });
