// Hub の token と読み取り専用の券を、ブラウザのどこに置くかを決める（監査 N24）。
//
// PC の手元の画面（origin が 127.0.0.1 / localhost / ::1 で、PC のブラウザ）では、タブの中
// （sessionStorage）にだけ置く。ブラウザを閉じても残る置き場（localStorage）は origin ごとで、
// origin はポートまでしか見ない。Hub が止まっている間や、別のポートへずれて起動した後に、
// 同じ PC の別の利用者がそのポートで待ち受けると、そこが返すページは同じ origin なので、
// 残っている値を読める。タブの中の値は、タブを閉じれば消える。再読み込みと、Hub を再起動
// した後の再接続は同じタブなので今までどおり動く。新しいタブやブックマークからは開けない
// ので、many-ai-cli（tray の「Hub を開く」など）から開く。開くたびに 1 回限りの番号で token
// を受け取る。
//
// スマホ・タブレットと、外からの画面（origin が 127.0.0.1 以外）は今までどおり localStorage
// に置く。「token 付きの URL や QR を一度開けば、保存した URL やホーム画面から開き直せる」
// 使い方のため。スマホは README の手順どおり SSH の転送でスマホ自身の 127.0.0.1 を開くので、
// origin だけでは PC の手元の画面と見分けられない。そこでブラウザの種類も見る。スマホは
// 1 人で使う端末で、「同じ PC の別の利用者がポートを取る」相手がいない。
//
// sw.ts（Service Worker）と rust/src/hub/reauth.html（サインインの画面）は、この module を
// 読み込めないので、同じ判定を自分で持っている。変えるときは 3 か所を揃える。
export const TOKEN_STORAGE_KEY = 'many-ai-cli-token';
export const ASSET_TICKET_STORAGE_KEY = 'many-ai-cli-asset-ticket';

export type KeyValueStore = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export interface TokenStores {
  /** ブラウザを閉じても残る置き場（localStorage）。使えなければ null。 */
  lasting: KeyValueStore | null;
  /** タブを閉じると消える置き場（sessionStorage）。使えなければ null。 */
  tab: KeyValueStore | null;
}

/** 画面の hostname（`location.hostname`）が、この PC 自身を指すか。 */
export function isLoopbackHost(hostname: string): boolean {
  const host = String(hostname || '').trim().toLowerCase()
    .replace(/^\[/, '').replace(/\]$/, '').replace(/\.$/, '');
  if (host === 'localhost' || host.endsWith('.localhost') || host === '::1') return true;
  const parts = host.split('.');
  return parts.length === 4 && parts[0] === '127'
    && parts.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255);
}

/** ブラウザの種類を見分けるのに使う値（`navigator` の一部）。 */
export interface BrowserKind {
  userAgent?: string;
  platform?: string;
  maxTouchPoints?: number;
  userAgentData?: { mobile?: boolean };
}

/** スマホ・タブレットのブラウザか。iPadOS の Safari は Mac と名乗るので、タッチの点数も見る。 */
export function isHandheld(browser: BrowserKind | null | undefined): boolean {
  if (!browser) return false;
  if (browser.userAgentData?.mobile === true) return true;
  if (/Android|iPhone|iPad|iPod/i.test(String(browser.userAgent || ''))) return true;
  return browser.platform === 'MacIntel' && Number(browser.maxTouchPoints) > 1;
}

/** token をタブの中にだけ置く画面か（PC の手元の画面）。 */
export function keepsTokenInTabOnly(hostname: string, browser: BrowserKind | null | undefined): boolean {
  return isLoopbackHost(hostname) && !isHandheld(browser);
}

function read(store: KeyValueStore | null, key: string): string {
  try { return store?.getItem(key) || ''; } catch (_) { return ''; }
}

function write(store: KeyValueStore | null, key: string, value: string): boolean {
  try {
    if (!store) return false;
    store.setItem(key, value);
    return true;
  } catch (_) { return false; }
}

function drop(store: KeyValueStore | null, key: string): void {
  try { store?.removeItem(key); } catch (_) { /* 使えない置き場には残っていない */ }
}

/**
 * 画面を開いたときの token を決め、置き場を整える。`urlToken` は URL の `?token=`（無ければ空）。
 * `tabOnly` は keepsTokenInTabOnly() の答え。見つからなければ空文字。
 */
export function loadToken(urlToken: string, tabOnly: boolean, stores: TokenStores): string {
  if (tabOnly) {
    // 前の版が localStorage に残した値は、この 1 回だけ使い、タブの中へ移して消す。
    const left = read(stores.lasting, TOKEN_STORAGE_KEY);
    drop(stores.lasting, TOKEN_STORAGE_KEY);
    drop(stores.lasting, ASSET_TICKET_STORAGE_KEY);
    const found = urlToken || read(stores.tab, TOKEN_STORAGE_KEY) || left;
    // タブの中にも置けないとき（private mode 等）は、このページの間だけ持つ。
    if (found) write(stores.tab, TOKEN_STORAGE_KEY, found);
    return found;
  }
  if (urlToken) {
    write(stores.lasting, TOKEN_STORAGE_KEY, urlToken);
    return urlToken;
  }
  return read(stores.lasting, TOKEN_STORAGE_KEY) || read(stores.tab, TOKEN_STORAGE_KEY);
}

/** 読み取り専用の券の置き場。PC の手元の画面ではタブの中、それ以外は localStorage。 */
export function ticketStore(tabOnly: boolean, stores: TokenStores): KeyValueStore | null {
  return tabOnly ? stores.tab : stores.lasting;
}

/** いまのブラウザの置き場。触れただけで例外になる環境（private mode 等）では null。 */
export function browserStores(): TokenStores {
  let lasting: KeyValueStore | null = null;
  let tab: KeyValueStore | null = null;
  try { lasting = localStorage; } catch (_) { /* 使えない */ }
  try { tab = sessionStorage; } catch (_) { /* 使えない */ }
  return { lasting, tab };
}

/** いまの画面が、token をタブの中にだけ置く画面か。画面の外（試験など）では false。 */
export function currentPageKeepsTokenInTabOnly(): boolean {
  try { return keepsTokenInTabOnly(location.hostname, navigator); } catch (_) { return false; }
}
