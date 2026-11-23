import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { collectCardNotes, computeBaselines } from '../src/app/session-card-notes.js';
import { buildSidebarTree, flattenSidebarTree } from '../src/app/sidebar-tree.js';
import { filterSessionListTree } from '../src/app/session-list-tab-state.js';
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
    // The sidebar's own tab filter is identity here; the color filter goes through the real tree.
    visibleSessionListTree: (tree: unknown) => tree, activeSessionColorFilter: '',
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

test('while a color filter is on, the baseline compares only the cards the group still shows', () => {
  const { ctx, sessions } = cards();
  sessions.set(4, { id: 4, cwd: 'example', branch: 'develop', permission_mode: 'auto', state: 'standby', color: 'red' });
  sessions.set(5, { id: 5, cwd: 'example', branch: 'develop', permission_mode: 'auto', state: 'standby', color: 'red' });
  ctx.sessionOrder = [1, 2, 3, 4, 5];
  assert.equal(ctx.compactBaselinesBySession().get(4).permissionMode, 'plan');
  assert.ok(ctx.cardNotesHtml(sessions.get(4), ctx.compactBaselinesBySession().get(4)).includes('card_note_permission'));
  ctx.activeSessionColorFilter = 'red';
  const filtered = ctx.compactBaselinesBySession();
  assert.equal(filtered.has(1), false, 'filtered-out cards are not counted');
  assert.equal(filtered.get(4).permissionMode, 'auto');
  assert.equal(ctx.cardNotesHtml(sessions.get(4), filtered.get(4)), '');
  assert.ok(!ctx.compactCardDetailsHtml(4).includes('card_note_permission'));
});

test('a sidebar tab that splits a project also narrows the baseline to the cards that tab shows', () => {
  const { ctx, sessions } = cards();
  sessions.set(4, { id: 4, cwd: 'example', branch: 'develop', permission_mode: 'auto', state: 'standby' });
  sessions.set(5, { id: 5, cwd: 'example', branch: 'develop', permission_mode: 'auto', state: 'standby' });
  ctx.sessionOrder = [1, 2, 3, 4, 5];
  // Same filter as session-list-tabs.ts: 1-3 belong to tab "a", 4-5 to tab "b".
  const owners = new Map([[1, 'a'], [2, 'a'], [3, 'a'], [4, 'b'], [5, 'b']]);
  let selected = 'a';
  ctx.visibleSessionListTree = (tree: any) => filterSessionListTree(tree, owners, selected);
  const inA = ctx.compactBaselinesBySession();
  assert.equal(inA.has(4), false, 'cards of the other tab are not counted');
  assert.equal(inA.get(1).permissionMode, 'plan');
  selected = 'b';
  const inB = ctx.compactBaselinesBySession();
  assert.equal(inB.has(1), false);
  assert.equal(inB.get(4).permissionMode, 'auto');
  assert.equal(ctx.cardNotesHtml(sessions.get(4), inB.get(4)), '', 'no ● for the usual mode of the cards the tab shows');
});

test('details carry the explanations the compact row drops, without repeating visible text', () => {
  const { ctx, sessions } = cards();
  ctx.cardStatusRowHtml = () => '<span class="card-longproc" data-tooltip="Synthetic: send ESC instead of resending">⚠ Long 5m</span>';
  const detail = ctx.compactCardDetailsHtml(1);
  assert.ok(detail.includes('⚠ Long 5m'));
  assert.ok(detail.includes('Synthetic: send ESC instead of resending'));
  assert.ok(detail.includes('memo_session_open_count:2'));
  assert.ok(!detail.includes('data-tooltip='));
  ctx.cardStatusRowHtml = () => '<span class="card-end-reason" data-tooltip="Synthetic end">Synthetic end</span>';
  assert.equal(ctx.compactCardDetailsHtml(1).split('Synthetic end').length - 1, 1);
  sessions.get(1).state = 'running';
  assert.ok(!ctx.compactCardDetailsHtml(1).includes('<div></div>'), 'empty rows are not drawn');
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
    openSessionGitTab: (id: number) => opened.push(id), executionModeKind: () => 'interactive',
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

test('the header chip never clips the permission warning; only variable-length names are capped', () => {
  const css = read('../src/styles/tabs.css');
  const rule = (selector: string) => {
    const start = css.indexOf(`\n${selector} {`);
    assert.ok(start >= 0, `${selector} has its own rule`);
    return css.slice(start, css.indexOf('}', start)).replace(/\/\*[\s\S]*?\*\//g, '');
  };
  // A width cap with overflow: hidden on the whole chip cuts the non-shrinking items off without an
  // ellipsis (the bypass warning first at ~960px). The tab bar's own horizontal scroll is the last resort.
  const chip = rule('.session-info-chip');
  assert.doesNotMatch(chip, /max-width/);
  assert.doesNotMatch(chip, /overflow\s*:\s*hidden/);
  // Base rule only (used at phone widths). At PC widths the @media block below lets the chip shrink.
  assert.match(chip, /flex-shrink: 0;/);
  assert.match(css.slice(css.indexOf('\n#unified-tab-bar {\n')).split('}')[0], /overflow-x: auto;/);
  // At PC widths (the rules that actually apply there sit indented inside @media, which rule() skips),
  // the branch shrinks like the other names down to 4 characters plus the button's padding and border,
  // instead of keeping its full width and bringing back the tab bar's horizontal scroll.
  const pcStart = css.indexOf('\n@media (min-width: 721px) {\n  #unified-tab-bar { container:');
  assert.ok(pcStart >= 0, 'the PC-width tab bar block exists');
  const pc = css.slice(pcStart, css.indexOf('\n}\n', pcStart)).replace(/\/\*[\s\S]*?\*\//g, '');
  assert.match(pc, /\.session-info-chip \{ flex: 0 1 auto; \}/);
  assert.match(pc, /\.session-info-chip \.session-info-branch \{ min-width: calc\(4ch \+ 26px\); \}/);
  assert.match(pc, /\.session-info-chip \.session-info-branch,\s*\.session-info-chip \.tsb-agent-name \{ flex-shrink: 4; \}/);
  // The launch permission chip keeps its full width and is never in a shrinking/ellipsis rule.
  assert.match(rule('.session-info-chip .session-permission-chip'), /flex-shrink: 0;/);
  for (const selector of ['.session-permission-chip', '.session-mode-chip', '.tsb-pill', '.sid'].map(name => `.session-info-chip ${name}`)) {
    const shrinking = css.split('}').filter(block => block.includes(selector) && /text-overflow|min-width:\s*0/.test(block));
    assert.deepEqual(shrinking, [], `${selector} is not truncated in the header`);
  }
  // Each variable-length name has its own cap with an ellipsis instead.
  const styles = read('../src/styles.css');
  assert.match(styles, /\.card-model\.card-model--with-icon \.card-model-text \{[^}]*max-width: 10ch;[^}]*text-overflow: ellipsis;/);
  assert.match(read('../src/styles/token-statusbar.css'), /\.tsb-agent-name \{[^}]*max-width: 10ch;[^}]*text-overflow: ellipsis;/);
  for (const [selector, cap] of [['.session-info-chip .card-subscription', '14ch'], ['.session-info-chip .session-info-branch', '18ch']]) {
    const capped = rule(selector);
    assert.match(capped, new RegExp(`max-width: ${cap};`));
    assert.match(capped, /text-overflow: ellipsis;/);
  }
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
