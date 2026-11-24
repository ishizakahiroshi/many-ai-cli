import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { safeClassToken } from '../src/app/provider-icon.js';

// ペイン見出しの状態記号が、サイドバーのカードと同じ記号・同じクラス・同じ更新規則で描かれることを固定する。
// 実画面（ブラウザでの見た目・回転・点滅）の確認ではなく、実際のソースを合成の環境で動かした呼び出し側の証拠。
const source = (name: string) => readFileSync(new URL('../src/app/' + name, import.meta.url), 'utf8');
const compile = (text: string) => new Bun.Transpiler({ loader: 'ts' }).transformSync(text.replace(/^import[\s\S]*?;\r?$/gm, '').replace(/^export /gm, ''));

class FakeBadge {
  classNameWrites = 0; htmlWrites = 0;
  private _className = ''; private _html = '';
  dataset: Record<string, string> = {}; attrs = new Map<string, string>();
  get className() { return this._className; }
  set className(value: string) { this._className = value; this.classNameWrites++; }
  get innerHTML() { return this._html; }
  set innerHTML(value: string) { this._html = value; this.htmlWrites++; }
  getAttribute(key: string) { return this.attrs.get(key) ?? null; }
  setAttribute(key: string, value: string) { this.attrs.set(key, String(value)); }
}

// カードが使う本物の stateActivityDecoration / stateIconSvgHtml（session-list.ts から切り出す）と、
// 見出しの本物の applyPaneStateBadge（pane-state-badge.ts）を同じ環境で動かす。
function paneFixture(opts: { starting?: boolean } = {}) {
  const list = source('session-list.ts');
  const start = list.indexOf('const STATE_ICON_SVG = {'), end = list.indexOf('\n// provider アイコンの定義', start);
  assert.ok(start >= 0 && end > start, 'the card state decoration block was not found in session-list.ts');
  const labels: Record<string, string> = { running: '実行中', standby: 'スタンバイ', waiting: '保留中', error: 'エラー' };
  const context: any = {
    safeClassToken, stateLabel: (state: string) => labels[state] || state, ti18n: (_key: string, fallback: string) => fallback,
    sessionStartups: { forSession: () => (opts.starting ? { phase: 'starting' } : undefined) },
    pendingSpawnConfirmationCount: () => 0, isSessionListStopped: () => false,
  };
  runInNewContext(compile(list.slice(start, end) + '\n' + source('pane-state-badge.ts') + '\nglobalThis.api={stateActivityDecoration,stateIconSvgHtml,applyPaneStateBadge};'), context);
  const api = context.api;
  return {
    decorate: (session: any) => JSON.parse(JSON.stringify(api.stateActivityDecoration(session))) as { className: string; iconKind: string; label: string },
    iconHtml: (kind: string) => api.stateIconSvgHtml(kind) as string,
    apply: (badge: FakeBadge, session: any) => api.applyPaneStateBadge(badge, session) as boolean,
  };
}

const SESSIONS: Array<[string, any]> = [
  ['standby', { id: 1, state: 'standby' }],
  ['running', { id: 2, state: 'running', workflow_active: true }],
  ['approval pending', { id: 3, state: 'waiting', awaiting_approval: true, awaiting_user: true }],
  ['awaiting input only', { id: 4, state: 'standby', awaiting_user: true }],
  ['processing', { id: 5, state: 'standby', workflow_active: true }],
  ['error', { id: 6, state: 'error' }],
];

test('the pane header draws the same icon, classes and tooltip as the session card for every state', () => {
  const { decorate, iconHtml, apply } = paneFixture();
  for (const [name, session] of SESSIONS) {
    const card = decorate(session); const badge = new FakeBadge(); apply(badge, session);
    assert.equal(badge.dataset.iconKind, card.iconKind, `${name}: icon kind`);
    assert.equal(badge.innerHTML, `<span class="card-state-icon" aria-hidden="true">${iconHtml(card.iconKind)}</span>`, `${name}: icon markup`);
    assert.equal(badge.className, `ph-badge card-state-pill ${safeClassToken(session.state)} ${card.className}`.trim(), `${name}: classes`);
    assert.equal(badge.getAttribute('title'), card.label, `${name}: tooltip`);
    assert.equal(badge.getAttribute('aria-label'), card.label, `${name}: accessible name`);
  }
});

test('an approval-pending session shows the card flag instead of the old warning glyph and asks for the slot highlight', () => {
  const { apply, iconHtml } = paneFixture(); const badge = new FakeBadge();
  assert.equal(apply(badge, SESSIONS[2][1]), true);
  assert.ok(badge.innerHTML.includes(iconHtml('flag')));
  assert.ok(badge.className.split(' ').includes('awaiting-approval'));
  for (const old of ['⚠', '●', '—']) assert.equal(badge.innerHTML.includes(old), false, `${old} is no longer drawn as text`);
  for (const [name, session] of SESSIONS.filter(([, s]) => s.id !== 3)) assert.equal(apply(new FakeBadge(), session), false, `${name}: no highlight`);
});

test('a launch whose startup screen is up draws the same spinning ring as the card even while the Hub says standby', () => {
  const { decorate, apply } = paneFixture({ starting: true }); const badge = new FakeBadge();
  const session = { id: 7, started_at: 'synthetic-start', state: 'standby' };
  apply(badge, session);
  assert.equal(decorate(session).iconKind, 'ring');
  assert.equal(badge.dataset.iconKind, 'ring');
  assert.ok(badge.className.split(' ').includes('running'));
});

test('an unchanged state touches nothing, so the running spinner is not restarted; a changed state replaces the icon', () => {
  const { apply } = paneFixture(); const badge = new FakeBadge();
  apply(badge, SESSIONS[1][1]); assert.deepEqual([badge.classNameWrites, badge.htmlWrites], [1, 1]);
  apply(badge, { ...SESSIONS[1][1], last_output_at: 'synthetic-later' }); apply(badge, SESSIONS[1][1]);
  assert.deepEqual([badge.classNameWrites, badge.htmlWrites], [1, 1], 'same state: no DOM writes');
  apply(badge, SESSIONS[0][1]); assert.equal(badge.htmlWrites, 2); assert.equal(badge.dataset.iconKind, 'dot');
  assert.equal(apply(badge, undefined), false, 'a missing session leaves the badge as it was');
});

test('the class contract is spelled the same way in the card and in the pane header', () => {
  const contract = 'card-state-pill ${safeClassToken(state)} ${activity.className}';
  assert.ok(source('session-list.ts').includes(contract), 'the card pill no longer builds its classes this way: update pane-state-badge.ts together');
  assert.ok(source('pane-state-badge.ts').includes(contract));
});

test('the multi view and the detached grid keep no icon rule of their own and are refreshed from the session', () => {
  for (const name of ['multi-pane.ts', 'detached-grid.ts']) {
    const text = source(name);
    assert.equal(text.includes('⚠'), false, `${name}: no text glyph for state`);
    assert.ok(text.includes('applyPaneStateBadge('), `${name}: draws through the shared function`);
    assert.ok(text.includes('refreshSlotBadge('), `${name}: refreshes from the session`);
  }
  for (const name of ['multi-pane.ts', 'detached-grid.ts', 'ws-client.ts', 'state.ts']) {
    assert.equal(source(name).includes('updateSlotBadge'), false, `${name}: the status-argument API must not come back`);
  }
  const list = source('session-list.ts'); const fn = list.indexOf('export function updateSessionCardStateInPlace(');
  assert.ok(fn >= 0);
  const refresh = list.indexOf('refreshSlotBadge', fn), earlyReturn = list.indexOf('if (!root || !s) return false;', fn);
  assert.ok(refresh > fn && refresh < earlyReturn, 'the pane badge follows the card before the sidebar-less early return');
});

// ---------------------------------------------------------------------------------------------
// カード本体の状態枠（承認待ち＝橙・光る／未確認で停止＝黄・静か。Issue #29）。
// 呼び出し側と同じ順（stateActivityDecoration → sessionCardStateClasses → その場の付け外し → CSS が後ろで上書き）で確かめる。
// 実画面（色・光り方・選択中の外側の輪がサイドバー端で切れないか）の確認ではない。
const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const readWeb = (rel: string) => readFileSync(new URL('../src/' + rel, import.meta.url), 'utf8');

// 本物の stateActivityDecoration と sessionCardStateClasses（どちらも session-list.ts）を同じ環境で動かす。
function cardClassFixture(stopped: boolean) {
  const list = source('session-list.ts');
  const start = list.indexOf('export function stateActivityDecoration('), end = list.indexOf('\n// provider アイコンの定義', start);
  assert.ok(start >= 0 && end > start, 'stateActivityDecoration was not found in session-list.ts');
  assert.ok(list.slice(start, end).includes('export function sessionCardStateClasses('), 'sessionCardStateClasses sits next to stateActivityDecoration');
  const labels: Record<string, string> = { running: '実行中', standby: 'スタンバイ', waiting: '保留中' };
  const context: any = {
    stateLabel: (state: string) => labels[state] || state, ti18n: (_key: string, fallback: string) => fallback,
    sessionStartups: { forSession: () => undefined }, pendingSpawnConfirmationCount: () => 0, isSessionListStopped: () => stopped,
  };
  runInNewContext(compile(list.slice(start, end) + '\nglobalThis.api={stateActivityDecoration,sessionCardStateClasses,SESSION_CARD_STATE_CLASSES};'), context);
  const api = context.api;
  return {
    classesFor: (session: any) => plain(api.sessionCardStateClasses(session.state || 'standby', api.stateActivityDecoration(session).className)) as string[],
    classes: (state: string, activity: string) => plain(api.sessionCardStateClasses(state, activity)) as string[],
    all: plain(api.SESSION_CARD_STATE_CLASSES) as string[],
  };
}

test('a finished-not-opened session and an approval-pending session both get their frame class on the card', () => {
  assert.deepEqual(cardClassFixture(true).classesFor({ id: 1, state: 'standby' }), ['stopped-unseen']);
  assert.deepEqual(cardClassFixture(false).classesFor({ id: 2, state: 'waiting', awaiting_approval: true }), ['waiting', 'awaiting-approval']);
  assert.deepEqual(cardClassFixture(false).classesFor({ id: 3, state: 'standby' }), [], 'an opened standby card has no frame');
});

test('card state classes follow the Hub state, and the startup ring does not frame the card', () => {
  const { classes, all } = cardClassFixture(false);
  assert.deepEqual(classes('running', ''), ['running']);
  assert.deepEqual(classes('standby', 'running'), [], 'the startup ring (activity "running") stays a pill decoration');
  assert.deepEqual(classes('standby', 'pending-spawn-confirm'), ['pending-spawn-confirm']);
  assert.deepEqual(classes('standby', 'awaiting-input'), []);
  assert.deepEqual(classes('standby', 'workflow-active'), []);
  for (const [state, activity] of [['running', ''], ['waiting', 'awaiting-approval'], ['standby', 'stopped-unseen'], ['standby', 'pending-spawn-confirm']]) {
    for (const name of classes(state, activity)) assert.ok(all.includes(name), `${name} is toggled by the in-place update`);
  }
});

test('the full render builds the card classes from the shared helper', () => {
  const list = source('session-list.ts');
  const render = list.slice(list.indexOf('export function renderSessionList('), list.indexOf('export function updateSessionCardStateInPlace('));
  assert.match(render, /sessionCardStateClasses\(state, activity\.className\)/);
  assert.match(render, /c\.className = 'card' \+ stateClasses \+/);
});

class FakeClassList {
  names = new Set<string>();
  toggle(name: string, on: boolean) { if (on) this.names.add(name); else this.names.delete(name); return on; }
  contains(name: string) { return this.names.has(name); }
}

// 本物の updateSessionCardStateInPlace を、本物の sessionCardStateClasses と合成のカードで動かす。
function inPlaceFixture() {
  const list = source('session-list.ts');
  const helperStart = list.indexOf('export const SESSION_CARD_STATE_CLASSES'), helperEnd = list.indexOf('\n// provider アイコンの定義', helperStart);
  const start = list.indexOf('export function updateSessionCardStateInPlace(');
  const tail = /\r?\n\}\r?\n/.exec(list.slice(start));
  assert.ok(helperStart >= 0 && helperEnd > helperStart, 'sessionCardStateClasses was not found in session-list.ts');
  assert.ok(start >= 0 && tail, 'updateSessionCardStateInPlace was not found in session-list.ts');
  const card = { classList: new FakeClassList(), querySelector: () => null };
  const session: any = { id: 5, state: 'standby' };
  let activityClassName = 'stopped-unseen';
  const context: any = {
    window: {}, CSS: { escape: (value: string) => value },
    document: { getElementById: (id: string) => (id === 'sessions' ? { querySelector: () => card } : null) },
    sessions: new Map([[5, session]]), activeSessionId: null,
    stateActivityDecoration: () => ({ className: activityClassName, iconKind: 'dot', label: '' }),
    safeClassToken, stateLabel: (value: string) => value, stateIconSvgHtml: () => '',
    updateProjectGroupStatusChipsForSession: () => {}, renderSessionStrip: () => {},
  };
  runInNewContext(compile(list.slice(helperStart, helperEnd) + '\n' + list.slice(start, start + tail.index + tail[0].length) + '\nglobalThis.update=updateSessionCardStateInPlace;'), context);
  return { card, session, setActivity: (value: string) => { activityClassName = value; }, update: () => context.update(5) as boolean };
}

test('the in-place update adds the stopped-unseen frame and removes it once the session is opened', () => {
  const fx = inPlaceFixture();
  assert.equal(fx.update(), true);
  assert.equal(fx.card.classList.contains('stopped-unseen'), true, 'finished, not opened: yellow frame');
  fx.setActivity(''); fx.update();
  assert.equal(fx.card.classList.contains('stopped-unseen'), false, 'opened: frame removed');
  fx.session.state = 'waiting'; fx.setActivity('awaiting-approval'); fx.update();
  assert.equal(fx.card.classList.contains('awaiting-approval'), true);
  assert.equal(fx.card.classList.contains('waiting'), true);
  fx.session.state = 'standby'; fx.setActivity(''); fx.update();
  assert.deepEqual([...fx.card.classList.names], [], 'every frame class is taken off again');
});

// CSS: 状態の枠は、同じ詳細度でカードの枠・影・左バーを決める規則より後ろに置く。
type CssRule = { index: number; selectors: string[]; body: string };
function cssRules(css: string): CssRule[] {
  const text = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: CssRule[] = [];
  const re = /([^{};]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) out.push({ index: out.length, selectors: m[1].split(',').map(s => s.trim()).filter(Boolean), body: m[2] });
  return out;
}
const targetsCard = (selector: string) => /\.card(?![-\w])/.test(selector);
const isFrameSelector = (selector: string) => /\.(?:awaiting-approval|stopped-unseen)(?![-\w])/.test(selector);
const FRAME_PROPS = /(?:^|;)\s*(?:border(?:-[\w-]+)?|box-shadow|background(?:-color)?|content|left|top|bottom|width|opacity|transform|animation(?:-name)?)\s*:/;

test('the state frame block comes after every card rule it overrides, and nothing later redraws the card frame', () => {
  const css = readWeb('styles.css');
  const all = cssRules(css);
  const frame = all.find(r => r.selectors.includes('.card.awaiting-approval') && r.body.includes('var(--state-appr)') && /box-shadow/.test(r.body));
  assert.ok(frame, 'the approval frame rule was not found');
  const lastAt = (selector: string) => {
    const found = all.filter(r => r.selectors.includes(selector));
    assert.ok(found.length > 0, `${selector} was not found`);
    return Math.max(...found.map(r => r.index));
  };
  for (const selector of ['.card.in-pane', '.card[draggable=true]::before', '.card[draggable=true]:hover::before', '.card.active', '.card.active::before', '.card[class*="card-color-"]', '.card[class*="card-color-"]::before', '.card.waiting::before']) {
    assert.ok(lastAt(selector) < frame.index, `${selector} must stay before the state frame block`);
  }
  for (const rule of all.filter(r => r.index > frame.index && r.selectors.some(targetsCard) && FRAME_PROPS.test(r.body))) {
    for (const selector of rule.selectors.filter(targetsCard)) assert.ok(isFrameSelector(selector), `${selector} after the state frame block would override it`);
  }
  assert.doesNotMatch(css, /\.card\.awaiting-approval\s*\{\s*background:\s*color-mix\(in srgb, var\(--warn-bg\)/, 'the old background-only approval rule is gone');
});

test('stylesheets linked after styles.css do not redraw the card frame, and both themes define the frame colors', () => {
  const links = [...readWeb('index.html').matchAll(/<link rel="stylesheet" href="\/(styles\/[\w-]+\.css)">/g)].map(m => m[1]);
  assert.ok(links.length > 0, 'no stylesheet is linked after styles.css');
  for (const file of links) {
    for (const rule of cssRules(readWeb(file))) {
      for (const selector of rule.selectors.filter(targetsCard)) assert.ok(!FRAME_PROPS.test(rule.body), `${file}: ${selector} would override the card state frame`);
    }
  }
  const css = readWeb('styles.css');
  assert.equal((css.match(/--state-appr:\s*#[0-9a-f]{6};/gi) || []).length, 2, 'dark and light themes define --state-appr');
  assert.equal((css.match(/--state-stop:\s*#[0-9a-f]{6};/gi) || []).length, 2, 'dark and light themes define --state-stop');
  assert.match(readWeb('styles/session-list-tabs.css'), /\.card-state-pill\.stopped-unseen\s*\{\s*color:\s*var\(--state-stop\);/);
  // The session strip's stopped-unseen mark uses the card frame's color, not a second yellow.
  assert.match(readWeb('styles/session-strip.css'), /\.session-strip-item\.stopped-unseen \.session-strip-state\s*\{\s*color:\s*var\(--state-stop\);/);
  assert.match(css, /\.card-state-pill\.awaiting-approval\s*\{[^}]*background:\s*var\(--state-appr\);/);
});
