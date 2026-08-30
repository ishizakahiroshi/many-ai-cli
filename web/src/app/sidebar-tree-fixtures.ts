// sidebar-tree-fixtures.ts — sidebar-tree.ts の不変条件を固定する。
//
// ここが赤くなったら、直すのはテストではなく実装。配置の規則（木の形はデータだけが
// 決める / 兄弟順と折りたたみ以外でノードが器を越えない）を壊す変更が入っている。
//
// 由来: docs/local/plan_sidebar-placement-tree_c2_tree-fn.md

import assert from 'node:assert/strict';
import test from 'node:test';
import type { SessionSnapshot } from '../types/proto.js';
import {
  NO_PROJECT_KEY,
  buildSidebarTree,
  deriveProjectKeyFromCwd,
  flattenSidebarTree,
  moveToSiblingFront,
} from './sidebar-tree.js';

// 合成データ。実在のパスは書かない（公開ファイルの層 1 防御）。
const MAIN = '/repos/main-app';
const OTHER = '/repos/other-app';
/** relay の子が実際に居る作業ディレクトリ。末尾は常に relay になる。 */
const RELAY_CWD = MAIN + '/.many-ai-cli/worktrees/rl-8f3/relay';

function ses(id: number, extra: Partial<SessionSnapshot> = {}): SessionSnapshot {
  return { id, cwd: MAIN, project_id: MAIN, ...extra } as SessionSnapshot;
}

/** 木の中から、その ID を含むプロジェクトノードの key を返す。 */
function projectKeyOf(tree: ReturnType<typeof buildSidebarTree>, id: number): string | null {
  for (const project of tree) {
    const stack = [...project.children];
    while (stack.length) {
      const node = stack.pop();
      if (!node) continue;
      if (node.id === id) return project.key;
      stack.push(...node.children);
    }
  }
  return null;
}

function nodeOf(tree: ReturnType<typeof buildSidebarTree>, id: number) {
  for (const project of tree) {
    const stack = [...project.children];
    while (stack.length) {
      const node = stack.pop();
      if (!node) continue;
      if (node.id === id) return node;
      stack.push(...node.children);
    }
  }
  return undefined;
}

// ---- 不変条件 1 -------------------------------------------------------------

test('不変条件1: 子は cwd が違っても、ルート祖先と同じプロジェクトの下に入る', () => {
  // relay の子は worktree で動くので cwd も project_id も親と一致しないことがある。
  // 以前はここで cwd の末尾（relay）が箱になり、親と別グループへ落ちていた。
  const tree = buildSidebarTree({
    sessions: [
      ses(12),
      ses(13, { cwd: RELAY_CWD, project_id: MAIN, parent_session_id: 12, role: 'implementation' }),
      ses(14, { cwd: RELAY_CWD, project_id: '', parent_session_id: 12, role: 'review' }),
    ],
    order: [12, 13, 14],
  });

  assert.equal(tree.length, 1, '親と子で箱が分かれてはいけない');
  assert.equal(projectKeyOf(tree, 13), MAIN);
  assert.equal(projectKeyOf(tree, 14), MAIN, 'project_id が取れない子も親の箱に入る');
  assert.equal(nodeOf(tree, 13)?.depth, 1);
  assert.equal(nodeOf(tree, 14)?.depth, 1);
});

test('不変条件1: 別リポジトリの子は、親と同じ箱には入らない', () => {
  const tree = buildSidebarTree({
    sessions: [
      ses(12),
      ses(13, { parent_session_id: 12 }),
      ses(21, { cwd: OTHER, project_id: OTHER }),
      ses(22, { cwd: OTHER + '/.many-ai-cli/worktrees/rl-2a1/relay', project_id: OTHER, parent_session_id: 21 }),
    ],
    order: [12, 13, 21, 22],
  });

  assert.equal(tree.length, 2);
  assert.equal(projectKeyOf(tree, 13), MAIN);
  assert.equal(projectKeyOf(tree, 22), OTHER, '別リポの子が本体リポの箱へ混ざってはいけない');
});

// ---- 不変条件 2 -------------------------------------------------------------

test('不変条件2: pinned が true でも、そのセッションは自分の箱の中に居る', () => {
  // 以前は __pinned__ という架空の箱を作り、プロジェクトからカードを引き抜いていた。
  const tree = buildSidebarTree({
    sessions: [ses(7), ses(9, { pinned: true }), ses(18, { cwd: OTHER, project_id: OTHER })],
    order: [7, 9, 18],
  });

  assert.deepEqual(tree.map(project => project.key).sort(), [MAIN, OTHER].sort());
  assert.equal(projectKeyOf(tree, 9), MAIN, 'pinned は所属を変えない');
});

test('不変条件2: 色フィルタで親が消えても、子は捨てられず同じ箱に残る', () => {
  const tree = buildSidebarTree({
    sessions: [
      ses(12, { color: 'blue' }),
      ses(13, { color: 'red', parent_session_id: 12 }),
    ],
    order: [12, 13],
    colorFilter: 'red',
  });

  assert.equal(projectKeyOf(tree, 13), MAIN);
  assert.equal(nodeOf(tree, 12), undefined, 'フィルタに合わないセッションは出ない');
  assert.equal(nodeOf(tree, 13)?.depth, 0, '親が消えたぶん段が上がる');
});

// ---- 不変条件 3 -------------------------------------------------------------

test('不変条件3: 線形の並びは木を深さ優先でたどった 1 経路になる', () => {
  const tree = buildSidebarTree({
    sessions: [
      ses(12),
      ses(13, { parent_session_id: 12 }),
      ses(14, { parent_session_id: 12 }),
      ses(7),
      ses(18, { cwd: OTHER, project_id: OTHER }),
    ],
    order: [12, 13, 14, 7, 18],
  });

  assert.deepEqual(flattenSidebarTree(tree), [12, 13, 14, 7, 18]);
});

test('不変条件3: 兄弟順を入れ替えると、線形の並びも同じ順で入れ替わる', () => {
  const sessions = [ses(12), ses(13, { parent_session_id: 12 }), ses(14, { parent_session_id: 12 }), ses(7)];

  const before = flattenSidebarTree(buildSidebarTree({ sessions, order: [12, 13, 14, 7] }));
  const after = flattenSidebarTree(buildSidebarTree({ sessions, order: [7, 12, 14, 13] }));

  assert.deepEqual(before, [12, 13, 14, 7]);
  assert.deepEqual(after, [7, 12, 14, 13], '親の位置も子の順序も兄弟順から決まる');
});

// ---- 回帰させたくない具体ケース ---------------------------------------------

test('relay の 3 役が conductor の直下に兄弟順で並ぶ', () => {
  const tree = buildSidebarTree({
    sessions: [
      ses(12, { role: 'conductor', orchestration_id: 'rl-8f3' }),
      ses(15, { parent_session_id: 12, role: 'review' }),
      ses(13, { parent_session_id: 12, role: 'implementation' }),
      ses(14, { parent_session_id: 12, role: 'implementation-strong' }),
    ],
    order: [12, 13, 14, 15],
  });

  assert.equal(tree.length, 1);
  assert.deepEqual(tree[0].children.map(node => node.id), [12]);
  assert.deepEqual(tree[0].children[0].children.map(node => node.id), [13, 14, 15]);
});

test('親が一覧に居ない子は捨てられず、ルートとして出る', () => {
  const tree = buildSidebarTree({
    sessions: [ses(13, { parent_session_id: 999 })],
    order: [13],
  });

  assert.equal(nodeOf(tree, 13)?.depth, 0);
  assert.equal(projectKeyOf(tree, 13), MAIN);
});

test('孫は depth 1 へ潰れ、カードが消えない', () => {
  const tree = buildSidebarTree({
    sessions: [
      ses(12),
      ses(13, { parent_session_id: 12 }),
      ses(14, { parent_session_id: 13 }),
      ses(15, { parent_session_id: 14 }),
    ],
    order: [12, 13, 14, 15],
  });

  assert.deepEqual(flattenSidebarTree(tree), [12, 13, 14, 15], '孫・ひ孫も消えない');
  assert.equal(nodeOf(tree, 14)?.depth, 1);
  assert.equal(nodeOf(tree, 15)?.depth, 1, '深さ上限を上げても 2 段までに収まる');
});

test('親子が循環していても止まらず、全員がどこかの箱に入る', () => {
  const tree = buildSidebarTree({
    sessions: [ses(1, { parent_session_id: 2 }), ses(2, { parent_session_id: 1 })],
    order: [1, 2],
  });

  assert.equal(flattenSidebarTree(tree).length, 2);
});

test('sessionOrder に無いセッションは末尾に来る', () => {
  const tree = buildSidebarTree({
    sessions: [ses(7), ses(9), ses(11)],
    order: [11],
  });

  assert.deepEqual(flattenSidebarTree(tree), [11, 7, 9]);
});

test('★ のプロジェクトが先頭、その後は groupOrder の順', () => {
  const third = '/repos/third-app';
  const tree = buildSidebarTree({
    sessions: [ses(1), ses(2, { cwd: OTHER, project_id: OTHER }), ses(3, { cwd: third, project_id: third })],
    order: [1, 2, 3],
    groupOrder: [MAIN, OTHER, third],
    projectFavorites: [third],
  });

  assert.deepEqual(tree.map(project => project.key), [third, MAIN, OTHER]);
  assert.equal(tree[0].favorite, true);
});

test('project_id が無いセッションは cwd の末尾で箱を作り、cwd も無ければ器なしに入る', () => {
  const tree = buildSidebarTree({
    sessions: [
      ses(1, { project_id: '', cwd: '/tmp/scratch' }),
      ses(2, { project_id: '', cwd: '' }),
    ],
    order: [1, 2],
  });

  assert.equal(projectKeyOf(tree, 1), 'scratch');
  assert.equal(projectKeyOf(tree, 2), NO_PROJECT_KEY);
});

test('見出しの名前は key の末尾セグメント', () => {
  assert.equal(deriveProjectKeyFromCwd(MAIN), 'main-app');
  assert.equal(deriveProjectKeyFromCwd('C:\\repos\\main-app'), 'main-app');
  assert.equal(deriveProjectKeyFromCwd(''), '');

  const tree = buildSidebarTree({ sessions: [ses(1)], order: [1] });
  assert.equal(tree[0].label, 'main-app');
});

// ---- 「器の中で先頭へ」 ------------------------------------------------------

test('先頭へ: ルートセッションは同じプロジェクトの先頭へ動き、他のプロジェクトを越えない', () => {
  const sessions = [ses(7), ses(9), ses(18, { cwd: OTHER, project_id: OTHER })];
  const order = moveToSiblingFront([18, 7, 9], sessions, 9);

  const tree = buildSidebarTree({ sessions, order });
  assert.equal(projectKeyOf(tree, 9), MAIN, '器を越えない');
  const main = tree.find(project => project.key === MAIN);
  assert.deepEqual(main?.children.map(node => node.id), [9, 7]);
});

test('先頭へ: 子セッションは同じ親の子の先頭へ動き、親から離れない', () => {
  const sessions = [
    ses(12),
    ses(13, { parent_session_id: 12 }),
    ses(14, { parent_session_id: 12 }),
    ses(7),
  ];
  const order = moveToSiblingFront([12, 13, 14, 7], sessions, 14);

  const tree = buildSidebarTree({ sessions, order });
  assert.equal(nodeOf(tree, 14)?.depth, 1, '子のまま');
  assert.deepEqual(tree[0].children[0].children.map(node => node.id), [14, 13]);
  assert.deepEqual(flattenSidebarTree(tree), [12, 14, 13, 7], '親より前には出ない');
});

test('先頭へ: すでに先頭なら並びは変わらない', () => {
  const sessions = [ses(7), ses(9)];
  assert.deepEqual(moveToSiblingFront([7, 9], sessions, 7), [7, 9]);
});

test('先頭へ: 元の配列を破壊しない', () => {
  const sessions = [ses(7), ses(9)];
  const original = [7, 9];
  moveToSiblingFront(original, sessions, 9);
  assert.deepEqual(original, [7, 9]);
});
