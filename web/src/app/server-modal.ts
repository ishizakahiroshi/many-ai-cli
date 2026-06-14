// server-modal.ts — ヘッダー 🖥 Server ボタンのモーダル制御 + /api/servers クライアント。
//
// 内蔵リモート接続（internal/hub/servers.go）の UI。プロファイル一覧の取得・
// 接続/切断・追加を行う。接続成功時は対象 Hub URL を **新規タブ**で開く
// （モーダル埋め込みはしない＝低リスク）。設計は settings-panel / workflow-modal に準拠。

import { t } from '../i18n.js';
import { token } from './util.js';

interface ServerProfile {
  name: string;
  type: string; // 'ssh' | 'wsl'
  mode?: string; // 'serve' | 'tunnel'
  host?: string;
  user?: string;
  ssh_port?: number;
  identity_file?: string;
  token_command?: string;
  distro?: string;
  binary?: string;
  cwd?: string;
  hub_port?: number;
}

interface ActiveConn {
  profile: string;
  pid: number;
  hub_url: string;
  owned: boolean;
}

interface ServersResponse {
  ok: boolean;
  profiles?: ServerProfile[];
  last_used?: string;
  active?: ActiveConn[];
}

// 接続中フェーズの最大待ち時間。サーバ側 serverConnectWaitTimeout(120s) に揃える。
const CONNECT_POLL_MS = 1000;
const CONNECT_POLL_MAX = 125;

let modalOpen = false;
let lastProfiles: ServerProfile[] = [];
let connectAbort = false;
let downHandler: ((e: MouseEvent | TouchEvent) => void) | null = null;
let keyHandler: ((e: KeyboardEvent) => void) | null = null;

// ── API ────────────────────────────────────────────────────────────────────

function tk(): string {
  return encodeURIComponent(token || '');
}

async function apiGet(path: string): Promise<any> {
  const res = await fetch(`${path}?token=${tk()}`);
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data?.detail || data?.error || res.statusText);
  return data;
}

async function apiPost(path: string, body?: unknown): Promise<any> {
  const res = await fetch(`${path}?token=${tk()}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: body == null ? undefined : JSON.stringify(body),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data?.detail || data?.error || res.statusText);
  return data;
}

// ── ステータス表示 ────────────────────────────────────────────────────────────

function setStatus(msg: string, isError = false): void {
  const el = document.getElementById('server-modal-status');
  if (!el) return;
  if (!msg) {
    el.hidden = true;
    el.textContent = '';
    return;
  }
  el.hidden = false;
  el.textContent = msg;
  el.classList.toggle('server-modal-status-error', isError);
}

// ── 一覧描画 ──────────────────────────────────────────────────────────────────

async function loadServers(): Promise<void> {
  const listEl = document.getElementById('server-profile-list');
  const emptyEl = document.getElementById('server-profile-empty');
  if (!listEl) return;
  try {
    const data: ServersResponse = await apiGet('/api/servers');
    lastProfiles = Array.isArray(data.profiles) ? data.profiles : [];
    const active = Array.isArray(data.active) ? data.active : [];
    renderProfiles(lastProfiles, active);
    if (emptyEl) emptyEl.hidden = lastProfiles.length > 0;
  } catch (err) {
    setStatus(t('server_load_failed') + ': ' + String((err as Error)?.message || err), true);
  }
}

function activeFor(name: string, active: ActiveConn[]): ActiveConn | null {
  for (const c of active) if (c.profile === name) return c;
  return null;
}

function renderProfiles(profiles: ServerProfile[], active: ActiveConn[]): void {
  const listEl = document.getElementById('server-profile-list');
  if (!listEl) return;
  listEl.innerHTML = '';
  for (const p of profiles) {
    const conn = activeFor(p.name, active);
    listEl.appendChild(buildRow(p, conn));
  }
}

function buildRow(p: ServerProfile, conn: ActiveConn | null): HTMLLIElement {
  const li = document.createElement('li');
  li.className = 'server-profile-row';

  const info = document.createElement('div');
  info.className = 'server-profile-info';

  const top = document.createElement('div');
  top.className = 'server-profile-top';

  const badge = document.createElement('span');
  badge.className = 'server-type-badge server-type-' + (p.type === 'wsl' ? 'wsl' : 'ssh');
  badge.textContent = p.type === 'wsl' ? 'WSL' : 'SSH';
  top.appendChild(badge);

  const nameEl = document.createElement('span');
  nameEl.className = 'server-profile-name';
  nameEl.textContent = p.name;
  top.appendChild(nameEl);

  const state = document.createElement('span');
  state.className = 'server-profile-state ' + (conn ? 'is-connected' : 'is-idle');
  state.textContent = conn ? t('server_state_connected') : t('server_state_idle');
  top.appendChild(state);

  info.appendChild(top);

  const detail = document.createElement('div');
  detail.className = 'server-profile-detail';
  const detailText = p.type === 'wsl'
    ? (p.distro ? p.distro : t('server_distro_default'))
    : [sshTarget(p), p.mode === 'tunnel' ? 'tunnel' : 'serve'].filter(Boolean).join(' · ');
  detail.textContent = detailText;
  info.appendChild(detail);

  li.appendChild(info);

  const actions = document.createElement('div');
  actions.className = 'server-profile-actions';

  if (conn) {
    const openBtn = document.createElement('button');
    openBtn.type = 'button';
    openBtn.className = 'server-btn-primary server-row-btn';
    openBtn.textContent = t('server_open');
    openBtn.addEventListener('click', () => { window.open(conn.hub_url, '_blank', 'noopener'); });
    actions.appendChild(openBtn);

    if (conn.owned) {
      const discBtn = document.createElement('button');
      discBtn.type = 'button';
      discBtn.className = 'server-btn-secondary server-row-btn';
      discBtn.textContent = t('server_disconnect');
      discBtn.addEventListener('click', () => disconnectProfile(p.name, discBtn));
      actions.appendChild(discBtn);
    }
  } else {
    const connBtn = document.createElement('button');
    connBtn.type = 'button';
    connBtn.className = 'server-btn-primary server-row-btn';
    connBtn.textContent = t('server_connect');
    connBtn.addEventListener('click', () => connectProfile(p.name, connBtn));
    actions.appendChild(connBtn);
  }

  li.appendChild(actions);
  return li;
}

function sshTarget(p: ServerProfile): string {
  if (p.user && p.host) return p.user + '@' + p.host;
  return p.host || '';
}

// ── 接続 / 切断 ────────────────────────────────────────────────────────────────

async function connectProfile(name: string, btn: HTMLButtonElement): Promise<void> {
  // ポップアップブロック回避: クリック（ユーザー操作）の同期内で空タブを開いておき、
  // 接続成功後にその URL へ遷移させる。失敗時は閉じる。
  const pending = window.open('about:blank', '_blank');
  connectAbort = false;
  btn.disabled = true;
  setStatus(t('server_connecting').replace('{name}', name));
  try {
    await apiPost('/api/servers/connect', { name });
    const hubURL = await pollConnectStatus();
    if (hubURL) {
      if (pending && !pending.closed) pending.location.href = hubURL;
      else window.open(hubURL, '_blank', 'noopener');
      setStatus(t('server_connected').replace('{name}', name));
    } else {
      // hubURL が空 = status が idle に戻った（接続要求が破棄された／上書きされた）。
      // connectAbort（モーダルを閉じた）時は何も出さない。それ以外は「接続中…」の
      // 残留を避けるため失敗表示にする。
      if (pending && !pending.closed) pending.close();
      if (!connectAbort) setStatus(t('server_connect_lost').replace('{name}', name), true);
    }
  } catch (err) {
    if (pending && !pending.closed) pending.close();
    setStatus(t('server_connect_failed') + ': ' + String((err as Error)?.message || err), true);
  } finally {
    btn.disabled = false;
    await loadServers();
  }
}

// pollConnectStatus は接続完了まで /api/servers/connect/status をポーリングし、
// 成功で hub_url を返す。失敗は例外、未完了タイムアウトは空文字を返す。
async function pollConnectStatus(): Promise<string> {
  for (let i = 0; i < CONNECT_POLL_MAX; i++) {
    if (connectAbort) return '';
    const data = await apiGet('/api/servers/connect/status');
    if (data.status === 'connected') return String(data.hub_url || '');
    if (data.status === 'error') throw new Error(data.error || 'connection error');
    if (data.status === 'idle') return '';
    await sleep(CONNECT_POLL_MS);
  }
  throw new Error(t('server_connect_timeout'));
}

async function disconnectProfile(name: string, btn: HTMLButtonElement): Promise<void> {
  btn.disabled = true;
  setStatus(t('server_disconnecting').replace('{name}', name));
  try {
    await apiPost('/api/servers/disconnect', { name, mode: 'disconnect' });
    setStatus(t('server_disconnected').replace('{name}', name));
  } catch (err) {
    setStatus(t('server_disconnect_failed') + ': ' + String((err as Error)?.message || err), true);
  } finally {
    btn.disabled = false;
    await loadServers();
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

// ── 追加フォーム ──────────────────────────────────────────────────────────────

function num(id: string): number {
  const el = document.getElementById(id) as HTMLInputElement | null;
  const v = parseInt((el?.value || '').trim(), 10);
  return Number.isFinite(v) ? v : 0;
}

function str(id: string): string {
  const el = document.getElementById(id) as HTMLInputElement | null;
  return (el?.value || '').trim();
}

function applyAddFormVisibility(): void {
  const typeSel = document.getElementById('server-add-type') as HTMLSelectElement | null;
  const modeSel = document.getElementById('server-add-mode') as HTMLSelectElement | null;
  const type = typeSel?.value || 'ssh';
  const mode = modeSel?.value || 'serve';
  const showSSH = type === 'ssh';
  document.querySelectorAll('.server-field-ssh').forEach((el) => {
    (el as HTMLElement).hidden = !showSSH;
  });
  document.querySelectorAll('.server-field-wsl').forEach((el) => {
    (el as HTMLElement).hidden = showSSH;
  });
  document.querySelectorAll('.server-field-tunnel').forEach((el) => {
    (el as HTMLElement).hidden = !(showSSH && mode === 'tunnel');
  });
}

function buildProfileFromForm(): ServerProfile {
  const type = (document.getElementById('server-add-type') as HTMLSelectElement | null)?.value || 'ssh';
  const p: ServerProfile = { name: str('server-add-name'), type };
  const binary = str('server-add-binary');
  const cwd = str('server-add-cwd');
  const hubPort = num('server-add-hub-port');
  if (binary) p.binary = binary;
  if (cwd) p.cwd = cwd;
  if (hubPort) p.hub_port = hubPort;
  if (type === 'wsl') {
    const distro = str('server-add-distro');
    if (distro) p.distro = distro;
  } else {
    const mode = (document.getElementById('server-add-mode') as HTMLSelectElement | null)?.value || 'serve';
    p.mode = mode;
    const host = str('server-add-host');
    const user = str('server-add-user');
    const sshPort = num('server-add-ssh-port');
    const identity = str('server-add-identity');
    if (host) p.host = host;
    if (user) p.user = user;
    if (sshPort) p.ssh_port = sshPort;
    if (identity) p.identity_file = identity;
    if (mode === 'tunnel') {
      const tokenCommand = str('server-add-token-command');
      if (tokenCommand) p.token_command = tokenCommand;
    }
  }
  return p;
}

function setAddError(msg: string): void {
  const el = document.getElementById('server-add-error');
  if (!el) return;
  if (!msg) { el.hidden = true; el.textContent = ''; return; }
  el.hidden = false;
  el.textContent = msg;
}

async function saveNewProfile(btn: HTMLButtonElement): Promise<void> {
  const profile = buildProfileFromForm();
  if (!profile.name) {
    setAddError(t('server_add_name_required'));
    return;
  }
  if (lastProfiles.some((p) => p.name === profile.name)) {
    setAddError(t('server_add_name_duplicate'));
    return;
  }
  setAddError('');
  btn.disabled = true;
  try {
    const next = lastProfiles.concat([profile]);
    await apiPost('/api/servers', { profiles: next });
    resetAddForm();
    const section = document.getElementById('server-add-section') as HTMLDetailsElement | null;
    if (section) section.open = false;
    await loadServers();
  } catch (err) {
    setAddError(String((err as Error)?.message || err));
  } finally {
    btn.disabled = false;
  }
}

function resetAddForm(): void {
  ['server-add-name', 'server-add-host', 'server-add-user', 'server-add-ssh-port',
    'server-add-identity', 'server-add-token-command', 'server-add-distro',
    'server-add-binary', 'server-add-cwd', 'server-add-hub-port'].forEach((id) => {
    const el = document.getElementById(id) as HTMLInputElement | null;
    if (el) el.value = '';
  });
  setAddError('');
}

// ── モーダル開閉 ──────────────────────────────────────────────────────────────

function openServerModal(): void {
  const modal = document.getElementById('server-modal');
  const btn = document.getElementById('server-btn');
  if (!modal) return;
  modal.hidden = false;
  modalOpen = true;
  btn?.setAttribute('aria-expanded', 'true');
  setStatus('');
  applyAddFormVisibility();
  void loadServers();

  downHandler = (e: MouseEvent | TouchEvent) => {
    const target = e.target as Node;
    const box = modal.querySelector('.server-modal-box');
    if (box && box.contains(target)) return;
    if ((target as HTMLElement).closest?.('#server-btn')) return; // 再クリックは toggle に委ねる
    closeServerModal();
  };
  keyHandler = (e: KeyboardEvent) => {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    e.stopPropagation();
    closeServerModal();
  };
  setTimeout(() => {
    if (downHandler) {
      document.addEventListener('mousedown', downHandler, true);
      document.addEventListener('touchstart', downHandler, true);
    }
  }, 0);
  document.addEventListener('keydown', keyHandler, true);
}

function closeServerModal(): void {
  const modal = document.getElementById('server-modal');
  const btn = document.getElementById('server-btn');
  modalOpen = false;
  connectAbort = true;
  if (modal) modal.hidden = true;
  btn?.setAttribute('aria-expanded', 'false');
  if (keyHandler) { document.removeEventListener('keydown', keyHandler, true); keyHandler = null; }
  if (downHandler) {
    document.removeEventListener('mousedown', downHandler, true);
    document.removeEventListener('touchstart', downHandler, true);
    downHandler = null;
  }
}

function toggleServerModal(): void {
  if (modalOpen) closeServerModal();
  else openServerModal();
}

// ── 初期化（app-entry から呼ぶ） ──────────────────────────────────────────────

export function initServerModal(): void {
  const btn = document.getElementById('server-btn');
  if (!btn) return;
  btn.addEventListener('click', (e) => { e.stopPropagation(); toggleServerModal(); });

  document.getElementById('server-modal-close')?.addEventListener('click', closeServerModal);
  document.getElementById('server-modal-refresh')?.addEventListener('click', () => { void loadServers(); });
  document.getElementById('server-add-type')?.addEventListener('change', applyAddFormVisibility);
  document.getElementById('server-add-mode')?.addEventListener('change', applyAddFormVisibility);
  document.getElementById('server-add-save')?.addEventListener('click', (e) => {
    saveNewProfile(e.currentTarget as HTMLButtonElement);
  });
  document.getElementById('server-add-cancel')?.addEventListener('click', () => {
    resetAddForm();
    const section = document.getElementById('server-add-section') as HTMLDetailsElement | null;
    if (section) section.open = false;
  });
}
