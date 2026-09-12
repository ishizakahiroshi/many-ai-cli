// session-strip.ts — 端末の上に 1 段だけ出る「セッション帯」。
//
// いま開いている箱（サイドバーのプロジェクトグループ）のセッションを横に並べ、
// 主画面の中だけで同じリポジトリのセッションを行き来できるようにする。
//
// 由来: docs/local/plan_project-box-open-and-session-strip_c2_session-strip.md
//
// ここが持っている前提を 3 つだけ書いておく。壊すと静かに効き目が消える。
//
//  1. 並び順は orderSessions() の結果をその箱で絞ったもの。独自に並べ替えない。
//     サイドバーと順序が食い違うと「同じ一覧の別の写し」が 2 つある状態になる。
//  2. 帯は #display-stack の外（index.html）に置く。中に入れると #action-bar を
//     absolute で重ねるための位置基準が変わる。
//  3. 高さのドラッグ中は PTY リサイズを抑える。抑えないと SIGWINCH が連発して
//     TUI が再描画され、同じ本文が scrollback へ積み上がる
//     （index.html の #display-stack 前のコメントと terminal.ts:2223-2232 が記録している）。
//     抑制は短い時間で張り直し、pointerup で 1 回だけ確定させる。長く張りっぱなしに
//     すると今度は Codex が古い行数のまま描き続けて空行が化石化する。

import { activeSessionId, openProjectKey, orderSessions, sessions } from './state.js';
import { NO_PROJECT_KEY, deriveProjectKeyFromCwd, projectKeyForSession } from './sidebar-tree.js';
// ブランチ名の省略規則は DOM に触らない純関数として切り出してある
// （node:test から検証するため。テストは session-strip-fixtures.ts）。
import { abbreviateBranchName } from './session-strip-branch.js';
import {
  activateSession,
  providerDisplayName,
  providerIconHtml,
  stateActivityDecoration,
  stateIconSvgHtml,
} from './session-list.js';
import { saveProjectView } from './project-view-store.js';
import { escapeHtml, ti18n } from './util.js';
import { t } from '../i18n.js';
import { FONTSIZE_MAP, STORAGE_FONTSIZE_KEY } from './user-prefs.js';
import {
  TERMINAL_LINE_HEIGHT,
  followSessionStripResize,
  suppressPtyResizeForInputLayout,
  syncPtySizeToViewportAfterLayout,
} from './terminal.js';
import type { SessionSnapshot } from '../types/proto.js';

// 帯の高さ（利用者がドラッグで決めた値）。端末ごと＝localStorage で、サーバーへは
// 持たせない（多ペインの行列数やタブ順と同じ扱い）。未設定なら文字サイズ比例の既定。
const STORAGE_STRIP_HEIGHT_KEY = 'ai_cli_hub_session_strip_height';

// 掴み帯の幅。action-bar-resize.ts:19 の HANDLE_ZONE_PX と同じ 8px。
const GRIP_ZONE_PX = 8;

// 既定の高さ＝端末 1 行の高さの何倍か。中（13px）で約 30px になる。
const DEFAULT_HEIGHT_IN_LINES = 1.7;
// 下限＝中身が読める高さ。上限＝端末が極端に潰れない高さ（実機で見て詰める）。
const MIN_HEIGHT_IN_LINES = 1.4;
const MAX_HEIGHT_IN_LINES = 12;
const MAX_HEIGHT_VIEWPORT_RATIO = 0.35;

// ドラッグ 1 回ぶんの抑制時間。pointerup の確定（既定 400ms 後）より短くしておき、
// 手を離したあとに抑制が残らないようにする。
const DRAG_SUPPRESS_MS = 250;
// pointerup 後に実寸を PTY へ送るまでの猶予。レイアウトが落ち着いてから 1 回だけ。
const SETTLE_DELAY_MS = 400;

// セッション一覧を出すタブ。multi は同じ段へ範囲トグル（C3）を出すので、この集合には
// 入れない（段そのものは出す）。approval / orchestration はセッション非依存の集約ビュー
// なので段ごと隠す。
const STRIP_VISIBLE_TABS = new Set(['terminal', 'chat', 'split', 'files', 'git', 'history']);

// 範囲トグルを出すタブ。段は STRIP_VISIBLE_TABS と共有する（高さを揃えるため）。
const SCOPE_TAB = 'multi';

// ---- 高さ -------------------------------------------------------------------

function terminalFontPx(): number {
  try {
    const stored = localStorage.getItem(STORAGE_FONTSIZE_KEY);
    return FONTSIZE_MAP[stored as keyof typeof FONTSIZE_MAP] || FONTSIZE_MAP.medium;
  } catch (_) {
    return FONTSIZE_MAP.medium;
  }
}

/** 端末 1 行の高さ（px）。帯の寸法はすべてこれの倍数で決める＝固定 px を焼かない。 */
function terminalLinePx(): number {
  return terminalFontPx() * TERMINAL_LINE_HEIGHT;
}

function defaultStripHeightPx(): number {
  return Math.round(terminalLinePx() * DEFAULT_HEIGHT_IN_LINES);
}

function stripHeightBounds(): { min: number; max: number } {
  const line = terminalLinePx();
  const min = Math.round(line * MIN_HEIGHT_IN_LINES);
  const viewport = (typeof window !== 'undefined' ? window.innerHeight : 0) || 0;
  const max = Math.max(min, Math.round(Math.min(
    viewport > 0 ? viewport * MAX_HEIGHT_VIEWPORT_RATIO : Number.POSITIVE_INFINITY,
    line * MAX_HEIGHT_IN_LINES,
  )));
  return { min, max };
}

/** 利用者が手で決めた高さ。null＝未指定（＝文字サイズ比例の既定に従う）。 */
function storedStripHeight(): number | null {
  try {
    const raw = localStorage.getItem(STORAGE_STRIP_HEIGHT_KEY);
    if (!raw) return null;
    const value = Number(raw);
    return Number.isFinite(value) && value > 0 ? value : null;
  } catch (_) {
    return null;
  }
}

function setStripHeightVar(px: number): void {
  document.documentElement.style.setProperty('--session-strip-h', Math.round(px) + 'px');
}

/**
 * 帯の高さと文字サイズを、いまの端末の文字サイズ設定へ合わせ直す。
 *
 * 利用者が手で高さを決めていればその値を（上下限でクランプして）使い、決めていなければ
 * 文字サイズ比例の既定を使う。文字サイズ設定を変えたときに settings.ts の applyFontSize
 * から呼ばれるので、既定のままなら新しい比例値へ追従する。
 */
export function applySessionStripMetrics(): void {
  const root = document.documentElement;
  if (!root) return;
  const { min, max } = stripHeightBounds();
  const stored = storedStripHeight();
  const height = stored === null ? defaultStripHeightPx() : Math.max(min, Math.min(max, stored));
  setStripHeightVar(height);
  root.style.setProperty('--session-strip-font-size', terminalFontPx() + 'px');
  // 掴み帯の幅は CSS 側にも要る。定義を 2 か所へ散らさないため変数で渡す。
  root.style.setProperty('--session-strip-grip-h', GRIP_ZONE_PX + 'px');
}

/** いま実際に描かれている帯の高さ。ドラッグの基準にする。 */
function currentStripHeightPx(): number {
  const el = document.getElementById('session-strip');
  const measured = el ? el.getBoundingClientRect().height : 0;
  if (measured > 0) return measured;
  const stored = storedStripHeight();
  return stored === null ? defaultStripHeightPx() : stored;
}

// ---- 描画 -------------------------------------------------------------------

// いま表示しているタブ。index.html の初期状態は terminal タブ。
let currentTabName = 'terminal';
// 直前に描いた内容のシグネチャ。同じなら DOM を作り直さない（1Hz の再描画で
// 帯がチカチカしないように。lastActionBarRender と同じ考え方）。
let lastStripSig = '';

/**
 * 帯が映す箱のキー。
 *
 * 正は C1 が作った openProjectKey。まだどの箱も開かれていない起動直後は、いま主画面に
 * 出ているセッションの箱を映す（openProjectKey は書き換えない。あの状態の持ち主は C1 で、
 * 復元は親 plan の C4 が担当する）。
 */
function resolveStripProjectKey(): string | null {
  if (openProjectKey) return openProjectKey;
  if (activeSessionId === null) return null;
  const current = sessions.get(activeSessionId);
  if (!current) return null;
  return projectKeyForSession(current, sessions.values() as any);
}

function stripSessionsFor(key: string): SessionSnapshot[] {
  const all = Array.from(sessions.values());
  // 並び順の正本は orderSessions()。ここでは絞るだけで並べ替えない。
  return orderSessions().filter(s => projectKeyForSession(s, all) === key);
}

function projectDisplayNameFor(key: string): string {
  // サイドバーの見出しと同じ名前を出す（key の末尾セグメント。git 管理外は「プロジェクト未設定」）。
  if (!key || key === NO_PROJECT_KEY) return t('no_project');
  return deriveProjectKeyFromCwd(key) || key;
}

function sessionItemHtml(s: any): string {
  const activity = stateActivityDecoration(s);
  const providerKey = String(s?.provider || '').trim();
  const providerLabel = providerKey ? providerDisplayName(providerKey) : '';
  const providerIcon = providerKey ? providerIconHtml(providerKey, 12) : '';
  const branch = String(s?.branch || '');
  // worktree_branch は帯に出さない（隔離 worktree のメタデータであって cwd の実ブランチではない）。
  // git 管理外でブランチが無いときは、ブランチの部分ごと出さない（"(no git)" も出さない）。
  const branchHtml = branch
    ? `<span class="session-strip-branch" title="${escapeHtml(branch)}" data-tooltip="${escapeHtml(branch)}">${escapeHtml(abbreviateBranchName(branch))}</span>`
    : '';
  const tooltip = [`#${s.id}`, providerLabel, branch, activity.label].filter(Boolean).join(' · ');
  const classes = ['session-strip-item'];
  if (s.state === 'running') classes.push('running');
  if (activity.className) classes.push(activity.className);
  if (s.id === activeSessionId) classes.push('active');
  return `<button type="button" class="${classes.join(' ')}" data-sid="${escapeHtml(String(s.id))}"`
    + ` title="${escapeHtml(tooltip)}" data-tooltip="${escapeHtml(tooltip)}" aria-label="${escapeHtml(tooltip)}"`
    + `${s.id === activeSessionId ? ' aria-current="true"' : ''}>`
    + `<span class="session-strip-state" aria-hidden="true">${stateIconSvgHtml(activity.iconKind)}</span>`
    + `<span class="session-strip-id">#${escapeHtml(String(s.id))}</span>`
    + (providerIcon ? `<span class="session-strip-provider" aria-hidden="true">${providerIcon}</span>` : '')
    + branchHtml
    + `</button>`;
}

/**
 * 帯を描き直す。
 *
 * renderSessionList() を流用しない（あちらは root.innerHTML = '' の全再構築で、帯まで
 * 巻き込むと無駄が大きい）。逆にここも必要なときだけ描く＝内容が変わっていなければ
 * DOM に触らない。
 */
export function renderSessionStrip(): void {
  const strip = document.getElementById('session-strip');
  if (!strip) return;
  const itemsEl = document.getElementById('session-strip-items');
  const projectEl = document.getElementById('session-strip-project') as HTMLButtonElement | null;
  const scopeEl = document.getElementById('session-strip-scope');
  if (!itemsEl || !projectEl) return;

  // multi タブは同じ段に範囲トグルを出す（セッション一覧は出さない）。
  if (currentTabName === SCOPE_TAB) {
    renderScopeToggle(strip, itemsEl, projectEl, scopeEl);
    return;
  }
  if (scopeEl) scopeEl.hidden = true;
  itemsEl.hidden = false;

  const key = resolveStripProjectKey();
  const list = key ? stripSessionsFor(key) : [];
  // 出すのは対象タブのときだけ。中身が無い帯は場所を取るだけなので出さない
  // （セッションが 1 個でも帯は出す＝0 個のときだけ隠す）。
  strip.hidden = !STRIP_VISIBLE_TABS.has(currentTabName) || !key || list.length === 0;
  if (strip.hidden) {
    lastStripSig = '';
    return;
  }

  // 描く内容そのものをシグネチャにする。中身が同じなら DOM に触らない
  // （1Hz の状態更新でも帯がチカチカしない・ホバー中のツールチップが消えない）。
  const name = projectDisplayNameFor(key as string);
  const itemsHtml = list.map(sessionItemHtml).join('');
  const sig = 'list\u0000' + name + '\u0000' + (key as string) + '\u0000' + itemsHtml;
  if (sig === lastStripSig) return;
  lastStripSig = sig;

  const projectTip = ti18n('session_strip_project_tooltip', 'Show this project in the sidebar');
  projectEl.hidden = false;
  projectEl.textContent = name;
  projectEl.title = `${name} · ${projectTip}`;
  projectEl.dataset.tooltip = projectTip;
  projectEl.dataset.project = key as string;

  itemsEl.innerHTML = itemsHtml;
}

// ---- 範囲トグル（multi タブ）------------------------------------------------
//
// multi タブは同じ段にセッション一覧ではなく「この箱だけ／全部」のトグルを出す。
// 段を 2 本に分けないのは、タブを往復するたびに端末の高さが動くと PTY リサイズが
// 飛んで TUI が再描画され、同じ本文が scrollback へ積み上がるため。
//
// 範囲そのものの正本は multi-pane.ts（MultiPaneManager）側にある。ここは読んで描き、
// 押されたら setScope を呼ぶだけ＝状態を 2 か所に持たない。

interface ScopeStatus {
  scope: string;
  effective: string;
  projectKey: string | null;
  visibleCount: number;
  capacity: number;
  overflow: number;
}

function readScopeStatus(): ScopeStatus {
  const mgr = (window as any).multiPaneManager;
  if (mgr && typeof mgr.getScopeStatus === 'function') {
    try {
      const s = mgr.getScopeStatus();
      return {
        scope: String(s?.scope || 'all'),
        effective: String(s?.effective || 'all'),
        projectKey: s?.projectKey ?? null,
        visibleCount: Number(s?.visibleCount) || 0,
        capacity: Number(s?.capacity) || 0,
        overflow: Number(s?.overflow) || 0,
      };
    } catch (_) { /* 下のフォールバックへ */ }
  }
  // MultiPaneManager がまだ居ない（起動直後など）。既定の「全部」として描く。
  return { scope: 'all', effective: 'all', projectKey: openProjectKey, visibleCount: 0, capacity: 0, overflow: 0 };
}

function scopeButtonHtml(
  scope: 'box' | 'all',
  label: string,
  pressed: boolean,
  tooltip: string,
  extraHtml: string,
  disabled: boolean,
): string {
  return `<button type="button" class="session-strip-scope-btn" data-scope="${scope}"`
    + ` aria-pressed="${pressed ? 'true' : 'false'}"${disabled ? ' disabled' : ''}`
    + ` title="${escapeHtml(tooltip)}" data-tooltip="${escapeHtml(tooltip)}">`
    + `<span class="session-strip-scope-label">${escapeHtml(label)}</span>`
    + extraHtml
    + `</button>`;
}

function scopeToggleHtml(status: ScopeStatus): string {
  const key = status.projectKey;
  const boxLabel = ti18n('multi_scope_box', 'This project only');
  const allLabel = ti18n('multi_scope_all', 'All');
  const boxOpen = !!key;
  // 押されている側は「実際に効いている範囲」で描く。保存値が 'box' でも箱が開いて
  // いなければ全部が並ぶので、そのときは「全部」を押された側にする（画面と中身を合わせる）。
  const boxPressed = status.effective === 'box';
  const boxTip = boxOpen
    ? `${boxLabel} · ${projectDisplayNameFor(key as string)}`
    : ti18n('multi_scope_box_disabled', 'Open a project in the sidebar to use this');
  const nameHtml = boxOpen
    ? `<span class="session-strip-scope-name">${escapeHtml(projectDisplayNameFor(key as string))}</span>`
    : '';
  const moreHtml = status.overflow > 0
    ? `<span class="session-strip-scope-more" title="${escapeHtml(ti18n('multi_scope_more_tooltip', '{count} sessions do not fit in the current grid', { count: status.overflow }))}"`
      + ` data-tooltip="${escapeHtml(ti18n('multi_scope_more_tooltip', '{count} sessions do not fit in the current grid', { count: status.overflow }))}">`
      + escapeHtml(ti18n('multi_scope_more', '{count} more', { count: status.overflow }))
      + `</span>`
    : '';
  const boxBtn = scopeButtonHtml('box', boxLabel, boxPressed, boxTip, nameHtml, !boxOpen);
  // 押せないボタンはマウスイベントを出さない（title も data-tooltip も拾われない）ので、
  // 押せないときだけ包みに理由を持たせる。そうしないと「なぜ押せないか」が読めない。
  const boxHtml = boxOpen
    ? boxBtn
    : `<span class="session-strip-scope-hint" title="${escapeHtml(boxTip)}" data-tooltip="${escapeHtml(boxTip)}">${boxBtn}</span>`;
  return boxHtml
    + scopeButtonHtml('all', allLabel, !boxPressed, ti18n('multi_scope_all_tooltip', 'Show every session in the grid'), '', false)
    + moreHtml;
}

function renderScopeToggle(
  strip: HTMLElement,
  itemsEl: HTMLElement,
  projectEl: HTMLButtonElement,
  scopeEl: HTMLElement | null,
): void {
  if (!scopeEl) { strip.hidden = true; lastStripSig = ''; return; }
  // 段は常に出す。箱を開いていなくてもトグル自体は要る（押せない理由を見せるため）。
  strip.hidden = false;
  projectEl.hidden = true;   // 箱の名前は「この箱だけ」の中に出すので二重に出さない
  itemsEl.hidden = true;
  scopeEl.hidden = false;
  scopeEl.setAttribute('role', 'group');
  scopeEl.setAttribute('aria-label', ti18n('multi_scope_label', 'Which sessions the grid shows'));

  const html = scopeToggleHtml(readScopeStatus());
  const sig = 'scope' + '\u0000' + html;
  if (sig === lastStripSig) return;
  lastStripSig = sig;
  scopeEl.innerHTML = html;
}

/** サイドバーの該当する箱の見出しまでスクロールする。折りたたみ状態は変えない。 */
function scrollSidebarToProject(key: string): void {
  const root = document.getElementById('sessions');
  if (!root || !key) return;
  let header: Element | null = null;
  try {
    header = root.querySelector(`.project-group-header[data-project="${CSS.escape(key)}"]`);
  } catch (_) {
    header = null;
  }
  if (header && typeof (header as HTMLElement).scrollIntoView === 'function') {
    (header as HTMLElement).scrollIntoView({ block: 'nearest' });
  }
}

// ---- タブ切替 ---------------------------------------------------------------

/**
 * いま画面に出ているタブの名前。
 *
 * setActiveTab の全分岐が setSessionStripTab を通るので、この値は画面と必ず一致する
 * （セッションごとの sessionViewMode は multi / approval / orchestration を持たない）。
 * 箱ごとの記憶（C4）が「今のタブ」を保存するときの出どころ。
 */
export function currentSessionStripTab(): string {
  return currentTabName;
}

/** setActiveTab の各分岐から呼ばれる。帯を出すタブかどうかだけを受け取る。 */
export function setSessionStripTab(name: unknown): void {
  const next = String(name || '');
  if (!next) return;
  currentTabName = next;
  renderSessionStrip();
}

// ---- 高さのドラッグ ---------------------------------------------------------

let followRafPending = false;
function followTerminal(): void {
  if (followRafPending) return;
  followRafPending = true;
  requestAnimationFrame(() => {
    followRafPending = false;
    followSessionStripResize();
  });
}

/** pointerup / dblclick の後で、実寸を PTY へ 1 回だけ送って確定させる。 */
function settlePtySize(reason: string): void {
  if (activeSessionId === null) return;
  syncPtySizeToViewportAfterLayout(activeSessionId, true, SETTLE_DELAY_MS, reason);
}

// 掴み帯のドラッグ。作法は multi-pane.ts の _wireSplitter に合わせる。
//   pointerdown で body へリサイズ中のクラスを付け、window に pointermove / pointerup を張る
//   pointermove ではスタイルを直接書くだけ（永続化も DOM 再構築もしない）
//   pointerup でリスナーを外し、クラスを外し、そこで初めて保存する
function wireGrip(grip: HTMLElement): void {
  grip.addEventListener('pointerdown', (e: PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const startY = e.clientY;
    const startHeight = currentStripHeightPx();
    const { min, max } = stripHeightBounds();
    let nextHeight = startHeight;

    const onMove = (ev: PointerEvent) => {
      nextHeight = Math.max(min, Math.min(max, startHeight + (ev.clientY - startY)));
      setStripHeightVar(nextHeight);
      // ドラッグ 1 回ごとに短く張り直す。長く張りっぱなしにしない。
      suppressPtyResizeForInputLayout(DRAG_SUPPRESS_MS);
      followTerminal();
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
      document.body.classList.remove('session-strip-resizing');
      try { localStorage.setItem(STORAGE_STRIP_HEIGHT_KEY, String(Math.round(nextHeight))); } catch (_) {}
      settlePtySize('session-strip-resize');
    };

    document.body.classList.add('session-strip-resizing');
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onUp);
  });

  // ダブルクリックで利用者指定の高さを解除し、文字サイズ比例の既定へ戻す
  // （action-bar-resize.ts の掴み帯と同じ作法）。
  grip.addEventListener('dblclick', (e: MouseEvent) => {
    e.preventDefault();
    try { localStorage.removeItem(STORAGE_STRIP_HEIGHT_KEY); } catch (_) {}
    applySessionStripMetrics();
    followTerminal();
    settlePtySize('session-strip-reset');
  });
}

// ---- 初期化 -----------------------------------------------------------------

export function initSessionStrip(): void {
  const strip = document.getElementById('session-strip');
  if (!strip) return;
  applySessionStripMetrics();

  const itemsEl = document.getElementById('session-strip-items');
  if (itemsEl) {
    itemsEl.addEventListener('click', (e: Event) => {
      const target = (e.target as HTMLElement | null)?.closest('.session-strip-item') as HTMLElement | null;
      if (!target) return;
      const id = parseInt(target.dataset.sid || '', 10);
      if (Number.isNaN(id)) return;
      // 新しいセッション切替経路を作らない。既存の activateSession だけを通す。
      activateSession(id);
      // 箱ごとの記憶（C4）。**保存は利用者の操作からだけ**呼ぶ。一覧の変化からは
      // 呼ばない（切断すると ws-client.ts が sessions.clear() するので、そこを
      // 「箱が消えた」と読むと切断のたびに記憶が消える）。
      // 箱が開いていないときは何もしない＝帯が映しているだけの箱を勝手に覚えない。
      if (openProjectKey) saveProjectView(openProjectKey, id, currentTabName);
    });
  }

  const projectEl = document.getElementById('session-strip-project');
  if (projectEl) {
    projectEl.addEventListener('click', () => {
      const key = (projectEl as HTMLElement).dataset.project || '';
      scrollSidebarToProject(key);
    });
  }

  const scopeEl = document.getElementById('session-strip-scope');
  if (scopeEl) {
    scopeEl.addEventListener('click', (e: Event) => {
      const btn = (e.target as HTMLElement | null)?.closest('.session-strip-scope-btn') as HTMLButtonElement | null;
      if (!btn || btn.disabled) return;
      const mgr = (window as any).multiPaneManager;
      if (!mgr || typeof mgr.setScope !== 'function') return;
      // 範囲の正本は MultiPaneManager。ここは押されたことを渡すだけで、
      // 行列数・比率・並び順のいずれも保存し直さない（setScope が render するだけ）。
      mgr.setScope(btn.dataset.scope || '');
      renderSessionStrip();
    });
  }

  const grip = document.getElementById('session-strip-grip');
  if (grip) wireGrip(grip as HTMLElement);

  renderSessionStrip();
}
