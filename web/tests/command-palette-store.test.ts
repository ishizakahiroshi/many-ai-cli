import { describe, expect, test } from 'bun:test';
import {
  COMMAND_PALETTE_COMMANDS,
  type CommandPaletteContext,
  commandPaletteDisabledReasonKey,
  commandPaletteMatchesQuery,
  filterCommandPaletteItems,
  isCommandPaletteCommandEnabled,
  listCommandPaletteCommands,
  nextCommandPaletteSelectionIndex,
  pickNextInOrder,
  pickNextPendingApprovalId,
  pickNextStandbySessionId,
} from '../src/app/command-palette-store.ts';

// 子 plan: docs/local/plan_ux-notify-palette-review_c3_palette.md 内部 C1。
//
// このモジュールは DOM にも i18n（t()）にも触れない。ラベルは呼び出し側が翻訳して渡す
// 想定なので、テストでは翻訳済みの体でラベル文字列を組み立てる。

function baseContext(overrides: Partial<CommandPaletteContext> = {}): CommandPaletteContext {
  return {
    pendingApprovalIds: [],
    standbySessionIds: [],
    activeSessionId: null,
    activeSessionHasWorkdir: false,
    ...overrides,
  };
}

describe('listCommandPaletteCommands', () => {
  test('空入力では定義順のまま全件を返す', () => {
    const items = listCommandPaletteCommands(baseContext());
    expect(items.map((item) => item.def.id)).toEqual(COMMAND_PALETTE_COMMANDS.map((def) => def.id));
  });

  test('承認待ちが 0 件のとき next-pending-approval は使えず、理由キーが付く', () => {
    const items = listCommandPaletteCommands(baseContext({ pendingApprovalIds: [] }));
    const item = items.find((i) => i.def.id === 'next-pending-approval');
    expect(item?.enabled).toBe(false);
    expect(item?.disabledReasonKey).toBe('palette_cmd_next_pending_approval_disabled');
  });

  test('承認待ちが 1 件以上あれば next-pending-approval は使える', () => {
    const items = listCommandPaletteCommands(baseContext({ pendingApprovalIds: [1] }));
    const item = items.find((i) => i.def.id === 'next-pending-approval');
    expect(item?.enabled).toBe(true);
    expect(item?.disabledReasonKey).toBeNull();
  });

  test('待機中セッションが 0 件のとき next-standby は使えない', () => {
    const items = listCommandPaletteCommands(baseContext({ standbySessionIds: [] }));
    const item = items.find((i) => i.def.id === 'next-standby');
    expect(item?.enabled).toBe(false);
  });

  test('アクティブセッションが無いと Review/Files/Git は使えない', () => {
    const items = listCommandPaletteCommands(baseContext({ activeSessionId: null, activeSessionHasWorkdir: false }));
    for (const id of ['open-review', 'open-files', 'open-git'] as const) {
      const item = items.find((i) => i.def.id === id);
      expect(item?.enabled).toBe(false);
      expect(item?.disabledReasonKey).not.toBeNull();
    }
  });

  test('アクティブセッションに作業ディレクトリが無いと Review/Files/Git は使えない', () => {
    const items = listCommandPaletteCommands(baseContext({ activeSessionId: 5, activeSessionHasWorkdir: false }));
    expect(items.find((i) => i.def.id === 'open-review')?.enabled).toBe(false);
  });

  test('アクティブセッションに作業ディレクトリがあれば Review/Files/Git は使える', () => {
    const items = listCommandPaletteCommands(baseContext({ activeSessionId: 5, activeSessionHasWorkdir: true }));
    expect(items.find((i) => i.def.id === 'open-review')?.enabled).toBe(true);
    expect(items.find((i) => i.def.id === 'open-files')?.enabled).toBe(true);
    expect(items.find((i) => i.def.id === 'open-git')?.enabled).toBe(true);
  });

  test('新規セッション・設定・ショートカット一覧は常に使える', () => {
    const items = listCommandPaletteCommands(baseContext());
    for (const id of ['new-session', 'open-settings-notify', 'open-settings-approval', 'show-shortcuts'] as const) {
      expect(items.find((i) => i.def.id === id)?.enabled).toBe(true);
    }
  });
});

describe('commandPaletteMatchesQuery / filterCommandPaletteItems', () => {
  test('空クエリは常に一致する', () => {
    expect(commandPaletteMatchesQuery('Go to next pending approval', ['approval', '承認'], '')).toBe(true);
  });

  test('英語キーワードで一致する', () => {
    expect(commandPaletteMatchesQuery('次の承認待ちへ', ['approval', '承認'], 'approval')).toBe(true);
  });

  test('日本語キーワードで一致する', () => {
    expect(commandPaletteMatchesQuery('Go to next pending approval', ['approval', '承認'], '承認')).toBe(true);
  });

  test('ラベル自体への一致でも拾う', () => {
    expect(commandPaletteMatchesQuery('New session', ['new', 'session', '新規'], 'session')).toBe(true);
  });

  test('どこにも一致しなければ false', () => {
    expect(commandPaletteMatchesQuery('New session', ['new', 'session', '新規'], 'xyz')).toBe(false);
  });

  test('filterCommandPaletteItems は空入力で全件をそのままの順で返す', () => {
    const items = [
      { label: 'A', keywords: ['a'] },
      { label: 'B', keywords: ['b'] },
    ];
    expect(filterCommandPaletteItems(items, '')).toEqual(items);
  });

  test('filterCommandPaletteItems は一致するものだけに絞る', () => {
    const items = [
      { label: 'Open Review', keywords: ['review', 'レビュー'] },
      { label: 'Open Files', keywords: ['files', 'ファイル'] },
    ];
    expect(filterCommandPaletteItems(items, 'レビュー').map((i) => i.label)).toEqual(['Open Review']);
  });
});

describe('commandPaletteDisabledReasonKey', () => {
  test('使えるコマンドは null', () => {
    expect(commandPaletteDisabledReasonKey('new-session', baseContext())).toBeNull();
  });

  test('使えないコマンドは理由キーを返す', () => {
    expect(commandPaletteDisabledReasonKey('next-standby', baseContext({ standbySessionIds: [] }))).toBe('palette_cmd_next_standby_disabled');
  });
});

describe('isCommandPaletteCommandEnabled', () => {
  test('COMMAND_PALETTE_COMMANDS の全 id で例外なく判定できる', () => {
    for (const def of COMMAND_PALETTE_COMMANDS) {
      expect(() => isCommandPaletteCommandEnabled(def.id, baseContext())).not.toThrow();
    }
  });
});

describe('pickNextInOrder / pickNextPendingApprovalId / pickNextStandbySessionId', () => {
  test('今の次を返す', () => {
    expect(pickNextInOrder([1, 2, 3], [1, 2, 3], 1)).toBe(2);
  });

  test('末尾なら先頭へ戻る', () => {
    expect(pickNextInOrder([1, 2, 3], [1, 2, 3], 3)).toBe(1);
  });

  test('候補が 0 件なら null', () => {
    expect(pickNextInOrder([1, 2, 3], [], 1)).toBeNull();
  });

  test('今の ID が候補に無ければ候補の先頭を返す', () => {
    expect(pickNextInOrder([1, 2, 3], [2, 3], 1)).toBe(2);
  });

  test('今の ID が null なら候補の先頭を返す', () => {
    expect(pickNextInOrder([1, 2, 3], [2, 3], null)).toBe(2);
  });

  test('pickNextPendingApprovalId は今の次を返し、末尾で先頭へ戻り、0 件で null', () => {
    expect(pickNextPendingApprovalId([1, 2, 3], [1, 2, 3], 1)).toBe(2);
    expect(pickNextPendingApprovalId([1, 2, 3], [1, 2, 3], 3)).toBe(1);
    expect(pickNextPendingApprovalId([1, 2, 3], [], 1)).toBeNull();
  });

  test('pickNextPendingApprovalId は候補が自分 1 件だけなら自分に留まる（cycle back to self）', () => {
    expect(pickNextPendingApprovalId([1, 2, 3], [2], 2)).toBe(2);
  });

  test('pickNextStandbySessionId は今のセッションを候補から除く', () => {
    // 今のセッション(1)が standby の候補集合に含まれていても、次は他のセッションになる。
    expect(pickNextStandbySessionId([1, 2, 3], [1, 2, 3], 1)).toBe(2);
  });

  test('pickNextStandbySessionId は待機中が自分だけなら null（自分には戻らない）', () => {
    expect(pickNextStandbySessionId([1, 2, 3], [1], 1)).toBeNull();
  });

  test('pickNextStandbySessionId は待機中が 0 件なら null', () => {
    expect(pickNextStandbySessionId([1, 2, 3], [], 1)).toBeNull();
  });
});

describe('nextCommandPaletteSelectionIndex', () => {
  test('0 件のときは 0 を返す（例外を投げない）', () => {
    expect(nextCommandPaletteSelectionIndex(0, 0, 1)).toBe(0);
    expect(nextCommandPaletteSelectionIndex(0, 0, -1)).toBe(0);
  });

  test('1 件のときはどちら向きでも同じ位置に留まる', () => {
    expect(nextCommandPaletteSelectionIndex(1, 0, 1)).toBe(0);
    expect(nextCommandPaletteSelectionIndex(1, 0, -1)).toBe(0);
  });

  test('末尾から ↓ で先頭へ戻る', () => {
    expect(nextCommandPaletteSelectionIndex(3, 2, 1)).toBe(0);
  });

  test('先頭から ↑ で末尾へ戻る', () => {
    expect(nextCommandPaletteSelectionIndex(3, 0, -1)).toBe(2);
  });

  test('通常の移動', () => {
    expect(nextCommandPaletteSelectionIndex(3, 0, 1)).toBe(1);
    expect(nextCommandPaletteSelectionIndex(3, 1, -1)).toBe(0);
  });
});
