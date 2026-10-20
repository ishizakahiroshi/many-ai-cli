import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { collectCardNotes, computeBaselines } from '../src/app/session-card-notes.js';
import { buildSidebarTree, flattenSidebarTree } from '../src/app/sidebar-tree.js';
import { abbreviateBranchName } from '../src/app/session-strip-branch.js';

// Execute the actual rendering functions with synthetic data. This is not screen acceptance.
const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');
const list = read('../src/app/session-list.ts');
const settings = read('../src/app/settings.ts');
const run = (source: string, ctx: any) => runInNewContext(new Bun.Transpiler({ loader: 'ts' }).transformSync(source.replace(/^export /gm, '')), ctx);
const between = (source: string, start: string, end: string) => source.slice(source.indexOf(start), source.indexOf(end, source.indexOf(start)));
const escapeHtml = (text: unknown) => String(text).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const t = (key: string, values: Record<string, unknown> = {}) => `${key}:${Object.values(values).join(',')}`;

function cards() {
  const sessions = new Map<number, any>([
    [1, { id: 1, cwd: 'example', branch: 'develop', permission_mode: 'plan', state: 'standby', label: '<Synthetic title>' }],
    [2, { id: 2, cwd: 'example', branch: 'develop', permission_mode: 'plan', state: 'standby' }],
    [3, { id: 3, cwd: 'other', parent_session_id: 1, branch: 'feature/example', permission_mode: 'plan', state: 'running' }],
  ]);
  const ctx: any = { sessions, sessionOrder: [1, 2, 3], collectCardNotes, computeBaselines, buildSidebarTree, flattenSidebarTree,
    t, ti18n: (key: string, _fallback: string, values: any) => t(key, values), escapeHtml,
    getDoneSummary: () => ({ text: 'Synthetic completion summary', kind: 'success' }),
    doneSummaryDisplayText: (done: any) => done?.text || '', doneSummaryLine: (text: string) => text,
    doneSummaryKindSuffix: () => 'success', getSessionCtxPct: () => ({ pct: 44, is1m: true }),
    getSessionAgentInfo: () => ({ model: 'Synthetic Model' }),
    sessionDisplayTitle: (s: any) => s.label || '', filterFirstMessage: (text: string) => text,
    stateActivityDecoration: (s: any) => ({ label: s.state }), stateLabel: (state: string) => state,
    providerDisplayName: () => 'Provider', providerIconHtml: () => '<svg></svg>',
    successorSessionIDOf: () => 0, permissionModeLabel: (value: string) => value,
    cardStatusRowHtml: () => '<span data-tooltip="Synthetic status">⚠</span>', openMemoCountForCwd: () => 2,
  };
  run(between(list, 'function cardDoneHtml', '// 1 枚のカードの 2 行目'), ctx);
  return { ctx, sessions };
}

test('compact rows retain live slots, escaped names and summaries, without 1M or instant informational tooltips', () => {
  const { ctx, sessions } = cards();
  const s = sessions.get(1);
  const html = ctx.compactCardHtml(s, '<span class="card-state-pill" data-tooltip="state"></span>', s.label, '', '', ctx.compactBaselinesBySession().get(1));
  for (const slot of ['card-done-slot', 'card-status-slot', 'card-ctx-slot', 'card-notes-slot', 'card-state-pill']) assert.ok(html.includes(slot));
  assert.ok(html.includes('&lt;Synthetic title&gt;'));
  assert.ok(html.includes('Synthetic completion summary'));
  assert.ok(html.includes('44%'));
  assert.ok(!html.includes('card-ctx-1m'));
  assert.ok(!html.includes('data-tooltip='));
  assert.ok(ctx.cardCtxHtml(s).includes('card-ctx-1m'), 'standard rendering retains 1M');
  for (const state of ['running', 'waiting']) {
    s.state = state;
    assert.equal(ctx.cardDoneHtml(s), '');
  }
});

test('project baselines include children from another cwd and recompute when metadata changes', () => {
  const { ctx, sessions } = cards();
  assert.equal(ctx.compactBaselinesBySession().get(3).branch, 'develop');
  assert.ok(ctx.cardNotesHtml(sessions.get(3), ctx.compactBaselinesBySession().get(3)).includes('card_note_branch'));
  sessions.get(1).permission_mode = 'auto';
  sessions.get(2).permission_mode = 'auto';
  assert.equal(ctx.compactBaselinesBySession().get(3).permissionMode, 'auto');
  sessions.get(3).note = '<Synthetic note>';
  const detail = ctx.compactCardDetailsHtml(3);
  assert.ok(detail.includes('card_note_permission'));
  assert.ok(detail.includes('&lt;Synthetic note&gt;'));
});

test('in-place updates keep compact slots current and do not restore 1M or duplicate tooltips', () => {
  const { ctx, sessions } = cards();
  const slots: Record<string, { innerHTML: string }> = Object.fromEntries(['card-status-slot', 'card-ctx-slot', 'card-done-slot', 'card-notes-slot'].map(key => [key, { innerHTML: '' }]));
  const card = { classList: { contains: () => true }, querySelector: (selector: string) => slots[selector.slice(1)] };
  let refreshed = 0;
  Object.assign(ctx, { document: { getElementById: () => ({ querySelector: () => card }) }, CSS: { escape: String }, refreshSessionCardPopover: () => refreshed++ });
  run(between(list, 'export function updateCardLiveInfo', '// 全カードの 2 行目'), ctx);
  sessions.get(1).note = '<New note>';
  ctx.updateCardLiveInfo(1);
  assert.ok(slots['card-notes-slot'].innerHTML.includes('&lt;New note&gt;'));
  assert.ok(!slots['card-ctx-slot'].innerHTML.includes('1m'));
  assert.ok(!slots['card-done-slot'].innerHTML.includes('data-tooltip'));
  sessions.get(1).state = 'running';
  ctx.updateCardLiveInfo(1);
  assert.equal(slots['card-done-slot'].innerHTML, '');
  assert.equal(refreshed, 2);
});

test('header omits empty launch/branch values and preserves Git-button focus without duplicate handlers', () => {
  let button: any = null;
  let markup = '';
  let replacements = 0;
  const document: any = { activeElement: null, getElementById: () => chip };
  const chip: any = { hidden: false, querySelector: () => button };
  Object.defineProperty(chip, 'innerHTML', { get: () => markup, set: (value: string) => {
    markup = value; replacements++;
    button = value.includes('session-info-branch') ? { listeners: [] as Function[], addEventListener(_type: string, fn: Function) { this.listeners.push(fn); }, focus() { document.activeElement = this; } } : null;
  } });
  const session: any = { id: 1, provider: 'example', state: 'standby', branch: '', permission_mode: '' };
  const opened: number[] = [];
  const ctx: any = { document, activeSessionId: 1, sessions: new Map([[1, session]]), escapeHtml, t,
    providerDisplayName: () => 'Provider', safeClassToken: String, providerIconHtml: () => '', getSessionAgentInfo: () => ({}),
    stateLabel: String, abbreviateBranchName, permissionModeChipHtml: (mode: string) => mode ? '<span>At launch: synthetic</span>' : '',
    openSessionGitTab: (id: number) => opened.push(id),
  };
  run(between(settings, 'export function renderSessionInfoChip', '// D12:'), ctx);
  ctx.renderSessionInfoChip();
  assert.equal(button, null);
  assert.ok(!markup.includes('At launch'));
  session.branch = 'feature/synthetic-branch-with-a-long-name'; session.permission_mode = 'plan';
  ctx.renderSessionInfoChip();
  assert.ok(markup.includes('At launch: synthetic'));
  assert.ok(markup.includes(abbreviateBranchName(session.branch)));
  button.focus(); const before = replacements;
  ctx.renderSessionInfoChip();
  assert.equal(replacements, before);
  assert.equal(button.listeners.length, 1);
  session.branch = 'feature/synthetic-other';
  ctx.renderSessionInfoChip();
  assert.equal(document.activeElement, button);
  button.listeners[0](); assert.deepEqual(opened, [1]);
});

test('density stays device-local, cleanup is wired, and popover summaries wrap', () => {
  const prefs = read('../src/app/user-prefs.ts');
  assert.ok(!prefs.slice(prefs.indexOf('export const _USER_PREFS_PATH_TO_LS')).includes('STORAGE_SIDEBAR_DENSITY_KEY'));
  assert.ok(settings.includes('localStorage.setItem(STORAGE_SIDEBAR_DENSITY_KEY, density)'));
  assert.match(settings, /export function setActiveTab\(sid, name\) \{\s*hideSessionCardPopover\(\)/);
  assert.match(list, /export function renderSessionList\(\) \{\s*hideSessionCardPopover\(\)/);
  assert.ok(read('../src/styles/session-card-compact.css').includes('.session-card-popover .card-done {\n  white-space: pre-wrap;'));
  const locales = ['en', 'ja', 'vi'].map(lang => JSON.parse(read(`../src/i18n/${lang}.json`)));
  const keys = Object.keys(locales[0]).filter(key => /^(?:settings_sidebar_density|sidebar_density_|card_note_)/.test(key));
  for (const locale of locales) for (const key of keys) assert.equal(typeof locale[key], 'string');
});


test('density defaults to standard and switches immediately without server preferences', () => {
  let compact = false;
  let renderCount = 0;
  let changed: (event: any) => void = () => {};
  const saved = new Map<string, string>();
  const select = { value: '', addEventListener: (_type: string, callback: any) => { changed = callback; } };
  const root = { classList: { toggle: (_name: string, value: boolean) => { compact = value; } } };
  const ctx: any = {
    document: { getElementById: (id: string) => id === 'sessions' ? root : select },
    localStorage: { getItem: (key: string) => saved.get(key), setItem: (key: string, value: string) => saved.set(key, value) },
    STORAGE_SIDEBAR_DENSITY_KEY: 'synthetic-density', queueMicrotask: (fn: Function) => fn(),
    renderSessionList: () => renderCount++, setUserPref: () => { throw new Error('must stay local'); },
  };
  run(between(settings, 'export function applySidebarDensity', 'export function applyFontSize'), ctx);
  const startup = between(settings, '  // Defer imported storage constants', '  applyLang(');
  run(startup, ctx);
  assert.equal(select.value, 'standard'); assert.equal(compact, false); assert.equal(renderCount, 0);
  run(between(settings, "  document.getElementById('sidebar-density-select')?.addEventListener", "  langEl.addEventListener"), ctx);
  changed({ target: { value: 'compact' } });
  assert.equal(saved.get('synthetic-density'), 'compact'); assert.equal(compact, true); assert.equal(renderCount, 1);
  changed({ target: { value: 'standard' } });
  assert.equal(saved.get('synthetic-density'), 'standard'); assert.equal(compact, false); assert.equal(renderCount, 2);
  saved.set('synthetic-density', 'compact'); run(startup, ctx);
  assert.equal(select.value, 'compact'); assert.equal(compact, true); assert.equal(renderCount, 3);
});
