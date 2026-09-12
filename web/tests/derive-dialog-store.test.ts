import { beforeEach, describe, expect, test } from 'bun:test';
import {
  availableDeriveKinds,
  buildDeriveBody,
  deriveRequestPath,
  deriveSubmitBlockedReason,
  type DeriveSelection,
} from '../src/app/derive-dialog-store.ts';
import { setLaunchOptionChoices } from '../src/app/spawn-confirm-store.ts';

// 子 plan: docs/local/plan_derived-session-launch_c3_derive-launch.md 内部 C2。
//
// 固定したいのは「種別で何が載り、何が載らないか」。引き継ぎと子は呼ぶ API が別で、
// 片方にしか無いキーがある。どちらにも載ってしまうと、Hub 側では「conductor が
// 起動した子」と「画面から立てた後継」の区別が付かなくなる。
//
// derive-dialog-store.ts は DOM に触れないので bun:test から直接 import できる
// （spawn-confirm-store.test.ts と同じ理由）。

beforeEach(() => {
  // effort の写像は /api/info 由来のモジュール singleton。テスト間で持ち越さない。
  setLaunchOptionChoices({ effort_levels: { claude: ['low', 'high'], codex: ['minimal'] } });
});

function handoff(overrides: Partial<DeriveSelection> = {}): DeriveSelection {
  return {
    kind: 'handoff',
    sourceSessionID: 7,
    provider: 'codex',
    cwd: '/tmp/sample-repo',
    prompt: '# 引き継ぎ\n前任の看板',
    ...overrides,
  };
}

function child(overrides: Partial<DeriveSelection> = {}): DeriveSelection {
  return {
    kind: 'child',
    sourceSessionID: 7,
    provider: 'claude',
    role: 'review',
    prompt: 'review',
    ...overrides,
  };
}

describe('deriveRequestPath', () => {
  test('handoff goes through the ordinary spawn entry point', () => {
    expect(deriveRequestPath(handoff())).toBe('/api/spawn');
  });

  test('child goes through the orchestration entry point of its parent', () => {
    expect(deriveRequestPath(child({ sourceSessionID: 12 }))).toBe('/api/sessions/12/spawn-child');
  });
});

describe('buildDeriveBody: 種別ごとに載る項目', () => {
  test('handoff carries handoff_from and never carries origin', () => {
    const body = buildDeriveBody(handoff());
    expect(body.handoff_from).toBe(7);
    expect(body.cwd).toBe('/tmp/sample-repo');
    expect(body.initial_prompt).toBe('# 引き継ぎ\n前任の看板');
    expect('origin' in body).toBe(false);
    expect('role' in body).toBe(false);
    expect('same_tree' in body).toBe(false);
  });

  test('child carries origin: "ui" and never carries handoff_from', () => {
    const body = buildDeriveBody(child());
    expect(body.origin).toBe('ui');
    expect(body.role).toBe('review');
    expect('handoff_from' in body).toBe(false);
    expect('cwd' in body).toBe(false);
  });

  test('child omits initial_prompt when the text area is empty', () => {
    expect('initial_prompt' in buildDeriveBody(child({ prompt: '   ' }))).toBe(false);
  });
});

describe('buildDeriveBody: 省略可の項目はキーごと載せない', () => {
  test('effort only rides along for a provider that has a mapping', () => {
    expect(buildDeriveBody(child({ provider: 'claude', effort: 'high' })).effort).toBe('high');
    // copilot に effort の写像は無い。選択が残っていても送らない。
    expect('effort' in buildDeriveBody(child({ provider: 'copilot', effort: 'high' }))).toBe(false);
    // 前の provider の値が残ったままでも、今の provider の候補に無ければ送らない。
    expect('effort' in buildDeriveBody(child({ provider: 'codex', effort: 'high' }))).toBe(false);
  });

  test('permission_preset is sent exactly as chosen, and omitted when unchosen', () => {
    expect(buildDeriveBody(child({ permissionPreset: 'bounded' })).permission_preset).toBe('bounded');
    expect(buildDeriveBody(handoff({ permissionPreset: 'full' })).permission_preset).toBe('full');
    expect('permission_preset' in buildDeriveBody(child())).toBe(false);
    expect('permission_preset' in buildDeriveBody(child({ permissionPreset: '  ' }))).toBe(false);
  });

  test('same_tree is sent only when checked; unchecked sends no key at all', () => {
    expect(buildDeriveBody(child({ sameTree: true })).same_tree).toBe(true);
    // false を送ると orchestration.worktree_auto の設定を黙って上書きしてしまう。
    expect('same_tree' in buildDeriveBody(child({ sameTree: false }))).toBe(false);
    expect('same_tree' in buildDeriveBody(child())).toBe(false);
  });

  test('model / execution_mode / subscription_profile_id follow the same rule', () => {
    const filled = buildDeriveBody(child({
      model: 'claude-sonnet-4-5',
      executionMode: 'interactive',
      subscriptionProfileID: 'profile-a',
    }));
    expect(filled.model).toBe('claude-sonnet-4-5');
    expect(filled.execution_mode).toBe('interactive');
    expect(filled.subscription_profile_id).toBe('profile-a');
    const empty = buildDeriveBody(child({ model: '', executionMode: '', subscriptionProfileID: '' }));
    expect('model' in empty).toBe(false);
    expect('execution_mode' in empty).toBe(false);
    expect('subscription_profile_id' in empty).toBe(false);
  });

  test('risk_confirmed is a handoff-only resend flag', () => {
    expect(buildDeriveBody(handoff({ riskConfirmed: true })).risk_confirmed).toBe(true);
    expect('risk_confirmed' in buildDeriveBody(handoff())).toBe(false);
    // 子の risk は spawn-child 側が段の表から決める。画面からは送らない。
    expect('risk_confirmed' in buildDeriveBody(child({ riskConfirmed: true }))).toBe(false);
  });

  // 「この役割では次回もこの段を使う」（子 plan:
  // docs/local/plan_derived-session-launch_c2_permission-tiers.md 内部 C6）。
  test('remember_permission is child-only', () => {
    expect(buildDeriveBody(child({ rememberPermission: true })).remember_permission).toBe(true);
    // 引き継ぎには役割が無いので、載せてはいけない。
    expect('remember_permission' in buildDeriveBody(handoff({ rememberPermission: true }))).toBe(false);
  });

  test('remember_permission carries false as a value, and undefined as no field at all', () => {
    // false は「記憶を消す」という指示なので、値として送る（省略と同じにしない）。
    const off = buildDeriveBody(child({ rememberPermission: false }));
    expect('remember_permission' in off).toBe(true);
    expect(off.remember_permission).toBe(false);
    // 欄を知らない呼び出しではキーごと現れない＝ Hub の記憶は動かない。
    expect('remember_permission' in buildDeriveBody(child())).toBe(false);
  });
});

describe('availableDeriveKinds', () => {
  test('a finished session can only be handed off', () => {
    expect(availableDeriveKinds(false)).toEqual(['handoff']);
  });

  test('a live session can do both', () => {
    expect(availableDeriveKinds(true)).toEqual(['handoff', 'child']);
  });
});

describe('deriveSubmitBlockedReason', () => {
  test('a provider is always required', () => {
    expect(deriveSubmitBlockedReason(handoff({ provider: '' }))).toBe('provider');
    expect(deriveSubmitBlockedReason(child({ provider: '' }))).toBe('provider');
  });

  test('handoff needs text, child needs a role', () => {
    expect(deriveSubmitBlockedReason(handoff({ prompt: '   ' }))).toBe('prompt');
    expect(deriveSubmitBlockedReason(child({ role: '' }))).toBe('role');
    // 子の文面は空でよい（役割だけで立てられる）。
    expect(deriveSubmitBlockedReason(child({ prompt: '' }))).toBe('');
    expect(deriveSubmitBlockedReason(handoff())).toBe('');
  });
});
