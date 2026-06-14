// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { token, showToast } from './util.js';
// serve 状態の取得は外部公開トグルと共有（plan_tailscale-serve-host-toggle.md C2）。
// 有効化/停止操作はツールバーの 🌐外部公開へ一本化したため、ここでは状態取得のみ参照する。
import { fetchExposeStatus, type TailscaleStatus, type TailscaleStateName } from './host-expose.js';

// ---- 📱 モバイル接続ウィザード（QR / SSH / VPN）----
// docs/local/plan_mobile-qr-ssh-tunnel.md C3 / docs/local/mockup-mobile-connect.html を基準に実装。
//
// セキュリティ方針:
//  - Hub の bind は 127.0.0.1 固定（変更しない）。スマホは SSH ローカルフォワード / VPN 経由で到達する。
//  - S7: 初回はリスク同意ゲート。同意状態は localStorage に保存し2回目以降スキップ（再表示リンクあり）。
//  - S7: 同意後も「token入りQR=パスワード相当・共有禁止」の警告バーを常時表示。
//  - S1: token 入り QR は click-to-reveal（最初ぼかし、タップで表示）。
//  - SEC-D: 表示した token QR は数秒経過 or document.hidden（タブ背面/離席）で自動再ぼかし。
//  - WireGuard 設定はクライアントサイドで QR 化のみ。サーバーには送らない。

// localStorage キー（device-local）。
const LS_CONSENT = 'ai_cli_hub_mobile_connect_consent';     // '1' = 同意済み
const LS_PATTERN = 'ai_cli_hub_mobile_connect_pattern';     // 'ssh' | 'vpn' | 'done'
const LS_PLATFORM = 'ai_cli_hub_mobile_connect_platform';   // 'ios' | 'android'
const LS_VPN_KIND = 'ai_cli_hub_mobile_connect_vpn_kind';   // 'tailscale' | 'wireguard'

// token 入り QR を自動再ぼかしするまでの猶予（SEC-D）。
const REVEAL_TIMEOUT_MS = 30000;

interface MobileConnectInfo {
  lan_ip: string;
  ssh_user: string;
  ssh_port: number;
  hub_port: number;
  ssh_command: string;
  ssh_url: string;
  hub_url: string;
  ssh_forward_url?: string;
  // SSH 経路: OpenSSH Server 状態（C2 backend）。
  sshd_state?: string;
  sshd_installed?: boolean;
  sshd_running?: boolean;
}

// Tailscale 自己診断の型（TailscaleStatus / TailscaleStateName）は host-expose.ts に集約済み。

type Pattern = 'ssh' | 'vpn' | 'done';
type Platform = 'ios' | 'android';
// VPN 経路は Tailscale を第一推奨（D2）。WireGuard は「上級」として畳む（D4）。
type VpnKind = 'tailscale' | 'wireguard';

interface WizardState {
  consented: boolean;
  pattern: Pattern | null;
  platform: Platform | null;
  vpnKind: VpnKind;
  step: 'connect' | null;
  help: boolean;
  reveal: boolean;       // token QR を表示中か（click-to-reveal）
  wgConfig: string;      // WireGuard 設定（クライアントサイドのみ・サーバー送信なし）
  wgServerIp: string;    // WG サーバ IP（ユーザー入力・D4: ダミー撤廃）
}

const APP_LINKS = {
  ssh: { name: 'Termius', url: 'termius.com', dl: 'https://termius.com/' },
  tailscale: { name: 'Tailscale', url: 'tailscale.com/download', dl: 'https://tailscale.com/download' },
  wireguard: { name: 'WireGuard', url: 'wireguard.com/install', dl: 'https://www.wireguard.com/install/' },
};

let modalOpen = false;
let info: MobileConnectInfo | null = null;
let loadError = '';
// Tailscale 自己診断の最新スナップショット（VPN/Tailscale 経路の状態駆動 UI 用）。
let tsStatus: TailscaleStatus | null = null;
let tsLoading = false;
let tsError = '';
// env_kind（A2/IP-4: メッセージ出し分け）。/api/info から取得し、無ければ 'local'。
let envKind = 'local';
let state: WizardState = freshState();
let revealTimer: ReturnType<typeof setTimeout> | null = null;
let keyHandler: ((e: KeyboardEvent) => void) | null = null;
let downHandler: ((e: MouseEvent | TouchEvent) => void) | null = null;
let visibilityHandler: (() => void) | null = null;

function freshState(): WizardState {
  return {
    consented: lsGet(LS_CONSENT) === '1',
    pattern: (lsGet(LS_PATTERN) as Pattern) || null,
    platform: (lsGet(LS_PLATFORM) as Platform) || null,
    vpnKind: (lsGet(LS_VPN_KIND) as VpnKind) || 'tailscale',
    step: null,
    help: false,
    reveal: false,
    wgConfig: '',
    wgServerIp: '',
  };
}

function lsGet(key: string): string | null {
  try { return localStorage.getItem(key); } catch (_) { return null; }
}
function lsSet(key: string, value: string): void {
  try { localStorage.setItem(key, value); } catch (_) { /* private mode 等は無視 */ }
}

// ── DOM ヘルパ ────────────────────────────────────────────────────────────────

function el<K extends keyof HTMLElementTagNameMap>(
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

// ── QR 描画（vendored qrcode-generator）────────────────────────────────────────
// typeNumber=0（自動）, errorCorrectionLevel 'M'。長い文字列でも収まるよう自動サイズ。
function makeQrCanvas(data: string, sizePx = 148): HTMLElement {
  try {
    if (typeof qrcode !== 'function') throw new Error('qrcode unavailable');
    const qr = qrcode(0, 'M');
    qr.addData(data);
    qr.make();
    const count = qr.getModuleCount();
    const canvas = document.createElement('canvas');
    const margin = 2; // モジュール単位の余白
    const total = count + margin * 2;
    const cell = Math.max(1, Math.floor(sizePx / total));
    const dim = cell * total;
    canvas.width = dim;
    canvas.height = dim;
    canvas.className = 'mc-qr-canvas';
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('2d context unavailable');
    ctx.fillStyle = '#ffffff';
    ctx.fillRect(0, 0, dim, dim);
    ctx.fillStyle = '#000000';
    for (let row = 0; row < count; row++) {
      for (let col = 0; col < count; col++) {
        if (qr.isDark(row, col)) {
          ctx.fillRect((col + margin) * cell, (row + margin) * cell, cell, cell);
        }
      }
    }
    return canvas;
  } catch (_) {
    return el('div', { class: 'mc-qr-fallback', text: t('mobile_connect_qr_failed') });
  }
}

// QR ブロック（任意で click-to-reveal）。reveal=true のときは secret 相当として最初ぼかす。
function qrBlock(data: string, caption: string, secret: boolean): HTMLElement {
  const wrap = el('div', { class: 'mc-qrwrap' });
  const qrBox = el('div', { class: 'mc-qr' });
  qrBox.appendChild(makeQrCanvas(data));

  if (secret && !state.reveal) {
    qrBox.classList.add('mc-qr-blur');
    const hint = el('button', { class: 'mc-reveal-hint', text: t('mobile_connect_reveal_hint'), attrs: { type: 'button' } });
    const doReveal = () => { state.reveal = true; armRevealTimer(); render(); };
    qrBox.addEventListener('click', doReveal);
    hint.addEventListener('click', (e) => { e.stopPropagation(); doReveal(); });
    wrap.append(qrBox, hint);
  } else {
    wrap.appendChild(qrBox);
  }
  wrap.appendChild(el('div', { class: 'mc-cap', text: caption }));
  return wrap;
}

// SEC-D: reveal タイマー（数秒で自動再ぼかし）。
function armRevealTimer(): void {
  clearRevealTimer();
  revealTimer = setTimeout(() => {
    if (state.reveal) { state.reveal = false; if (modalOpen) render(); }
  }, REVEAL_TIMEOUT_MS);
}
function clearRevealTimer(): void {
  if (revealTimer) { clearTimeout(revealTimer); revealTimer = null; }
}

// コピー + フィードバック。
function copyCmd(text: string, anchor?: Element): void {
  if (navigator.clipboard?.writeText) {
    navigator.clipboard.writeText(text).then(
      () => showToast(t('mobile_connect_copied'), anchor),
      () => showToast(t('mobile_connect_copy_failed'), anchor),
    );
  } else {
    showToast(t('mobile_connect_copy_failed'), anchor);
  }
}

// コマンド行（コピー可能）。
function cmdRow(text: string): HTMLElement {
  const row = el('div', { class: 'mc-cmd' });
  row.appendChild(el('span', { class: 'mc-cmd-text', text }));
  const btn = el('button', { class: 'mc-cmd-copy', text: t('mobile_connect_copy'), attrs: { type: 'button' } });
  btn.addEventListener('click', () => copyCmd(text, btn));
  row.appendChild(btn);
  return row;
}

// 機能ステータスバー（G1 トレードオフ可視化: 🔔通知 / 🎤音声 / 📲ホーム追加）。
function featBar(secure: boolean): HTMLElement {
  const bar = el('div', { class: 'mc-feat' });
  const cell = (icon: string, label: string) => {
    const c = el('div', { class: 'mc-feat-cell ' + (secure ? 'ok' : 'ng') });
    c.appendChild(el('span', { class: 'mc-feat-ic', text: secure ? icon : '🚫' }));
    c.appendChild(el('span', { text: label }));
    return c;
  };
  bar.append(
    cell('🔔', t('mobile_connect_feat_notify')),
    cell('🎤', t('mobile_connect_feat_voice')),
    cell('📲', t('mobile_connect_feat_home')),
  );
  return bar;
}

function ctxNote(secure: boolean, extraKey?: string): HTMLElement {
  const note = el('div', { class: 'mc-ctx-note ' + (secure ? 'mc-ctx-ok' : 'mc-ctx-ng') });
  note.textContent = (secure ? t('mobile_connect_ctx_secure') : t('mobile_connect_ctx_insecure'))
    + (extraKey ? ' ' + t(extraKey) : '');
  return note;
}

// ステップインジケータ（0..4）。
function stepDots(active: number): HTMLElement {
  const steps = el('div', { class: 'mc-steps' });
  for (let i = 0; i <= 4; i++) {
    steps.appendChild(el('span', { class: 'mc-step-dot' + (i <= active ? ' on' : '') }));
  }
  return steps;
}

function choiceBtn(icon: string, name: string, desc: string, badge: { text: string; cls: string } | null, onClick: () => void): HTMLElement {
  const btn = el('button', { class: 'mc-choice', attrs: { type: 'button' } });
  btn.appendChild(el('span', { class: 'mc-choice-ico', text: icon }));
  const tcol = el('span', { class: 'mc-choice-t' });
  tcol.appendChild(el('span', { class: 'mc-choice-nm', text: name }));
  tcol.appendChild(el('span', { class: 'mc-choice-ds', text: desc }));
  if (badge) tcol.appendChild(el('span', { class: 'mc-badge ' + badge.cls, text: badge.text }));
  btn.appendChild(tcol);
  btn.addEventListener('click', onClick);
  return btn;
}

function navRow(opts: { back?: () => void; primary?: { text: string; onClick: () => void }; backText?: string }): HTMLElement {
  const nav = el('div', { class: 'mc-nav' });
  if (opts.back) {
    const b = el('button', { class: 'mc-nav-btn', text: opts.backText || t('mobile_connect_back'), attrs: { type: 'button' } });
    b.addEventListener('click', opts.back);
    nav.appendChild(b);
  } else {
    nav.appendChild(el('span'));
  }
  if (opts.primary) {
    const p = el('button', { class: 'mc-nav-btn mc-nav-primary', text: opts.primary.text, attrs: { type: 'button' } });
    p.addEventListener('click', opts.primary.onClick);
    nav.appendChild(p);
  } else {
    nav.appendChild(el('span'));
  }
  return nav;
}

// ── レンダリング ──────────────────────────────────────────────────────────────

function bodyEl(): HTMLElement | null {
  return document.getElementById('mobile-connect-body');
}

function render(): void {
  const body = bodyEl();
  const warnbar = document.getElementById('mobile-connect-warnbar');
  if (!body) return;
  body.innerHTML = '';

  // 警告バーは同意後に常時表示（S7）。
  if (warnbar) warnbar.hidden = !state.consented;

  // エラー / 空状態（UX-E）。
  if (loadError) {
    body.appendChild(stepDots(0));
    body.appendChild(el('div', { class: 'mc-q', text: t('mobile_connect_error_title') }));
    body.appendChild(el('div', { class: 'mc-error', text: loadError }));
    body.appendChild(navRow({
      primary: { text: t('mobile_connect_retry'), onClick: () => { void loadInfo(true); } },
    }));
    return;
  }

  // ステップ0: リスク同意ゲート（S7）。
  if (!state.consented) {
    renderConsent(body);
    return;
  }

  // 情報未取得（読込中）。
  if (!info) {
    body.appendChild(stepDots(0));
    body.appendChild(el('div', { class: 'mc-loading', text: t('mobile_connect_loading') }));
    return;
  }

  if (state.help) { renderHelp(body); return; }
  if (!state.pattern) { renderPattern(body); return; }
  if (state.pattern === 'done') { renderDone(body); return; }
  if (!state.platform) { renderPlatform(body); return; }
  if (state.step !== 'connect') { renderAppInstall(body); return; }
  renderConnect(body);
}

function renderConsent(body: HTMLElement): void {
  body.appendChild(stepDots(0));
  body.appendChild(el('div', { class: 'mc-q', text: t('mobile_connect_consent_title') }));
  body.appendChild(el('div', { class: 'mc-risk', html: t('mobile_connect_consent_body') }));

  const label = el('label', { class: 'mc-chk' });
  const chk = el('input', { attrs: { type: 'checkbox', id: 'mobile-connect-agree' } }) as HTMLInputElement;
  label.append(chk, el('span', { text: t('mobile_connect_consent_agree') }));
  body.appendChild(label);

  const goBtn = el('button', { class: 'mc-nav-btn mc-nav-primary', text: t('mobile_connect_consent_proceed'), attrs: { type: 'button' } }) as HTMLButtonElement;
  goBtn.disabled = true;
  chk.addEventListener('change', () => { goBtn.disabled = !chk.checked; });
  goBtn.addEventListener('click', () => {
    state.consented = true;
    lsSet(LS_CONSENT, '1');
    if (!info && !loadError) void loadInfo(false);
    render();
  });
  const nav = el('div', { class: 'mc-nav' });
  nav.append(el('span'), goBtn);
  body.appendChild(nav);
  setTimeout(() => chk.focus(), 0);
}

function renderHelp(body: HTMLElement): void {
  body.appendChild(stepDots(1));
  body.appendChild(el('div', { class: 'mc-q', text: t('mobile_connect_help_title') }));
  body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_help_sub') }));

  const table = el('table', { class: 'mc-cmp' });
  const rows: string[][] = [
    [t('mobile_connect_help_col_method'), t('mobile_connect_help_col_effort'), t('mobile_connect_help_col_external'), t('mobile_connect_help_col_features')],
    // D3: 生IP（VPN 直）経路は撤廃。Tailscale serve は HTTPS 一本（secure context・フル機能）。
    [t('mobile_connect_help_ssh'), t('mobile_connect_help_effort_mid'), t('mobile_connect_help_ssh_external'), t('mobile_connect_help_full')],
    [t('mobile_connect_help_vpn_https'), t('mobile_connect_help_effort_low'), '◎', t('mobile_connect_help_full')],
  ];
  rows.forEach((cells, ri) => {
    const tr = el('tr');
    cells.forEach((c, ci) => {
      const cell = el(ri === 0 ? 'th' : 'td', { text: c });
      if (ri !== 0 && ci === 0) cell.className = 'mc-cmp-h';
      tr.appendChild(cell);
    });
    table.appendChild(tr);
  });
  body.appendChild(table);
  body.appendChild(ctxNote(true, 'mobile_connect_help_tip'));
  body.appendChild(navRow({ back: () => { state.help = false; render(); } }));
}

function renderPattern(body: HTMLElement): void {
  body.appendChild(stepDots(1));
  const qRow = el('div', { class: 'mc-q mc-q-row' });
  qRow.appendChild(el('span', { text: t('mobile_connect_q1') }));
  const helpBtn = el('button', { class: 'mc-link', text: t('mobile_connect_which'), attrs: { type: 'button' } });
  helpBtn.addEventListener('click', () => { state.help = true; render(); });
  qRow.appendChild(helpBtn);
  body.appendChild(qRow);

  body.appendChild(choiceBtn('🏠', t('mobile_connect_opt_ssh_name'), t('mobile_connect_opt_ssh_desc'),
    { text: t('mobile_connect_badge_full'), cls: 'mc-badge-full' }, () => pick('ssh')));
  body.appendChild(choiceBtn('🌍', t('mobile_connect_opt_vpn_name'), t('mobile_connect_opt_vpn_desc'),
    { text: t('mobile_connect_badge_full'), cls: 'mc-badge-full' }, () => pick('vpn')));
  body.appendChild(choiceBtn('✅', t('mobile_connect_opt_done_name'), t('mobile_connect_opt_done_desc'),
    { text: t('mobile_connect_badge_keep'), cls: 'mc-badge-keep' }, () => pick('done')));

  body.appendChild(navRow({ back: () => resetWizard(), backText: t('mobile_connect_to_start') }));
}

function renderDone(body: HTMLElement): void {
  if (!info) return;
  body.appendChild(stepDots(4));
  body.appendChild(el('div', { class: 'mc-q', text: t('mobile_connect_done_title') }));
  body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_done_sub') }));
  body.appendChild(featBar(true));
  body.appendChild(ctxNote(true, 'mobile_connect_done_ctx_extra'));
  body.appendChild(qrBlock(info.hub_url, t('mobile_connect_url_cap'), true));
  body.appendChild(navRow({
    back: () => { changePattern(); },
    backText: t('mobile_connect_change_method'),
    primary: { text: t('mobile_connect_done'), onClick: () => closeModal() },
  }));
}

function renderPlatform(body: HTMLElement): void {
  body.appendChild(stepDots(2));
  body.appendChild(el('div', { class: 'mc-q', text: t('mobile_connect_q2') }));
  body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_q2_sub') }));
  const row = el('div', { class: 'mc-row2' });
  const pickPlat = (p: Platform) => { state.platform = p; lsSet(LS_PLATFORM, p); render(); };
  const ios = choiceBtn('🍎', t('mobile_connect_iphone'), '', null, () => pickPlat('ios'));
  const android = choiceBtn('🤖', t('mobile_connect_android'), '', null, () => pickPlat('android'));
  ios.classList.add('mc-choice-col');
  android.classList.add('mc-choice-col');
  row.append(ios, android);
  body.appendChild(row);
  body.appendChild(navRow({ back: () => { state.pattern = null; render(); } }));
}

function renderAppInstall(body: HTMLElement): void {
  body.appendChild(stepDots(3));
  body.appendChild(el('div', { class: 'mc-q', text: t('mobile_connect_q3') }));

  if (state.pattern === 'vpn') {
    const seg = el('div', { class: 'mc-seg' });
    const mk = (kind: VpnKind, label: string) => {
      const b = el('button', { class: 'mc-seg-btn' + (state.vpnKind === kind ? ' on' : ''), text: label, attrs: { type: 'button' } });
      b.addEventListener('click', () => { state.vpnKind = kind; lsSet(LS_VPN_KIND, kind); render(); });
      return b;
    };
    seg.append(mk('tailscale', t('mobile_connect_vpn_tailscale')), mk('wireguard', t('mobile_connect_vpn_wireguard')));
    body.appendChild(seg);
  }

  const app = state.pattern === 'ssh' ? APP_LINKS.ssh : APP_LINKS[state.vpnKind];
  body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_q3_sub', { os: state.platform === 'ios' ? 'iPhone' : 'Android' }) }));
  body.appendChild(qrBlock('https://' + app.url, app.url, false));

  const link = el('a', { class: 'mc-applink', attrs: { href: app.dl, target: '_blank', rel: 'noopener noreferrer' } });
  link.appendChild(el('span', { class: 'mc-applink-ico', text: '📲' }));
  const lcol = el('div');
  lcol.appendChild(el('div', { text: app.name }));
  lcol.appendChild(el('div', { class: 'mc-applink-url', text: app.url }));
  link.appendChild(lcol);
  body.appendChild(link);

  body.appendChild(navRow({
    back: () => { state.platform = null; render(); },
    primary: { text: t('mobile_connect_installed_next'), onClick: () => { state.step = 'connect'; render(); } },
  }));
}

function renderConnect(body: HTMLElement): void {
  if (!info) return;
  body.appendChild(stepDots(4));

  if (state.pattern === 'ssh') {
    renderConnectSSH(body);
    return;
  }

  // VPN: Tailscale を第一推奨として前面に（D2）。
  // セグメントで Tailscale（推奨）/ WireGuard（上級）を切替。
  const seg = el('div', { class: 'mc-seg' });
  const mkKind = (kind: VpnKind, label: string) => {
    const b = el('button', { class: 'mc-seg-btn' + (state.vpnKind === kind ? ' on' : ''), text: label, attrs: { type: 'button' } });
    b.addEventListener('click', () => {
      if (state.vpnKind === kind) return;
      state.vpnKind = kind;
      lsSet(LS_VPN_KIND, kind);
      state.reveal = false;
      if (kind === 'tailscale' && !tsStatus) void loadTailscale(false);
      render();
    });
    return b;
  };
  seg.append(
    mkKind('tailscale', t('mobile_connect_vpn_tailscale')),
    mkKind('wireguard', t('mobile_connect_vpn_wireguard')),
  );
  body.appendChild(seg);

  if (state.vpnKind === 'tailscale') {
    renderConnectTailscale(body);
  } else {
    renderConnectWireguard(body);
  }
}

// ── SSH 経路（上級・接続後の 127.0.0.1 QR は正しい唯一の経路として維持）──────────
function renderConnectSSH(body: HTMLElement): void {
  if (!info) return;
  body.appendChild(el('div', { class: 'mc-q', text: t('mobile_connect_connect_ssh_title') }));
  body.appendChild(featBar(true));
  body.appendChild(ctxNote(true, 'mobile_connect_ssh_ctx_extra'));

  // C2: OpenSSH Server 状態を表示。未導入/停止なら有効化導線。
  appendSshdState(body);

  body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_ssh_steps') }));
  body.appendChild(qrBlock(info.ssh_url, t('mobile_connect_ssh_host_cap', { url: info.ssh_url }), false));
  body.appendChild(cmdRow(info.ssh_command));
  body.appendChild(el('div', { class: 'mc-cap mc-cap-warn', text: t('mobile_connect_ssh_note') }));
  body.appendChild(el('hr', { class: 'mc-hr' }));
  const forwardURL = info.ssh_forward_url || info.hub_url;
  body.appendChild(qrBlock(forwardURL, t('mobile_connect_ssh_url_cap'), true));
  appendNotifyTest(body);
  appendHomeHint(body);
  body.appendChild(navRow({
    back: () => { state.step = null; state.reveal = false; render(); },
    primary: { text: t('mobile_connect_done'), onClick: () => closeModal() },
  }));
}

// C2: OpenSSH Server 状態バッジ＋未導入時の有効化導線。
function appendSshdState(body: HTMLElement): void {
  const st = info?.sshd_state || 'unknown';
  let cls = 'mc-status mc-status-warn';
  let txt = t('mobile_connect_sshd_unknown');
  if (st === 'running') { cls = 'mc-status mc-status-ok'; txt = t('mobile_connect_sshd_running'); }
  else if (st === 'stopped') { cls = 'mc-status mc-status-warn'; txt = t('mobile_connect_sshd_stopped'); }
  else if (st === 'not_installed') { cls = 'mc-status mc-status-ng'; txt = t('mobile_connect_sshd_not_installed'); }
  body.appendChild(el('div', { class: cls, text: txt }));
  if (st === 'not_installed' || st === 'stopped') {
    body.appendChild(el('div', { class: 'mc-cap mc-cap-warn', text: t('mobile_connect_sshd_enable_hint') }));
  }
}

// ── Tailscale 経路（第一推奨・状態駆動）────────────────────────────────────────
function renderConnectTailscale(body: HTMLElement): void {
  body.appendChild(el('div', { class: 'mc-q', text: t('mobile_connect_connect_ts_title') }));

  // A2/IP-4: env_kind 連動メッセージ。remote/remote-tunnel ではリモート側で serve が動く旨を明示。
  if (envKind === 'remote' || envKind === 'remote-tunnel') {
    body.appendChild(el('div', { class: 'mc-ts-remote-note', text: t('mobile_connect_ts_remote_note') }));
  }

  if (tsLoading && !tsStatus) {
    body.appendChild(el('div', { class: 'mc-loading', text: t('mobile_connect_ts_loading') }));
    appendTailscaleNav(body);
    return;
  }
  if (tsError) {
    body.appendChild(el('div', { class: 'mc-error', text: tsError }));
    body.appendChild(navRow({
      back: () => { state.step = null; state.reveal = false; render(); },
      primary: { text: t('mobile_connect_retry'), onClick: () => { void loadTailscale(true); } },
    }));
    return;
  }
  if (!tsStatus) {
    // 未取得（初回）。取得をキックして loading 表示。
    void loadTailscale(false);
    body.appendChild(el('div', { class: 'mc-loading', text: t('mobile_connect_ts_loading') }));
    appendTailscaleNav(body);
    return;
  }

  const st = tsStatus;
  // ライブ状態バッジ。
  body.appendChild(tsStateBadge(st.state));

  switch (st.state) {
    case 'not_installed':
      body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_ts_not_installed') }));
      // CLI 不在（Docker/degrade 含む）→ SSH トンネル/launcher へ誘導。
      body.appendChild(el('div', { class: 'mc-ctx-note mc-ctx-ng', text: t('mobile_connect_ts_degrade_ssh') }));
      appendTailscaleAppLink(body);
      appendSwitchToSshLink(body);
      break;
    case 'not_logged_in':
      body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_ts_not_logged_in') }));
      appendTailscaleAppLink(body);
      break;
    case 'serve_disabled_on_tailnet':
      body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_ts_serve_disabled') }));
      // Funnel 警告（チェックを外す）を明示。
      body.appendChild(el('div', { class: 'mc-ts-funnel-warn', html: t('mobile_connect_ts_funnel_warn') }));
      if (st.admin_url) {
        const a = el('a', { class: 'mc-applink', attrs: { href: st.admin_url, target: '_blank', rel: 'noopener noreferrer' } });
        a.appendChild(el('span', { class: 'mc-applink-ico', text: '🔧' }));
        const c = el('div');
        c.appendChild(el('div', { text: t('mobile_connect_ts_admin_open') }));
        c.appendChild(el('div', { class: 'mc-applink-url', text: st.admin_url }));
        a.appendChild(c);
        body.appendChild(a);
      }
      // 有効化後の再取得導線。
      appendTailscaleRefreshBtn(body);
      break;
    case 'serve_inactive':
      body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_ts_serve_inactive') }));
      // C2: 有効化操作はツールバーの 🌐外部公開トグルへ一本化（二重管理の解消）。
      // ここでは導線案内＋等価コマンド（情報）のみ残す。
      body.appendChild(el('div', { class: 'mc-ctx-note mc-ctx-ng', text: t('mobile_connect_ts_toolbar_hint') }));
      body.appendChild(el('div', { class: 'mc-cap', text: t('mobile_connect_ts_equiv_cmd') }));
      body.appendChild(cmdRow(st.serve_command));
      break;
    case 'ready':
      // 本物 https_url の QR（click-to-reveal＋SEC-D 自動再ぼかし）。QR はスマホ接続の本来価値なのでウィザードに残す。
      body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_ts_ready_sub') }));
      body.appendChild(featBar(true));
      body.appendChild(ctxNote(true, 'mobile_connect_ts_ctx_https'));
      if (st.https_url) {
        body.appendChild(qrBlock(st.https_url, t('mobile_connect_ts_url_cap'), true));
      } else {
        body.appendChild(el('div', { class: 'mc-error', text: t('mobile_connect_ts_no_url') }));
      }
      appendNotifyTest(body);
      appendHomeHint(body);
      // C2: 公開の停止もツールバーの 🌐外部公開トグルへ一本化（ウィザードからは停止ボタンを撤去）。
      body.appendChild(el('div', { class: 'mc-cap mc-cap-warn', text: t('mobile_connect_ts_toolbar_stop_hint') }));
      break;
    default:
      body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_ts_loading') }));
      break;
  }

  appendTailscaleNav(body);
}

function tsStateBadge(stateName: TailscaleStateName): HTMLElement {
  let cls = 'mc-status mc-status-warn';
  let label = '';
  switch (stateName) {
    case 'ready': cls = 'mc-status mc-status-ok'; label = t('mobile_connect_ts_badge_ready'); break;
    case 'serve_inactive': cls = 'mc-status mc-status-warn'; label = t('mobile_connect_ts_badge_serve_inactive'); break;
    case 'serve_disabled_on_tailnet': cls = 'mc-status mc-status-warn'; label = t('mobile_connect_ts_badge_serve_disabled'); break;
    case 'not_logged_in': cls = 'mc-status mc-status-ng'; label = t('mobile_connect_ts_badge_not_logged_in'); break;
    case 'not_installed': cls = 'mc-status mc-status-ng'; label = t('mobile_connect_ts_badge_not_installed'); break;
  }
  return el('div', { class: cls, text: label });
}

function appendTailscaleAppLink(body: HTMLElement): void {
  const app = APP_LINKS.tailscale;
  const link = el('a', { class: 'mc-applink', attrs: { href: app.dl, target: '_blank', rel: 'noopener noreferrer' } });
  link.appendChild(el('span', { class: 'mc-applink-ico', text: '📲' }));
  const lcol = el('div');
  lcol.appendChild(el('div', { text: app.name }));
  lcol.appendChild(el('div', { class: 'mc-applink-url', text: app.url }));
  link.appendChild(lcol);
  body.appendChild(link);
}

function appendSwitchToSshLink(body: HTMLElement): void {
  const row = el('div', { class: 'mc-notify-test' });
  const btn = el('button', { class: 'mc-link', text: t('mobile_connect_ts_switch_ssh'), attrs: { type: 'button' } });
  btn.addEventListener('click', () => { state.pattern = 'ssh'; state.reveal = false; lsSet(LS_PATTERN, 'ssh'); render(); });
  row.appendChild(btn);
  body.appendChild(row);
}

function appendTailscaleRefreshBtn(body: HTMLElement): void {
  const row = el('div', { class: 'mc-notify-test' });
  const btn = el('button', { class: 'mc-link', text: t('mobile_connect_ts_refresh'), attrs: { type: 'button' } });
  btn.addEventListener('click', () => { void loadTailscale(true); });
  row.appendChild(btn);
  body.appendChild(row);
}

function appendTailscaleNav(body: HTMLElement): void {
  body.appendChild(navRow({
    back: () => { state.step = null; state.reveal = false; render(); },
    primary: { text: t('mobile_connect_done'), onClick: () => closeModal() },
  }));
}

// ── WireGuard 経路（上級・D4: ペースト→クライアントサイド QR は維持）─────────────
function renderConnectWireguard(body: HTMLElement): void {
  if (!info) return;
  body.appendChild(el('div', { class: 'mc-q', text: t('mobile_connect_connect_wg_title') }));
  body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_wg_paste_sub') }));
  const ta = el('textarea', { class: 'mc-textarea', attrs: { placeholder: '[Interface]\nPrivateKey = ...\nAddress = ...\n[Peer]\n...' } }) as HTMLTextAreaElement;
  ta.value = state.wgConfig;
  ta.addEventListener('input', () => { state.wgConfig = ta.value; refreshWgQr(); });
  body.appendChild(ta);
  const wgWrap = el('div', { class: 'mc-wg-qr' });
  body.appendChild(wgWrap);
  renderWgQrInto(wgWrap);

  body.appendChild(el('hr', { class: 'mc-hr' }));
  body.appendChild(el('div', { class: 'mc-sub', text: t('mobile_connect_wg_after') }));

  // D4: ダミー URL 撤廃。WG サーバ IP をユーザーが入力して本物 URL を組み立てる。
  const ipInput = el('input', {
    class: 'mc-input',
    attrs: { type: 'text', placeholder: t('mobile_connect_wg_ip_placeholder'), value: state.wgServerIp },
  }) as HTMLInputElement;
  ipInput.addEventListener('input', () => { state.wgServerIp = ipInput.value.trim(); state.reveal = false; refreshWgUrlQr(); });
  body.appendChild(ipInput);

  const urlWrap = el('div', { class: 'mc-wg-url-qr' });
  body.appendChild(urlWrap);
  renderWgUrlQrInto(urlWrap);

  body.appendChild(el('div', { class: 'mc-cap mc-cap-warn', text: t('mobile_connect_wg_url_note') }));

  body.appendChild(navRow({
    back: () => { state.step = null; state.reveal = false; render(); },
    primary: { text: t('mobile_connect_done'), onClick: () => closeModal() },
  }));
}

// WireGuard QR は textarea 入力に追従して個別更新（全体 render を避ける）。
function refreshWgQr(): void {
  const wrap = document.querySelector('.mc-wg-qr') as HTMLElement | null;
  if (wrap) renderWgQrInto(wrap);
}
function renderWgQrInto(wrap: HTMLElement): void {
  wrap.innerHTML = '';
  const cfg = state.wgConfig.trim();
  if (!cfg) {
    wrap.appendChild(el('div', { class: 'mc-cap', text: t('mobile_connect_wg_placeholder') }));
    return;
  }
  // クライアントサイドのみで QR 化。サーバーには送らない（S4）。
  wrap.appendChild(qrBlock(cfg, t('mobile_connect_wg_qr_cap'), true));
}

// D4: WG サーバ IP 入力に追従して「接続後 URL」QR を個別更新（ダミー撤廃）。
function refreshWgUrlQr(): void {
  const wrap = document.querySelector('.mc-wg-url-qr') as HTMLElement | null;
  if (wrap) renderWgUrlQrInto(wrap);
}
function renderWgUrlQrInto(wrap: HTMLElement): void {
  wrap.innerHTML = '';
  const ip = state.wgServerIp.trim();
  if (!ip || !info) {
    wrap.appendChild(el('div', { class: 'mc-cap', text: t('mobile_connect_wg_ip_prompt') }));
    return;
  }
  // ユーザー入力の WG サーバ IP で本物 URL を組み立てる（token は info 由来・既存と同水準）。
  const tokenQuery = extractTokenQuery();
  const url = `http://${ip}:${info.hub_port}/${tokenQuery}`;
  wrap.appendChild(qrBlock(url, t('mobile_connect_wg_url_cap'), true));
}

// info.hub_url（= http://127.0.0.1:<port>/?token=…）から ?token=… のクエリ部を取り出す。
// token を JS 側で組み立てず、サーバーが返した URL のクエリを再利用する。
function extractTokenQuery(): string {
  const src = info?.ssh_forward_url || info?.hub_url || '';
  const idx = src.indexOf('?');
  return idx >= 0 ? src.slice(idx) : '';
}

// UX-C: 通知テスト。push 購読状態を案内する簡易導線。
function appendNotifyTest(body: HTMLElement): void {
  const row = el('div', { class: 'mc-notify-test' });
  const btn = el('button', { class: 'mc-link', text: t('mobile_connect_notify_test'), attrs: { type: 'button' } });
  btn.addEventListener('click', async () => {
    try {
      if (!('Notification' in window)) {
        showToast(t('mobile_connect_notify_unsupported'), btn);
        return;
      }
      let perm = Notification.permission;
      if (perm === 'default') perm = await Notification.requestPermission();
      if (perm !== 'granted') {
        showToast(t('mobile_connect_notify_denied'), btn);
        return;
      }
      new Notification('many-ai-cli', { body: t('mobile_connect_notify_body') });
      showToast(t('mobile_connect_notify_sent'), btn);
    } catch (_) {
      showToast(t('mobile_connect_notify_unsupported'), btn);
    }
  });
  row.appendChild(btn);
  body.appendChild(row);
}

// UX-B: ホーム画面に追加（OS別）。
function appendHomeHint(body: HTMLElement): void {
  const key = state.platform === 'android' ? 'mobile_connect_home_android' : 'mobile_connect_home_ios';
  // platform 未確定（done パターン等）の場合は両方の短い案内。
  const text = state.platform ? t(key) : t('mobile_connect_home_generic');
  body.appendChild(el('div', { class: 'mc-home-hint', text }));
}

// ── 状態遷移 ──────────────────────────────────────────────────────────────────

function pick(p: Pattern): void {
  state.pattern = p;
  state.reveal = false;
  lsSet(LS_PATTERN, p);
  render();
}

// UX-A: 「方法を変える」で①へ戻る（記憶した pattern/platform をクリア）。
function changePattern(): void {
  state.pattern = null;
  state.platform = null;
  state.step = null;
  state.reveal = false;
  render();
}

function resetWizard(): void {
  state = freshState();
  state.consented = true; // 同意は維持（最初へ＝①へ）
  state.pattern = null;
  state.platform = null;
  render();
}

// ── データ取得 ────────────────────────────────────────────────────────────────

async function loadInfo(force: boolean): Promise<void> {
  if (info && !force) return;
  loadError = '';
  info = null;
  if (modalOpen) render();
  try {
    const res = await fetch(`/api/mobile-connect?token=${encodeURIComponent(token || '')}`, { cache: 'no-store' });
    if (!res.ok) {
      loadError = res.status === 401
        ? t('mobile_connect_error_unauthorized')
        : t('mobile_connect_error_http', { status: String(res.status) });
      if (modalOpen) render();
      return;
    }
    const data = await res.json() as MobileConnectInfo;
    info = data;
    if (!data.lan_ip) {
      loadError = t('mobile_connect_error_no_lan_ip');
    }
  } catch (_) {
    loadError = t('mobile_connect_error_network');
  }
  if (modalOpen) render();
}

// A2/IP-4: env_kind を /api/info から取得（メッセージ出し分け用）。失敗時は 'local' 据え置き。
async function loadEnvKind(): Promise<void> {
  try {
    const res = await fetch(`/api/info?token=${encodeURIComponent(token || '')}`, { cache: 'no-store' });
    if (!res.ok) return;
    const data = await res.json() as { env_kind?: string };
    const raw = String(data?.env_kind || '').trim().toLowerCase().replace(/_/g, '-');
    if (raw === 'remote' || raw === 'remote-tunnel' || raw === 'wsl' || raw === 'local') envKind = raw;
  } catch (_) { /* 取得失敗は local 据え置き */ }
}

// ── Tailscale 自己診断の取得・serve 操作（C1 backend エンドポイント）─────────────

// 状態取得は host-expose.ts の共有関数経由（serve の有効化/停止はツールバーへ一本化）。
// ツールバー側で公開を on/off した結果もこの共有キャッシュに反映されるため、
// ウィザードを次に開いた/再取得したときに最新状態が出る。
async function loadTailscale(force: boolean): Promise<void> {
  if (tsLoading) return;
  if (tsStatus && !force) return;
  tsLoading = true;
  tsError = '';
  if (force) tsStatus = null;
  if (modalOpen && state.vpnKind === 'tailscale') render();
  const r = await fetchExposeStatus(force);
  if (r.ok) {
    tsStatus = r.status;
  } else {
    tsError = r.httpStatus === 'network'
      ? t('mobile_connect_error_network')
      : r.httpStatus === 401
        ? t('mobile_connect_error_unauthorized')
        : t('mobile_connect_error_http', { status: String(r.httpStatus) });
  }
  tsLoading = false;
  if (modalOpen && state.vpnKind === 'tailscale') render();
}

// ── モーダル開閉 ──────────────────────────────────────────────────────────────

function openModal(): void {
  const modal = document.getElementById('mobile-connect-modal');
  const btn = document.getElementById('mobile-connect-btn');
  if (!modal) return;
  modal.hidden = false;
  modalOpen = true;
  btn?.setAttribute('aria-expanded', 'true');

  // 2回目以降（UX-A）: 同意済み + pattern 記憶があれば④へ直行。
  state = freshState();
  if (state.consented && state.pattern && state.pattern !== 'done') {
    if (state.pattern === 'ssh') {
      if (state.platform) state.step = 'connect';
    } else if (state.pattern === 'vpn') {
      if (state.platform) state.step = 'connect';
    }
  }

  void loadInfo(false);
  void loadEnvKind();
  render();

  downHandler = (e: MouseEvent | TouchEvent) => {
    const target = e.target as Node;
    const box = modal.querySelector('.mc-box');
    if (box && box.contains(target)) return;
    if ((target as HTMLElement).closest?.('#mobile-connect-btn')) return;
    closeModal();
  };
  keyHandler = (e: KeyboardEvent) => {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    e.stopPropagation();
    closeModal();
  };
  // SEC-D: タブ背面/離席で即ぼかし。
  visibilityHandler = () => {
    if (document.hidden && state.reveal) { state.reveal = false; if (modalOpen) render(); }
  };
  setTimeout(() => {
    if (downHandler) {
      document.addEventListener('mousedown', downHandler, true);
      document.addEventListener('touchstart', downHandler, true);
    }
  }, 0);
  document.addEventListener('keydown', keyHandler, true);
  document.addEventListener('visibilitychange', visibilityHandler);

  // 初期フォーカス（UX-D）。
  setTimeout(() => {
    const focusTarget = modal.querySelector('.mc-box input, .mc-box button:not(.mc-modal-close)') as HTMLElement | null;
    (focusTarget || (modal.querySelector('.mc-modal-close') as HTMLElement | null))?.focus();
  }, 0);
}

function closeModal(): void {
  const modal = document.getElementById('mobile-connect-modal');
  const btn = document.getElementById('mobile-connect-btn');
  modalOpen = false;
  state.reveal = false;
  // ready 時の本物 QR を背面に残さないよう Tailscale スナップショットを破棄。
  tsStatus = null;
  tsError = '';
  clearRevealTimer();
  if (modal) modal.hidden = true;
  btn?.setAttribute('aria-expanded', 'false');
  if (keyHandler) { document.removeEventListener('keydown', keyHandler, true); keyHandler = null; }
  if (visibilityHandler) { document.removeEventListener('visibilitychange', visibilityHandler); visibilityHandler = null; }
  if (downHandler) {
    document.removeEventListener('mousedown', downHandler, true);
    document.removeEventListener('touchstart', downHandler, true);
    downHandler = null;
  }
  (btn as HTMLElement | null)?.focus();
}

function toggleModal(): void {
  if (modalOpen) closeModal();
  else openModal();
}

// ── モーダル DOM 生成 ─────────────────────────────────────────────────────────

function ensureModal(): void {
  if (document.getElementById('mobile-connect-modal')) return;
  const modal = el('div', { attrs: { id: 'mobile-connect-modal' } });
  modal.hidden = true;

  const box = el('div', { class: 'mc-box', attrs: { role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'mobile-connect-title' } });

  const head = el('div', { class: 'mc-head' });
  head.appendChild(el('span', { class: 'mc-head-icon', text: '📱' }));
  head.appendChild(el('span', { class: 'mc-head-title', text: t('mobile_connect_title'), attrs: { id: 'mobile-connect-title' } }));
  const reShow = el('button', { class: 'mc-link mc-head-reshow', text: t('mobile_connect_show_risk'), attrs: { type: 'button' } });
  reShow.addEventListener('click', () => { state.consented = false; state.help = false; render(); });
  head.appendChild(reShow);
  const closeBtn = el('button', { class: 'mc-modal-close', text: '✕', attrs: { type: 'button', 'aria-label': t('settings_close') } });
  closeBtn.addEventListener('click', () => closeModal());
  head.appendChild(closeBtn);
  box.appendChild(head);

  const warnbar = el('div', { class: 'mc-warnbar', html: t('mobile_connect_warnbar'), attrs: { id: 'mobile-connect-warnbar' } });
  warnbar.hidden = true;
  box.appendChild(warnbar);

  box.appendChild(el('div', { class: 'mc-body', attrs: { id: 'mobile-connect-body' } }));
  modal.appendChild(box);
  document.body.appendChild(modal);
}

// ── 初期化（app-entry から呼ぶ）────────────────────────────────────────────────

export function initMobileConnect(): void {
  const btn = document.getElementById('mobile-connect-btn');
  if (!btn) return;
  ensureModal();
  btn.addEventListener('click', (e) => { e.stopPropagation(); toggleModal(); });
}
