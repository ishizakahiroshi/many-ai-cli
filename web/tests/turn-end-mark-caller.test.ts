import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import * as store from '../src/app/turn-end-notify-store.js';

// ws-client.ts の実際の呼び出し側を、session-spawn-caller.test.ts と同じやり方
// （ソースの該当範囲を切り出して最小の DOM / 写しで走らせる）で確かめる。
// これは呼び出し元の配線の証拠で、ブラウザの実画面・ネイティブ入力の受入ではない。
//
// 対象: docs/local/bugfix_session-stop-false-positive_2026-10-18.md の親レビュー R1〜R6
//  - R1/R2: 開始は Hub が確認した開始（turn_start / user_turn_started）だけ。端子出力の
//    running では開かない。
//  - R3: 終了は turn_end（Git 計測と独立した Additive Rust-only 通知）と done_summary。
//  - R4/R5: 開始は provider の task_started（turn_start, turn_id 付き）でも届き、接続直後は
//    Hub が今開いているターンを送る。
const wsText = readFileSync(new URL('../src/app/ws-client.ts', import.meta.url), 'utf8');
const compile = (text: string) =>
  new Bun.Transpiler({ loader: 'ts' }).transformSync(text.replace(/^import[\s\S]*?;\r?$/gm, '').replace(/^export /gm, ''));
const slice = (start: string, end: string) => {
  const a = wsText.indexOf(start);
  const b = wsText.indexOf(end, a);
  assert.ok(a >= 0 && b > a, `ws-client.ts の範囲が見つからない: ${start}`);
  return wsText.slice(a, b);
};
const iso = (ms: number) => new Date(ms).toISOString();

// applyTurnEndEvidence: 証拠 → 印（タブの数字・カードの黄色い●）と通知の呼び出し。
function evidenceCaller(options: { viewed?: boolean; bellOff?: boolean; settingEnabled?: boolean } = {}) {
  const calls = { marked: [] as number[], rendered: 0, sounded: 0, notified: [] as Array<{ id: number; startedAt?: number }> };
  const ctx: any = {
    ...store,
    isSessionCurrentlyViewed: () => options.viewed === true,
    noteSessionListTurnEnd: (id: number) => { calls.marked.push(id); },
    renderSessionList: () => { calls.rendered++; },
    isTurnEndBellOff: () => options.bellOff === true,
    isTurnEndNotifyEnabled: () => options.settingEnabled !== false,
    playNotificationSound: () => { calls.sounded++; },
    showDesktopTurnEndNotification: (id: number, startedAt?: number) => { calls.notified.push({ id, startedAt }); },
  };
  const text = compile(slice('function applyTurnEndEvidence', 'export function _connectWs()'));
  runInNewContext(`${text}\nglobalThis.applyEvidence = applyTurnEndEvidence;`, ctx);
  return { calls, apply: ctx.applyEvidence as (id: number, evidence: any) => void };
}

test('実際の呼び出し側: ターン終了の証拠で印を付けて描き直し、通知も出す', () => {
  const f = evidenceCaller();
  const id = 401, t0 = 41_000_000;
  store.noteTurnStarted(id, { startedAt: iso(t0) });
  f.apply(id, { at: iso(t0 + 30_000), startedAt: iso(t0), turnId: 'turn-1' });
  assert.deepEqual(f.calls.marked, [id]);
  assert.equal(f.calls.rendered, 1); // 印はタブの数字とカードの●に出る。描き直しが要る。
  assert.equal(f.calls.sounded, 1);
  assert.deepEqual(f.calls.notified, [{ id, startedAt: t0 }]);
});

test('実際の呼び出し側: 今見ているセッションでは印も通知も出さない', () => {
  const f = evidenceCaller({ viewed: true });
  const id = 402, t0 = 42_000_000;
  store.noteTurnStarted(id, { startedAt: iso(t0) });
  f.apply(id, { at: iso(t0 + 30_000), startedAt: iso(t0) });
  assert.deepEqual(f.calls.marked, []);
  assert.equal(f.calls.rendered, 0);
  assert.equal(f.calls.sounded, 0);
  assert.deepEqual(f.calls.notified, []);
});

test('実際の呼び出し側: セッションのベルが OFF なら印だけ付けて音は鳴らさない', () => {
  const f = evidenceCaller({ bellOff: true });
  const id = 403, t0 = 43_000_000;
  store.noteTurnStarted(id, { startedAt: iso(t0) });
  f.apply(id, { at: iso(t0 + 30_000), startedAt: iso(t0) });
  assert.deepEqual(f.calls.marked, [id]);
  assert.equal(f.calls.rendered, 1);
  assert.equal(f.calls.sounded, 0);
  assert.deepEqual(f.calls.notified, []);
});

// user_turn_started の枝: 確定入力だけがターンを開く（R1/R2）。
function userTurnCaller() {
  const dispatched: any[] = [];
  const ctx: any = {
    window: { dispatchEvent: (event: any) => { dispatched.push(event); } },
    CustomEvent: function (this: any, type: string, init: any) { this.type = type; this.detail = init && init.detail; },
    ...store,
    renderSessionList: () => { ctx.rendered++; },
    rendered: 0,
  };
  const branch = slice("  if (m.type === 'user_turn_started') {", "  if (m.type === 'approval_patterns_updated') {");
  runInNewContext(compile(`function apply(m:any) {\n${branch}\n}\nglobalThis.applyMessage = apply;`), ctx);
  return { dispatched, ctx, apply: ctx.applyMessage as (m: any) => void };
}

test('実際の user_turn_started の枝: 入力でターンを開き、開始時刻を Hub の値から取る', () => {
  const f = userTurnCaller();
  const id = 501, t0 = 51_000_000;
  f.apply({ type: 'user_turn_started', session_id: id, approval_source_epoch: 3, started_at: iso(t0) });
  assert.equal(store.isStoppedMarkTurnInProgress(id), true);
  assert.equal(f.ctx.rendered, 1); // 未確認印を隠すために描き直す
  assert.equal(f.dispatched.length, 1);
  assert.equal(f.dispatched[0].type, 'many-user-turn-started');
  // 同じターンの再送では描き直さない。
  f.apply({ type: 'user_turn_started', session_id: id, started_at: iso(t0) });
  assert.equal(f.ctx.rendered, 1);
});

test('実際の user_turn_started の枝: started_at が無い接続でもターンは開く', () => {
  const f = userTurnCaller();
  const id = 502;
  f.apply({ type: 'user_turn_started', session_id: id, approval_source_epoch: 1 });
  assert.equal(store.isStoppedMarkTurnInProgress(id), true);
});

// turn_start の枝: provider 自身の task_started でターンを開く（R4/R5）。
function turnStartCaller() {
  const ctx: any = {
    ...store,
    sessions: new Map([[1, { id: 1, provider: 'codex' }]]),
    renderSessionList: () => { ctx.rendered++; },
    rendered: 0,
  };
  const branch = slice("  if (m.type === 'turn_start') {", "  if (m.type === 'turn_end') {");
  runInNewContext(compile(`function apply(m:any) {\n${branch}\n}\nglobalThis.applyMessage = apply;`), ctx);
  return { ctx, apply: ctx.applyMessage as (m: any) => void };
}

test('実際の turn_start の枝: provider の開始でターンを開き、接続直後の復元でも描き直す', () => {
  const f = turnStartCaller();
  const id = 503, t0 = 53_000_000;
  f.apply({ type: 'turn_start', session_id: id, turn_id: 'turn-1', provider: 'codex', started_at: iso(t0) });
  assert.equal(store.isStoppedMarkTurnInProgress(id), true);
  assert.equal(f.ctx.rendered, 1);
  // 再接続で追跡が消えたあと、同じ開始が届けば Hub の状態から復元する（R5）。
  store.forgetTurnEndNotify(id);
  assert.equal(store.isStoppedMarkTurnInProgress(id), false);
  f.apply({ type: 'turn_start', session_id: id, turn_id: 'turn-1', provider: 'codex', started_at: iso(t0) });
  assert.equal(store.isStoppedMarkTurnInProgress(id), true);
  assert.equal(f.ctx.rendered, 2);
});

// turn_end の枝: Hub が確認した終了（Git 計測と独立）を証拠にする（R3）。
function turnEndCaller(provider = 'codex') {
  const evidence: any[] = [];
  const ctx: any = {
    sessions: new Map([[1, { id: 1, provider }]]),
    applyTurnEndEvidence: (id: number, record: any) => { evidence.push({ id, ...record }); },
  };
  const branch = slice("  if (m.type === 'turn_end') {", "  if (m.type === 'user_turn_started') {");
  runInNewContext(compile(`function apply(m:any) {\n${branch}\n}\nglobalThis.applyMessage = apply;`), ctx);
  return { evidence, apply: ctx.applyMessage as (m: any) => void };
}

test('実際の turn_end の枝: Git 差分 0・Git 管理外・直接入力でも届いた終了を証拠にする', () => {
  const f = turnEndCaller();
  f.apply({ type: 'turn_end', session_id: 1, turn_id: 'turn-3', provider: 'codex', started_at: iso(60_000_000), ended_at: iso(60_030_000) });
  assert.deepEqual(f.evidence, [{ id: 1, at: iso(60_030_000), startedAt: iso(60_000_000), turnId: 'turn-3' }]);
});

test('実際の turn_end の枝: ended_at の無いイベントは証拠にしない', () => {
  const f = turnEndCaller();
  f.apply({ type: 'turn_end', session_id: 1, turn_id: 'turn-4', provider: 'codex', started_at: iso(61_000_000), ended_at: '' });
  assert.deepEqual(f.evidence, []);
});

test('実際の git_turn の枝: 完了カード向けの既存イベントだけを出し、印の証拠には使わない', () => {
  // git_turn は Git の開始ベースラインと計測成功が前提（親レビュー R3）。印の証拠は
  // turn_end が持つので、git_turn は今までどおり CustomEvent だけを出す。
  const dispatched: any[] = [];
  const evidence: any[] = [];
  const ctx: any = {
    window: { dispatchEvent: (event: any) => { dispatched.push(event); } },
    CustomEvent: function (this: any, type: string, init: any) { this.type = type; this.detail = init && init.detail; },
    sessions: new Map([[1, { id: 1, provider: 'codex' }]]),
    applyTurnEndEvidence: (id: number, record: any) => { evidence.push({ id, ...record }); },
  };
  const branch = slice("  if (m.type === 'git_turn') {", "  if (m.type === 'turn_end') {");
  runInNewContext(compile(`function apply(m:any) {\n${branch}\n}\nglobalThis.applyMessage = apply;`), ctx);
  ctx.applyMessage({ type: 'git_turn', session_id: 1, turn: 3, started_at: iso(50_000_000), ended_at: iso(50_030_000), files_changed: 0, added: 0, removed: 0 });
  assert.equal(dispatched.length, 1);
  assert.equal(dispatched[0].type, 'many-git-turn');
  assert.deepEqual(evidence, []);
});

// done_summary の枝: 記録を保存してから証拠として渡す（通知本文が完了サマリーを読む）。
function doneSummaryCaller() {
  const order: string[] = [];
  const ctx: any = {
    sessions: new Map([[1, { id: 1, provider: 'claude' }]]),
    activeSessionId: 1,
    setDoneSummary: (id: number) => { order.push('summary:' + id); },
    applyTurnEndEvidence: (id: number) => { order.push('evidence:' + id); },
    updateCardLiveInfo: () => { order.push('card'); },
    syncLiveStatusDomForActive: () => { order.push('status'); },
  };
  const branch = slice("  if (m.type === 'done_summary') {", "  if (m.type === 'session_spawn_correlated') {");
  runInNewContext(compile(`function apply(m:any) {\n${branch}\n}\nglobalThis.applyMessage = apply;`), ctx);
  return { order, apply: ctx.applyMessage as (m: any) => void };
}

test('実際の done_summary の枝: 最後の session_update より後に届いても証拠として渡す', () => {
  const f = doneSummaryCaller();
  f.apply({ type: 'done_summary', session_id: 1, done_summary: { at: iso(60_000_000), fallback: false } });
  assert.deepEqual(f.order, ['summary:1', 'evidence:1', 'card', 'status']);
  // done_summary が無いメッセージでは何もしない。
  const empty = doneSummaryCaller();
  empty.apply({ type: 'done_summary', session_id: 1 });
  assert.deepEqual(empty.order, []);
});

test('通知と印の呼び出しはターン終了の証拠を扱う関数の中だけにある', () => {
  // 端子出力の遷移から通知・印を作る経路が残っていないこと。
  assert.ok(!wsText.includes('observeTurnEndCandidate'));
  assert.ok(!wsText.includes('turnEndRunningStartedAt'));
  assert.ok(!wsText.includes('observeStoppedMarkActivity'));
  const fn = slice('function applyTurnEndEvidence', 'export function _connectWs()');
  assert.ok(fn.includes('showDesktopTurnEndNotification') && fn.includes('playNotificationSound'));
  const outside = wsText.replace(/^import[\s\S]*?;\r?$/gm, '').replace(fn, '');
  assert.ok(!outside.includes('showDesktopTurnEndNotification'));
  assert.ok(!outside.includes('playNotificationSound'));
  // ターンの開始は、Hub が確認した開始の 2 枝（provider の turn_start と確定入力の
  // user_turn_started）だけで作る。
  assert.ok(wsText.includes('noteTurnStarted(m.session_id, { turnId: m.turn_id, startedAt: m.started_at })'));
  assert.ok(wsText.includes('noteTurnStarted(m.session_id, { startedAt: m.started_at })'));
  const startedOutside = outside
    .replace(slice("  if (m.type === 'turn_start') {", "  if (m.type === 'turn_end') {"), '')
    .replace(slice("  if (m.type === 'user_turn_started') {", "  if (m.type === 'approval_patterns_updated') {"), '');
  assert.ok(!startedOutside.includes('noteTurnStarted('));
});
