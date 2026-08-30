// sidebar-tree.ts — サイドバーのカードがどこに出るかを決める唯一の場所。
//
// 規則は 1 文だけ。
//
//   サイドバーは 1 本の木を描く。木の形はデータだけが決める。ユーザー操作は木の形を
//   変えず、同じ親を持つ兄弟の順序と、畳む／畳まないだけを変える。
//
// なぜ切り出したか: 以前この計算は renderSessionList の中で DOM を組み立てながら
// 行われていて、テストが書けなかった。書けないので規則はコメントでしか守られず、
// 「配置を決める仕組み」が 4 つ（sessionOrder / groupOrder / projectFavorites /
// pinned）まで増え、そのうち pinned は並び順ではなく所属そのものを書き換えていた。
// 木の形を関数に閉じ込め、不変条件を sidebar-tree-fixtures.ts で固定する。
//
// 固定している不変条件は 3 つ。
//   1. 子はルート祖先と同じプロジェクトのノードに必ず入る
//   2. 色フィルタ以外の理由でノードが器を越えない
//   3. 線形の並び（multi-pane のスロット順・スワイプ順）は、木を深さ優先で
//      たどる 1 経路からだけ作る
//
// DOM に触らない純関数だけを置く（node:test から検証するため）。描画は
// session-list.ts が担当する。
//
// 由来: docs/local/plan_sidebar-placement-tree_c2_tree-fn.md

import type { SessionSnapshot } from '../types/proto.js';

/** git 管理外など、プロジェクトを特定できないセッションが入る器。 */
export const NO_PROJECT_KEY = '__no_project__';

/** 親をたどる上限。データが壊れて循環していても止まらないようにするための保険。 */
const MAX_ANCESTOR_HOPS = 64;

/** 木に描く最大の段。0 = プロジェクト直下、1 = セッションの下。孫以降は 1 へ潰す。 */
const MAX_SESSION_DEPTH = 1;

export interface SidebarSessionNode {
  id: number;
  /** 0 = プロジェクト直下、1 = 親セッションの下。2 以上は作らない。 */
  depth: number;
  children: SidebarSessionNode[];
}

export interface SidebarProjectNode {
  /** 器の identity。project_id があればそれ、無ければ cwd の末尾セグメント。 */
  key: string;
  /** 見出しに出す名前。key の末尾セグメント。 */
  label: string;
  favorite: boolean;
  children: SidebarSessionNode[];
}

export interface SidebarTreeInput {
  /** 全セッション。フィルタ前のものを渡す（親が除外されても所属を正しく解決するため）。 */
  sessions: SessionSnapshot[];
  /** sessionOrder。兄弟を並べるときの比較キーとして使う。 */
  order: number[];
  /** プロジェクトの並び順。 */
  groupOrder?: string[];
  /** ★ を付けたプロジェクト。先頭へ出す。 */
  projectFavorites?: string[];
  /** 空文字なら全件。木から外れる唯一の理由。 */
  colorFilter?: string;
}

/**
 * cwd の末尾セグメントからプロジェクトキーを導く。project_id が取れないセッション
 * （git 管理外・Hub が古い）のためのフォールバック。
 *
 * FilesTab の可視性判定（settings.ts / ws-client.ts が参照する SessionSnapshot.project）
 * も同じ規則を使う。定義をここ 1 箇所に置き、state.ts は再 export するだけにしている。
 */
export function deriveProjectKeyFromCwd(cwd: unknown): string {
  if (!cwd) return '';
  return String(cwd).replace(/\\/g, '/').split('/').filter(part => part.length > 0).pop() || '';
}

/** そのセッション自身が属する器のキー。親は見ない（呼び出し側がルート祖先で解決する）。 */
function ownProjectKey(session: SessionSnapshot): string {
  const projectID = String(session.project_id || '').trim();
  if (projectID) return projectID;
  return deriveProjectKeyFromCwd(session.cwd) || NO_PROJECT_KEY;
}

/** key の末尾セグメント。見出しに出す名前。 */
function projectLabel(key: string): string {
  if (key === NO_PROJECT_KEY) return '';
  return deriveProjectKeyFromCwd(key) || key;
}

interface Ancestry {
  /** ルート祖先。親が居ない・親が一覧に無い・循環している場合は途中で打ち切った位置。 */
  root: SessionSnapshot;
  /** 親子が循環している。この場合そのセッションは親へぶら下げず、ルートとして扱う。 */
  cyclic: boolean;
}

/**
 * ルート祖先までたどる。孤児（親が一覧に居ない子）は捨てず、その位置をルートにする。
 * 循環しているデータでも止まらない。
 */
function ancestryOf(session: SessionSnapshot, byID: Map<number, SessionSnapshot>): Ancestry {
  let current = session;
  const seen = new Set<number>([current.id]);
  for (let hop = 0; hop < MAX_ANCESTOR_HOPS; hop++) {
    const parentID = Number(current.parent_session_id || 0);
    if (!parentID) return { root: current, cyclic: false };
    if (seen.has(parentID)) return { root: current, cyclic: true };
    const parent = byID.get(parentID);
    if (!parent) return { root: current, cyclic: false };
    seen.add(parentID);
    current = parent;
  }
  return { root: current, cyclic: true };
}

/**
 * 1 セッションが入る器のキーを返す。木を組み立てずに「このカードはどの見出しの下か」
 * だけ知りたいとき（状態チップの部分更新など）に使う。buildSidebarTree と同じ規則で
 * 解決するので、両者の答えがずれることはない。
 */
export function projectKeyForSession(session: SessionSnapshot, all: Iterable<SessionSnapshot>): string {
  if (!session) return NO_PROJECT_KEY;
  const byID = new Map<number, SessionSnapshot>();
  for (const other of all) if (other) byID.set(other.id, other);
  return ownProjectKey(ancestryOf(session, byID).root);
}

/**
 * サイドバーに描く木を組み立てる。
 *
 * 所属（どのプロジェクトに入るか）はルート祖先から決まるので、worktree で動いている
 * 子セッションが親と別の cwd を持っていても親と同じ器に入る。
 */
export function buildSidebarTree(input: SidebarTreeInput): SidebarProjectNode[] {
  const all = Array.isArray(input.sessions) ? input.sessions.filter(Boolean) : [];
  const byID = new Map<number, SessionSnapshot>();
  for (const session of all) byID.set(session.id, session);

  const orderIndex = new Map<number, number>();
  const order = Array.isArray(input.order) ? input.order : [];
  order.forEach((id, index) => {
    if (!orderIndex.has(id)) orderIndex.set(id, index);
  });
  const rank = (id: number): number => {
    const index = orderIndex.get(id);
    return index === undefined ? Number.MAX_SAFE_INTEGER : index;
  };
  const bySiblingOrder = (a: SidebarSessionNode, b: SidebarSessionNode): number =>
    rank(a.id) - rank(b.id) || a.id - b.id;

  // 色フィルタだけが木から外れる理由。所属の解決は必ずフィルタ前の一覧で行うので、
  // 親がフィルタで消えても子は正しいプロジェクトに残る。
  const colorFilter = String(input.colorFilter || '');
  const visible = colorFilter ? all.filter(session => session.color === colorFilter) : all;
  const visibleIDs = new Set<number>(visible.map(session => session.id));

  // プロジェクトごとに、表示対象のセッションを親子へ組み直す。
  const projects = new Map<string, { key: string; tops: SidebarSessionNode[]; nodes: Map<number, SidebarSessionNode> }>();
  const ancestry = new Map<number, Ancestry>();
  const ancestryFor = (session: SessionSnapshot): Ancestry => {
    let info = ancestry.get(session.id);
    if (!info) {
      info = ancestryOf(session, byID);
      ancestry.set(session.id, info);
    }
    return info;
  };
  const projectKeyOf = (session: SessionSnapshot): string => ownProjectKey(ancestryFor(session).root);

  for (const session of visible) {
    const key = projectKeyOf(session);
    let bucket = projects.get(key);
    if (!bucket) {
      bucket = { key, tops: [], nodes: new Map() };
      projects.set(key, bucket);
    }
    bucket.nodes.set(session.id, { id: session.id, depth: 0, children: [] });
  }

  for (const session of visible) {
    const bucket = projects.get(projectKeyOf(session));
    if (!bucket) continue;
    const node = bucket.nodes.get(session.id);
    if (!node) continue;
    const parentID = Number(session.parent_session_id || 0);
    // 親が表示対象に無いセッション（ルート、親が色フィルタで消えたもの、壊れたデータで
    // 循環しているもの）はプロジェクト直下へ置く。捨てない。
    const linkable = parentID && visibleIDs.has(parentID) && !ancestryFor(session).cyclic;
    const parentNode = linkable ? bucket.nodes.get(parentID) : undefined;
    if (parentNode) parentNode.children.push(node);
    else bucket.tops.push(node);
  }

  // 段を確定する。プロジェクト直下が 0、その下が 1。孫以降も 1 へ潰して同じ段へ並べ、
  // 深さ上限を上げた利用者がいてもカードが消えたり無限にインデントしたりしない。
  // 潰したぶんも含めて兄弟順で並べ直すので、孫が親より前に出ることはない。
  const collapseDescendants = (node: SidebarSessionNode, into: SidebarSessionNode[]): void => {
    for (const child of node.children) {
      into.push(child);
      collapseDescendants(child, into);
    }
  };
  const layoutTops = (tops: SidebarSessionNode[]): SidebarSessionNode[] => {
    tops.sort(bySiblingOrder);
    for (const top of tops) {
      const descendants: SidebarSessionNode[] = [];
      collapseDescendants(top, descendants);
      for (const descendant of descendants) {
        descendant.depth = MAX_SESSION_DEPTH;
        descendant.children = [];
      }
      descendants.sort(bySiblingOrder);
      top.depth = 0;
      top.children = descendants;
    }
    return tops;
  };

  const favorites = Array.isArray(input.projectFavorites) ? input.projectFavorites : [];
  const groupOrder = Array.isArray(input.groupOrder) ? input.groupOrder : [];
  const favoriteIndex = new Map(favorites.map((key, index) => [key, index] as const));
  const groupIndex = new Map(groupOrder.map((key, index) => [key, index] as const));

  const result: SidebarProjectNode[] = [];
  for (const bucket of projects.values()) {
    const children = layoutTops(bucket.tops);
    result.push({
      key: bucket.key,
      label: projectLabel(bucket.key),
      favorite: favoriteIndex.has(bucket.key),
      children,
    });
  }

  // ★ が先。その中では projectFavorites の順、次に groupOrder の順。
  // どちらにも無い key は末尾（最初に現れたセッションの順）。
  const firstAppearance = new Map<string, number>();
  result.forEach(project => {
    const first = project.children.length ? rank(project.children[0].id) : Number.MAX_SAFE_INTEGER;
    firstAppearance.set(project.key, first);
  });
  result.sort((a, b) => {
    if (a.favorite !== b.favorite) return a.favorite ? -1 : 1;
    if (a.favorite && b.favorite) return (favoriteIndex.get(a.key) ?? 0) - (favoriteIndex.get(b.key) ?? 0);
    const ai = groupIndex.has(a.key) ? (groupIndex.get(a.key) as number) : -1;
    const bi = groupIndex.has(b.key) ? (groupIndex.get(b.key) as number) : -1;
    if (ai !== -1 && bi !== -1) return ai - bi;
    if (ai !== -1) return -1;
    if (bi !== -1) return 1;
    return (firstAppearance.get(a.key) ?? 0) - (firstAppearance.get(b.key) ?? 0);
  });

  return result;
}

/**
 * セッションを「自分の器の中で先頭へ」動かした sessionOrder を返す（元の配列は変えない）。
 *
 * 器を越えない。ルートセッションなら同じプロジェクトのルートの先頭へ、子セッションなら
 * 同じ親の子の先頭へ動く。★（プロジェクトを先頭へ）と同じ意味論で、以前の 📌 のように
 * 所属を書き換えて別の箱へ移すことはしない。
 *
 * 新規セッションは sessionOrder の末尾に積まれる（state.ts の addToSessionOrder）ので、
 * 先頭へ動かしたものが後から来たセッションに押し下げられることはない。
 */
export function moveToSiblingFront(order: number[], sessions: SessionSnapshot[], id: number): number[] {
  const source = Array.isArray(order) ? order : [];
  const tree = buildSidebarTree({ sessions, order: source });

  let siblings: SidebarSessionNode[] | null = null;
  for (const project of tree) {
    if (project.children.some(node => node.id === id)) {
      siblings = project.children;
      break;
    }
    for (const top of project.children) {
      if (top.children.some(node => node.id === id)) {
        siblings = top.children;
        break;
      }
    }
    if (siblings) break;
  }
  if (!siblings || siblings.length === 0) return [...source];
  const firstID = siblings[0].id;
  if (firstID === id) return [...source];

  const next = source.filter(value => value !== id);
  const anchor = next.indexOf(firstID);
  next.splice(anchor === -1 ? 0 : anchor, 0, id);
  return next;
}

/**
 * 木を深さ優先でたどってセッション ID を返す。サイドバー・multi-pane のスロット順・
 * スワイプ順は、すべてこの 1 経路から作る。
 *
 * 畳んでいるノードの子も含める。畳む／畳まないは表示の話であって、順序の話ではない。
 */
export function flattenSidebarTree(tree: SidebarProjectNode[]): number[] {
  const out: number[] = [];
  const walk = (nodes: SidebarSessionNode[]): void => {
    for (const node of nodes) {
      out.push(node.id);
      walk(node.children);
    }
  };
  for (const project of tree) walk(project.children);
  return out;
}
