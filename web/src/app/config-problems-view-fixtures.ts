import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { interpolate } from '../i18n.js';
import {
  KNOWN_PROBLEM_CODES,
  buildConfigBanner,
  parseConfigProblems,
  problemKey,
  visibleText,
  type Translate,
} from './config-problems-view.js';

type Dictionary = Record<string, string>;
const load = (lang: string): Dictionary =>
  JSON.parse(readFileSync(new URL(`../i18n/${lang}.json`, import.meta.url), 'utf8'));
const dictionaries: Record<string, Dictionary> = { en: load('en'), ja: load('ja'), vi: load('vi') };

// 画面の window.t と同じ置き換え（辞書に無い key は key そのものを返す）。置き換えは画面と同じ関数を使う。
const translator = (lang: string): Translate => (key, vars) => {
  const text = String(dictionaries[lang][key] ?? key);
  return vars ? interpolate(text, vars) : text;
};

// Hub（ConfigProblem::to_json）が返す形。値はすべて合成。
const reservedId = {
  section: 'custom_providers',
  index: 2,
  id: 'shell',
  field: 'id',
  code: 'reserved-id',
  dropped: true,
  related_index: null,
  summary: 'custom_providers entry #2 (id: shell), field `id`',
  reason: 'English reason from the Hub',
  effect: 'English effect from the Hub',
  example: 'custom_providers:\n  - id: my-cli\n    command: my-cli',
  fix_command: null,
  check_command: 'many-ai-cli config check',
};
const headless = {
  ...reservedId,
  index: 3,
  id: 'muse',
  field: 'headless.format',
  code: 'headless-format',
  dropped: false,
};
const configProblems = (extra: Record<string, unknown>) => ({
  status: 'problems',
  problems: [reservedId],
  restart_needed: false,
  check_command: 'many-ai-cli config check',
  ...extra,
});

test('parseConfigProblems: 古い Hub（欄なし）や形違いは何も出さない', () => {
  assert.equal(parseConfigProblems(undefined), null);
  assert.equal(parseConfigProblems(null), null);
  assert.equal(parseConfigProblems('problems'), null);
  assert.equal(parseConfigProblems({ status: 'weird' }), null);
  assert.equal(buildConfigBanner(null, translator('ja')), null);
});

test('誤りが無ければ帯は出ない（直ったら消える）', () => {
  const ok = parseConfigProblems({ status: 'ok', problems: [], restart_needed: false, check_command: 'x' });
  assert.equal(buildConfigBanner(ok, translator('ja')), null);
  const missing = parseConfigProblems({ status: 'unavailable', problems: [], restart_needed: false });
  assert.equal(buildConfigBanner(missing, translator('ja')), null);
});

test('誤り 1 件: 何件目・id・欄・なぜ・結果・例・確かめ方が日本語で出る', () => {
  const view = buildConfigBanner(parseConfigProblems(configProblems({})), translator('ja'));
  assert.ok(view);
  assert.equal(view.kind, 'problems');
  assert.equal(view.title, '設定に誤りが 1 件あります');
  assert.equal(view.items.length, 1);
  const [item] = view.items;
  assert.equal(item.heading, 'custom_providers の 2 件目（id: shell）');
  assert.equal(item.field, 'id');
  assert.ok(item.reason.includes('`id: shell` は予約語です'), item.reason);
  assert.ok(item.effect.includes('外しました'), item.effect);
  assert.ok(item.effect.includes('spawn の選択肢'), item.effect);
  assert.ok(item.example.includes('custom_providers:'));
  assert.equal(view.checkCommand, 'many-ai-cli config check');
  assert.deepEqual(view.notes, []);
});

test('headless の誤り: 外してはいない（対話では使える）と書く', () => {
  const view = buildConfigBanner(
    parseConfigProblems(configProblems({ problems: [reservedId, headless] })),
    translator('ja'),
  );
  assert.ok(view);
  assert.equal(view.title, '設定に誤りが 2 件あります');
  assert.ok(view.items[1].effect.includes('headless'), view.items[1].effect);
  assert.ok(view.items[1].effect.includes('対話'), view.items[1].effect);
  assert.ok(view.items[1].reason.includes('headless.format'));
});

test('id が重なったときは、使われるほうの何件目かを出す', () => {
  const duplicate = { ...reservedId, index: 4, id: 'muse', code: 'duplicate-id', related_index: 1 };
  const view = buildConfigBanner(parseConfigProblems(configProblems({ problems: [duplicate] })), translator('ja'));
  assert.ok(view);
  assert.ok(view.items[0].reason.includes('1 件目と重なっています'), view.items[0].reason);
  assert.ok(view.items[0].effect.includes('1 件目のほうが使われます'), view.items[0].effect);
});

test('id が読めない項目は何件目かだけで示す', () => {
  const noId = { ...reservedId, id: null, code: 'missing-id' };
  const view = buildConfigBanner(parseConfigProblems(configProblems({ problems: [noId] })), translator('en'));
  assert.ok(view);
  assert.equal(view.items[0].heading, 'custom_providers entry #2');
});

test('画面が知らない code は Hub の英文をそのまま出す', () => {
  const future = { ...reservedId, code: 'brand-new-code', reason: 'Hub says why', effect: 'Hub says what now' };
  const view = buildConfigBanner(parseConfigProblems(configProblems({ problems: [future] })), translator('ja'));
  assert.ok(view);
  assert.equal(view.items[0].reason, 'Hub says why');
  assert.equal(view.items[0].effect, 'Hub says what now');
});

test('3 言語とも全部の code の文言があり、差し込み欄が揃っている', () => {
  const holes = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
  for (const code of KNOWN_PROBLEM_CODES) {
    const key = problemKey(code, 'reason');
    const base = holes(dictionaries.en[key] ?? '');
    for (const lang of ['en', 'ja', 'vi']) {
      assert.ok(dictionaries[lang][key], `${lang} に ${key} が無い`);
      assert.deepEqual(holes(dictionaries[lang][key]), base, `${lang} ${key} の差し込み欄`);
    }
  }
  const mine = Object.keys(dictionaries.en).filter((key) => key.startsWith('config_problem'));
  assert.ok(mine.length >= 30);
  for (const lang of ['ja', 'vi']) {
    for (const key of mine) {
      assert.ok(dictionaries[lang][key], `${lang} に ${key} が無い`);
      assert.deepEqual(holes(dictionaries[lang][key]), holes(dictionaries.en[key]), `${lang} ${key}`);
    }
    const extra = Object.keys(dictionaries[lang]).filter((key) => key.startsWith('config_problem') && !(key in dictionaries.en));
    assert.deepEqual(extra, []);
  }
});

test('ファイルでは直ったが Hub は起動時のまま: 再起動を促す帯だけ出す', () => {
  const view = buildConfigBanner(
    parseConfigProblems({ status: 'ok', problems: [], restart_needed: true, check_command: 'many-ai-cli config check' }),
    translator('ja'),
  );
  assert.ok(view);
  assert.equal(view.kind, 'fixed');
  assert.deepEqual(view.items, []);
  assert.ok(view.summary.includes('再起動'));
});

test('一部だけ直したときは、残りの誤りと再起動の注意を出す', () => {
  const view = buildConfigBanner(parseConfigProblems(configProblems({ restart_needed: true })), translator('ja'));
  assert.ok(view);
  assert.equal(view.kind, 'problems');
  assert.equal(view.notes.length, 1);
  assert.ok(view.notes[0].includes('再起動'));
});

test('読み込めなくなったとき: 位置だけを出し、行の中身は出さない', () => {
  const info = parseConfigProblems({
    status: 'unreadable',
    problems: [],
    restart_needed: false,
    check_command: 'many-ai-cli config check',
    unreadable: { line: 7, column: 3, reason: 'YAML syntax error', reset_command: 'many-ai-cli config reset --yes' },
  });
  const view = buildConfigBanner(info, translator('ja'));
  assert.ok(view);
  assert.equal(view.kind, 'unreadable');
  assert.equal(view.title, '設定ファイルを読めません（7 行目 3 列目）。');
  assert.ok(view.notes[0].includes('many-ai-cli config reset --yes'));
  const vi = buildConfigBanner(info, translator('vi'));
  assert.ok(vi);
  assert.ok(vi.title.includes('dòng 7, cột 3'), vi.title);
  const noPosition = buildConfigBanner(
    parseConfigProblems({ status: 'unreadable', unreadable: { reset_command: 'r' } }),
    translator('en'),
  );
  assert.ok(noPosition);
  assert.equal(noPosition.title, 'The settings file cannot be read.');
});

test('読み込めなくなったとき: 新しいセッションは起動できない・動いているものはそのまま、と書く', () => {
  const info = parseConfigProblems({
    status: 'unreadable',
    problems: [],
    restart_needed: false,
    check_command: 'many-ai-cli config check',
    unreadable: { line: 7, column: 3, reason: 'YAML syntax error', reset_command: 'many-ai-cli config reset --yes' },
  });
  const ja = buildConfigBanner(info, translator('ja'));
  assert.ok(ja);
  assert.ok(ja.summary.includes('新しいセッションは起動できません'), ja.summary);
  assert.ok(ja.summary.includes('今動いているセッションはそのまま'), ja.summary);
  assert.ok(!ja.summary.includes('起動したときに読んだ設定のまま'), ja.summary);
  const en = buildConfigBanner(info, translator('en'));
  assert.ok(en);
  assert.ok(en.summary.includes('no new session can be started'), en.summary);
  assert.ok(!en.summary.includes('keeps running with the settings'), en.summary);
  const vi = buildConfigBanner(info, translator('vi'));
  assert.ok(vi);
  assert.ok(vi.summary.includes('không thể khởi động phiên mới'), vi.summary);
});

test('置き換えの値に $ の特殊記号が入っていても、そのまま出る', () => {
  assert.equal(interpolate('id: {id}!', { id: "a$'b$&c$`d$$" }), "id: a$'b$&c$`d$$!");
  const dollar = { ...reservedId, id: "x$'y" };
  const view = buildConfigBanner(parseConfigProblems(configProblems({ problems: [dollar] })), translator('ja'));
  assert.ok(view);
  assert.equal(view.items[0].heading, "custom_providers の 2 件目（id: x$'y）");
  assert.ok(view.items[0].reason.includes("`id: x$'y` は予約語です"), view.items[0].reason);
});

test('id の見えない書式制御文字（向きを変える文字など）は画面に出さない', () => {
  const rlo = String.fromCodePoint(0x202e);
  const zeroWidth = String.fromCodePoint(0x200b);
  const isolate = String.fromCodePoint(0x2066);
  const bom = String.fromCodePoint(0xfeff);
  const bell = String.fromCodePoint(7);
  assert.equal(visibleText('abc' + rlo + 'gpj.exe'), 'abcgpj.exe');
  assert.equal(visibleText('a' + zeroWidth + 'b' + isolate + 'c' + bom + 'd' + bell + 'e'), 'abcde');
  const sneaky = { ...reservedId, id: 'abc' + rlo + 'gpj.exe' };
  const view = buildConfigBanner(parseConfigProblems(configProblems({ problems: [sneaky] })), translator('en'));
  assert.ok(view);
  assert.ok(!view.items[0].heading.includes(rlo), view.items[0].heading);
  assert.ok(!view.items[0].reason.includes(rlo), view.items[0].reason);
});

test('先頭だけ読まれて 2 つ目以降の文書は読まれない: ファイル全体の項目として出る', () => {
  const extra = {
    section: 'config',
    index: null,
    id: null,
    field: 'documents',
    code: 'extra-documents',
    dropped: false,
    related_index: null,
    reason: 'English reason from the Hub',
    effect: 'English effect from the Hub',
    example: 'token: x',
    fix_command: null,
  };
  const view = buildConfigBanner(parseConfigProblems(configProblems({ problems: [extra] })), translator('ja'));
  assert.ok(view);
  assert.equal(view.items[0].heading, 'config.yaml（ファイル全体）');
  assert.ok(view.items[0].reason.includes('YAML の文書が 2 つ以上'), view.items[0].reason);
  assert.ok(view.items[0].effect.includes('読まれず'), view.items[0].effect);
});

test('同じ id の別の 1 件が使われているときは、どの 1 件が使われるかを出す', () => {
  const replaced = { ...reservedId, index: 1, id: 'twin', code: 'command-unterminated-quote', field: 'command', related_index: 2 };
  const view = buildConfigBanner(parseConfigProblems(configProblems({ problems: [replaced] })), translator('ja'));
  assert.ok(view);
  assert.ok(view.items[0].effect.includes('2 件目'), view.items[0].effect);
  assert.ok(view.items[0].effect.includes('使われます'), view.items[0].effect);
  const alone = { ...replaced, related_index: null };
  const view2 = buildConfigBanner(parseConfigProblems(configProblems({ problems: [alone] })), translator('ja'));
  assert.ok(view2);
  assert.ok(view2.items[0].effect.includes('外しました'), view2.items[0].effect);
});

test('Hub の保存が外した項目: バックアップの名前と戻し方を出し、「直りました」とは言わない', () => {
  const removed = {
    kind: 'removed-by-save',
    section: 'custom_providers',
    index: 3,
    id: 'half',
    field: 'command',
    code: 'missing-command',
    backup_id: '20261016_101500',
    removed_at: '2026-10-16 10:15:00',
    reason: 'English reason from the Hub',
    restore_command: 'many-ai-cli config backup restore 20261016_101500',
    dismiss_command: 'many-ai-cli config check --dismiss-removed',
  };
  // ファイルはもう直っている（誤りは無い）が、外した知らせが残っている間は「問題あり」のまま。
  const info = parseConfigProblems({
    status: 'problems',
    problems: [],
    removed_by_save: [removed],
    restart_needed: true,
    check_command: 'many-ai-cli config check',
  });
  assert.ok(info);
  assert.equal(info.removed.length, 1);
  const view = buildConfigBanner(info, translator('ja'));
  assert.ok(view);
  assert.equal(view.kind, 'problems');
  assert.equal(view.title, '設定に誤りが 1 件あります');
  assert.equal(view.items[0].heading, 'custom_providers の 3 件目（id: half）');
  assert.ok(view.items[0].effect.includes('20261016_101500'), view.items[0].effect);
  assert.equal(view.items[0].fixCommand, 'many-ai-cli config backup restore 20261016_101500');
  assert.ok(view.notes.some((note) => note.includes('--dismiss-removed')), view.notes.join('|'));
  const en = buildConfigBanner(info, translator('en'));
  assert.ok(en);
  assert.ok(en.items[0].effect.includes('backup 20261016_101500'), en.items[0].effect);
  // 知らせが無く、ファイルが直っていて再起動待ちのときだけ「直りました」の帯になる。
  const fixed = parseConfigProblems({ status: 'ok', problems: [], removed_by_save: [], restart_needed: true });
  assert.equal(buildConfigBanner(fixed, translator('ja'))?.kind, 'fixed');
});

test('知らせがある間は「再起動で反映」「ファイルで直った」と言わない（restart_needed が立っていても）', () => {
  const removed = {
    kind: 'removed-by-save',
    section: 'custom_providers',
    index: 2,
    id: 'half',
    field: 'command',
    code: 'missing-command',
    backup_id: '20261016_101500',
    restore_command: 'many-ai-cli config backup restore 20261016_101500',
    dismiss_command: 'many-ai-cli config check --dismiss-removed',
  };
  for (const status of ['problems', 'ok'] as const) {
    const info = parseConfigProblems({
      status,
      problems: [],
      removed_by_save: [removed],
      restart_needed: true,
      check_command: 'many-ai-cli config check',
    });
    const view = buildConfigBanner(info, translator('ja'));
    if (status === 'ok') {
      // Hub は知らせがあれば problems を返す。万一 ok で来ても「直った」の帯にはしない。
      assert.equal(view, null);
      continue;
    }
    assert.ok(view);
    assert.equal(view.kind, 'problems');
    const restartNote = translator('ja')('config_problems_restart_note');
    assert.ok(!view.notes.includes(restartNote), view.notes.join('|'));
    assert.ok(!JSON.stringify(view).includes('直っています'), JSON.stringify(view));
  }
  // 知らせが無ければ、これまでどおり再起動を促す。
  const none = parseConfigProblems({ status: 'problems', problems: [{ section: 'custom_providers', index: 1, id: 'x', field: 'command', code: 'missing-command', dropped: true }], removed_by_save: [], restart_needed: true });
  const view = buildConfigBanner(none, translator('ja'));
  assert.ok(view?.notes.includes(translator('ja')('config_problems_restart_note')));
});

test('「ほか N 件」と「知らせの記録が読めない」も帯に出る（3 言語・件数に数える）', () => {
  const more = { kind: 'removed-more', backup_id: '20261016_101500', more: 7950, restore_command: 'many-ai-cli config backup restore 20261016_101500', dismiss_command: 'many-ai-cli config check --dismiss-removed' };
  const broken = { kind: 'record-unreadable', record_file: 'config.yaml.bak_20261016_101500.removed.json', reason: 'it is damaged or is not a record this program wrote', restore_command: null, dismiss_command: 'many-ai-cli config check --dismiss-removed' };
  const info = parseConfigProblems({ status: 'problems', problems: [], removed_by_save: [more, broken], restart_needed: false, check_command: 'many-ai-cli config check' });
  assert.ok(info);
  assert.deepEqual(info.removed.map((item) => [item.kind, item.more]), [['removed-more', 7950], ['record-unreadable', 0]]);
  for (const lang of ['en', 'ja', 'vi']) {
    const view = buildConfigBanner(info, translator(lang));
    assert.ok(view, lang);
    assert.equal(view.items.length, 2, lang);
    assert.ok(view.items[0].heading.includes('7950'), `${lang}: ${view.items[0].heading}`);
    assert.ok(view.items[1].heading.includes('config.yaml.bak_20261016_101500.removed.json'), `${lang}: ${view.items[1].heading}`);
    assert.equal(view.items[1].fixCommand, 'many-ai-cli config check --dismiss-removed', lang);
    // 辞書にある文言が使われ、キー名がそのまま出ていない。
    assert.ok(!JSON.stringify(view).includes('config_problem_'), `${lang}: ${JSON.stringify(view)}`);
  }
});

test('3 言語で帯の見出しが出る', () => {
  const info = parseConfigProblems(configProblems({}));
  const titles = ['en', 'ja', 'vi'].map((lang) => buildConfigBanner(info, translator(lang))?.title);
  assert.deepEqual(titles, ['1 mistake(s) in the settings', '設定に誤りが 1 件あります', 'Cài đặt có 1 lỗi']);
});
