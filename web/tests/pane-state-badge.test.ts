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
