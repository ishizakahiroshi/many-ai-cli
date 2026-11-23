import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { FILES_PANE_CLASS, shouldOpenFilesBeside, type FilesOpenSurface } from '../src/app/files-open-mode';

// 端末を単独で見ている、よくある状態を土台にして、1 項目ずつ変える。
const terminalOnly: FilesOpenSurface = {
  hasSession: true,
  compact: false,
  detached: false,
  displayVisible: true,
  displayMode: 'terminal',
  multiVisible: false,
  multiPaneMode: null,
};

describe('Files タブを端末の隣へ開くか', () => {
  test('端末だけを見ているときは隣へ開く', () => {
    expect(shouldOpenFilesBeside(terminalOnly)).toBe(true);
  });

  test('このセッションの分割ペインを見ているときも隣へ足す', () => {
    expect(shouldOpenFilesBeside({
      ...terminalOnly, displayVisible: false, displayMode: null, multiVisible: true, multiPaneMode: 'workspace',
    })).toBe(true);
  });

  test('全セッション一覧（マルチ）では従来どおりタブを切り替える', () => {
    expect(shouldOpenFilesBeside({
      ...terminalOnly, displayVisible: false, displayMode: null, multiVisible: true, multiPaneMode: 'overview',
    })).toBe(false);
  });

  test('チャット・Git・Files・レビューなど端末以外を見ているときは切り替える（3 枚に増やさない）', () => {
    for (const mode of ['chat', 'git', 'files', 'review', 'approval', 'history', 'orchestration']) {
      expect(shouldOpenFilesBeside({ ...terminalOnly, displayMode: mode })).toBe(false);
    }
  });

  test('狭い画面・別窓の固定タブ・セッションなしでは並べない', () => {
    expect(shouldOpenFilesBeside({ ...terminalOnly, compact: true })).toBe(false);
    expect(shouldOpenFilesBeside({ ...terminalOnly, detached: true })).toBe(false);
    expect(shouldOpenFilesBeside({ ...terminalOnly, hasSession: false })).toBe(false);
  });

  test('どの画面も見えていないとき（読み込み途中）は何もしない', () => {
    expect(shouldOpenFilesBeside({
      ...terminalOnly, displayVisible: false, displayMode: null, multiVisible: false, multiPaneMode: null,
    })).toBe(false);
  });

  test('display-area が見えていれば、multi-view の残り値には引きずられない', () => {
    expect(shouldOpenFilesBeside({ ...terminalOnly, displayMode: 'chat', multiPaneMode: 'workspace' })).toBe(false);
  });
});

describe('分割ペインの Files の見た目', () => {
  test('ペインの印のクラス名が files.css に残っている（名前を変えて CSS だけ取り残さない）', () => {
    const css = readFileSync(join(import.meta.dir, '../src/styles/files.css'), 'utf8');
    expect(css).toContain(`.${FILES_PANE_CLASS} .files-tab-preview-pane`);
    expect(css).toContain(`.${FILES_PANE_CLASS} .files-tab-tree-pane`);
  });
});

// ── タブの段と起動フォームが横にはみ出さないことの確認（bugfix_pending-batch-small-fixes_2026-11-21 の C3・C4）──
// Files タブと同じタブの段の見た目なので、ここに置く。実画面の幅はテストでは測れないので、
// はみ出しを止めている CSS の約束を文字で確かめる（幅の実測は静的な確認で取った）。

const stylesDir = join(import.meta.dir, '../src/styles');
const tabsCss = readFileSync(join(stylesDir, 'tabs.css'), 'utf8');
const spawnCss = readFileSync(join(stylesDir, 'spawn.css'), 'utf8');
const indexHtml = readFileSync(join(import.meta.dir, '../src/index.html'), 'utf8');

/** `prelude {` で始まる最初のブロックの中身（波かっこの対応で切り出す）。 */
function blockAfter(css: string, prelude: string): string {
  const start = css.indexOf(prelude);
  if (start < 0) return '';
  const open = css.indexOf('{', start + prelude.length);
  let depth = 0;
  for (let i = open; i < css.length; i++) {
    if (css[i] === '{') depth++;
    else if (css[i] === '}') {
      depth--;
      if (depth === 0) return css.slice(open + 1, i);
    }
  }
  return '';
}

/** `@container unified-tab-bar (<feature>: <n>px)` の一覧。 */
function containerStages(css: string): Array<{ feature: string; px: number; body: string }> {
  const out: Array<{ feature: string; px: number; body: string }> = [];
  for (const m of css.matchAll(/@container unified-tab-bar \((min|max)-width: (\d+)px\)/g)) {
    out.push({ feature: m[1], px: Number(m[2]), body: blockAfter(css.slice(m.index), m[0]) });
  }
  return out;
}

/** PC 幅（721px 以上）の @media の中身。container とチップを縮める規則はここにある。 */
// 同じ幅の「and (pointer: fine)」付きの @media が先にあるので、条件がこれだけの方を選ぶ。
const pcBlock = blockAfter(tabsCss.slice(tabsCss.indexOf('@media (min-width: 721px) {')), '@media (min-width: 721px)');

/** pcBlock の中で、selector を含む規則の中身を全部つなげたもの。 */
function rulesFor(selectorPart: string): string {
  return [...pcBlock.matchAll(/([^{}]+)\{([^}]*)\}/g)]
    .filter((m) => m[1].includes(selectorPart))
    .map((m) => m[2])
    .join('\n');
}

describe('タブの段は幅が足りないとき段階的に詰める（C3）', () => {
  test('段自身の幅で判定する container を、PC 幅だけで持つ', () => {
    expect(pcBlock).toContain('#unified-tab-bar { container: unified-tab-bar / inline-size; }');
    // スマホ幅の並び（styles.css）には container を足さない。
    expect(tabsCss.match(/container: unified-tab-bar/g)?.length).toBe(1);
  });

  test('チップはタブより先に縮むが、切り取らない（細くなれるのは子の最小幅の合計まで）', () => {
    expect(pcBlock).toContain('.session-info-chip { flex: 0 1 auto; }');
    expect(pcBlock).toContain('.session-info-chip > * { flex-shrink: 0; }');
    // チップに min-width: 0 や overflow: hidden を付けると、子の最小幅の合計が段に入らないとき
    // 末尾の契約が横スクロールもできずに切り取られる（英語の長い権限表示で実測）。
    // 注記（「overflow: hidden も付けない」など）に当たらないよう、コメントを外してから読む。
    const cssWithoutComments = tabsCss.replace(/\/\*[\s\S]*?\*\//g, '');
    const chipRules = [...cssWithoutComments.matchAll(/(^|[\s,}])\.session-info-chip\s*\{([^}]*)\}/g)].map((m) => m[2]).join('\n');
    expect(chipRules).not.toMatch(/min-width:\s*0/);
    expect(chipRules).not.toMatch(/overflow(-x)?:\s*(hidden|clip)/);
    // 起動時の権限は縮める対象に入れない。
    expect(pcBlock).not.toContain('session-permission-chip');
  });

  test('縮める項目は最小幅（min-content）を 4 文字ぶんまで下げられる形にする', () => {
    // nowrap のままだと項目の最小幅が全文の幅になり、チップが全文より細くなれない。
    // 1 行に折り返させて 2 行目以降を隠すと、最小幅は min-width の 4 文字ぶんになる。
    for (const sel of ['.card-provider-chip', '.tsb-agent-name', '.card-subscription', '.card-model-text']) {
      const body = rulesFor(sel);
      expect({ sel, clamp: /-webkit-line-clamp: 1;/.test(body) }).toEqual({ sel, clamp: true });
      expect({ sel, wrap: /white-space: normal;/.test(body) && /word-break: break-all;/.test(body) }).toEqual({ sel, wrap: true });
      expect({ sel, nowrap: /white-space: nowrap/.test(body) }).toEqual({ sel, nowrap: false });
      expect({ sel, min: /min-width: (calc\(4ch \+ 12px\)|4ch);/.test(body) }).toEqual({ sel, min: true });
    }
    // styles.css の .card-model.card-model--with-icon（min-width: 0・tabs.css より後）に負けない強さで書く。
    expect(pcBlock).toContain('.session-info-chip .card-model.card-model--with-icon { min-width: calc(13px + 4ch); }');
    expect(pcBlock).toContain('.session-info-chip .card-model.card-model--with-icon .card-model-text');
  });

  test('チップの末尾は 起動時の権限 → 契約 → ブランチ の並び（コメントと実物を揃える）', () => {
    const settingsSrc = readFileSync(join(import.meta.dir, '../src/app/settings.ts'), 'utf8');
    expect(settingsSrc).toContain('${agentNameBadge}${permissionBadge}${subBadge}${branchBadge}`;');
    expect(tabsCss).toContain('「… → エージェント名 → 起動時の権限 → 契約 → ブランチ」で、末尾はブランチ');
  });

  test('段階の閾値は 絵を外す → 項目を外す → 絵だけ の順に狭くなる', () => {
    const stages = containerStages(tabsCss);
    expect(stages.filter((s) => s.feature === 'min')).toEqual([]);
    const max = stages.filter((s) => s.feature === 'max').map((s) => s.px);
    expect(max).toEqual([...max].sort((a, b) => b - a));
    expect(max.length).toBe(3);
    const [icons, items, iconOnly] = stages.filter((s) => s.feature === 'max');
    expect(icons.body).toContain('#unified-tab-bar .view-tab-icon { display: none; }');
    expect(items.body).toContain('.session-mode-chip:not(.headless)');
    expect(iconOnly.body).toContain('.view-tab:not([data-tab="terminal"]):not([data-tab="chat"])');
    // 名前は読み上げ用に残す（display: none にしない）。
    expect(iconOnly.body).toContain('clip-path: inset(50%);');
    expect(iconOnly.body).not.toMatch(/> span[^{]*\{[^}]*display: none/);
  });

  test('絵だけにするタブは、外す文字が 1 つだけで、件数の印には id が付いていて残る', () => {
    const tabs = [...indexHtml.matchAll(/<button class="[^"]*\bview-tab\b[^"]*" data-tab="([a-z]+)"[^>]*>([\s\S]*?)<\/button>/g)];
    expect(tabs.length).toBeGreaterThanOrEqual(9);
    for (const [, name, inner] of tabs) {
      if (name === 'terminal' || name === 'chat' || name === 'orchestration') continue;
      const spans = [...inner.matchAll(/<span\b([^>]*)>/g)].map((m) => m[1]);
      const labels = spans.filter((attrs) => !/class="view-tab-icon"/.test(attrs) && !/\bid="/.test(attrs));
      expect({ name, labels: labels.length }).toEqual({ name, labels: 1 });
      expect(inner).toMatch(/class="view-tab-icon"/);
    }
  });

  test('絵だけになるタブは、マウスを乗せると名前が出る', () => {
    const i18nDir = join(import.meta.dir, '../src/i18n');
    const dicts = ['ja', 'en', 'vi'].map((l) => JSON.parse(readFileSync(join(i18nDir, `${l}.json`), 'utf8')) as Record<string, string>);
    const tabs = [...indexHtml.matchAll(/<button (class="[^"]*\bview-tab\b[^"]*" data-tab="([a-z]+)"[^>]*)>/g)];
    // Files・Git・Review は settings.ts の refreshLazyTabClasses が名前から始まるツールチップを付け直す。
    const lazyTabs = new Set(['files', 'git', 'review']);
    for (const [, attrs, name] of tabs) {
      if (name === 'terminal' || name === 'chat' || lazyTabs.has(name)) continue;
      const key = attrs.match(/data-i18n-tooltip="([^"]+)"/)?.[1] ?? '';
      expect({ name, key: key !== '' }).toEqual({ name, key: true });
      for (const d of dicts) expect({ name, key, has: typeof d[key] === 'string' && d[key] !== '' }).toEqual({ name, key, has: true });
    }
    expect(indexHtml).toContain('id="tab-multi" type="button" data-i18n-tooltip="tab_multi"');
    expect(indexHtml).toContain('id="approval-tab-btn" data-i18n-tooltip="tab_approval"');
    expect(indexHtml).toContain('id="history-tab-btn" data-i18n-tooltip="detached_tab_title_history"');
    const settingsSrc = readFileSync(join(import.meta.dir, '../src/app/settings.ts'), 'utf8');
    const lazyFn = blockAfter(settingsSrc, 'export function refreshLazyTabClasses(sid)');
    expect(lazyFn).toContain("for (const tab of ['files', 'git', 'review'])");
    expect(lazyFn).toContain('const name = t(`tab_${tab}`);');
    expect(lazyFn).toContain('btn.dataset.tooltip = loaded ? name : `${name} · ${t(`tab_${tab}_lazy_tooltip`)}`;');
  });
});

describe('起動フォームの欄はパネルの幅に収まる（C4）', () => {
  test('select は器の幅を上限にする（一番長い選択肢の幅まで広がらない）', () => {
    expect(spawnCss).toMatch(/^\.spawn-select \{ max-width: 100%; \}$/m);
  });

  test('モデル欄の入力は縮められ、右端のボタンを押し出さない', () => {
    expect(spawnCss).toMatch(/^\.spawn-model-row > \.spawn-input \{ flex: 1; min-width: 0; \}$/m);
  });

  test('承認モード欄は flex 行ではなくブロックの器に置かれている（max-width が効く前提）', () => {
    expect(indexHtml).toMatch(/<div id="spawn-permission-opts">\s*<select id="spawn-permission-mode" class="ui-input spawn-select">/);
    expect(spawnCss).not.toMatch(/#spawn-permission-opts\s*\{[^}]*display:\s*flex/);
  });
});
