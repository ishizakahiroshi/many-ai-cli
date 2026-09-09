// spawn-panel-fixtures.ts — spawn-panel.ts の純関数を node:test で固定する。
//
// ここが赤くなったら、直すのはテストではなく実装。CWD ドロップダウンの並び規則を壊す
// 変更が入っている。
//
// 由来: docs/local/plan_spawn-cwd-subdir-sort.md

import assert from 'node:assert/strict';
import test from 'node:test';
import {
  compareCwdByBasename,
  filterCwdSubdirItems,
  joinCwdChild,
  sortCwdSubdirItems,
  splitCwdPath,
  splitCwdTypeahead,
} from './cwd-path.js';

// 合成データ。実在のパスは書かない（公開ファイルの層 1 防御）。
const PARENT = 'C:\\dev\\public';
const SUBDIRS = [
  'PlainSheet',
  'ai-audit-prompts',
  'ShotTTL',
  'ai-delegation-and-review',
];

test('splitCwdPath: パスを親と basename に分割する', () => {
  // parent は区切り文字を含む（ラベル描画で親プレフィクスとして使う既存仕様）。
  assert.deepEqual(splitCwdPath('C:\\repos\\proj'), { parent: 'C:\\repos\\', basename: 'proj' });
  assert.deepEqual(splitCwdPath('/repos/proj'), { parent: '/repos/', basename: 'proj' });
  // 末尾の区切り文字は basename から落とす
  assert.deepEqual(splitCwdPath('C:\\repos\\proj' + String.fromCharCode(92)), { parent: 'C:\\repos\\', basename: 'proj' });
  assert.deepEqual(splitCwdPath('/repos/proj/'), { parent: '/repos/', basename: 'proj' });
  assert.deepEqual(splitCwdPath('proj'), { parent: '', basename: 'proj' });
  assert.deepEqual(splitCwdPath('C:\\repos/proj' + String.fromCharCode(92)), { parent: 'C:\\repos/', basename: 'proj' });
});

test('compareCwdByBasename: basename 昇順、同名はフルパスで安定化', () => {
  const a = PARENT + '\\PlainSheet';
  const b = PARENT + '\\ai-audit-prompts';
  // 符号だけ見る。localeCompare の戻り値は -1/0/1 とは限らない。
  assert.equal(
    Math.sign(compareCwdByBasename(a, b)),
    Math.sign('PlainSheet'.localeCompare('ai-audit-prompts')),
  );

  // 同じ basename ならフルパスで比較
  const p1 = 'C:\\repos\\proj';
  const p2 = 'D:\\repos\\proj';
  assert.equal(compareCwdByBasename(p1, p2), p1.localeCompare(p2));
});

// ---- 不変条件 1: favSet が空なら結果は compareCwdByBasename と一致する ----

test('sortCwdSubdirItems: 空 favSet では compareCwdByBasename 順に一致する', () => {
  const paths = SUBDIRS.map(n => PARENT + '\\' + n);
  const sorted = sortCwdSubdirItems(paths, new Set());
  const expected = paths.slice().sort(compareCwdByBasename);
  assert.deepEqual(sorted, expected);
});

// ---- 不変条件 2: favSet があるときはお気に入りが非お気に入りより前に来る ----

test('sortCwdSubdirItems: お気に入りが先頭グループ、非お気に入りが後ろグループ', () => {
  const paths = SUBDIRS.map(n => PARENT + '\\' + n);
  const favSet = new Set([PARENT + '\\ShotTTL', PARENT + '\\ai-audit-prompts']);
  const sorted = sortCwdSubdirItems(paths, favSet);

  // お気に入り 2 件が先頭、非お気に入り 2 件が後ろ
  const favCount = sorted.filter(p => favSet.has(p)).length;
  assert.equal(favCount, 2);
  // 先頭 2 件がお気に入り
  assert.ok(favSet.has(sorted[0]));
  assert.ok(favSet.has(sorted[1]));
  // 後ろ 2 件が非お気に入り
  assert.ok(!favSet.has(sorted[2]));
  assert.ok(!favSet.has(sorted[3]));

  // お気に入りグループ内も basename 昇順
  const favGroup = sorted.slice(0, 2);
  const favExpected = favGroup.slice().sort(compareCwdByBasename);
  assert.deepEqual(favGroup, favExpected);

  // 非お気に入りグループ内も basename 昇順
  const nonFavGroup = sorted.slice(2);
  const nonFavExpected = nonFavGroup.slice().sort(compareCwdByBasename);
  assert.deepEqual(nonFavGroup, nonFavExpected);
});

// ---- 不変条件 3: 同グループ内は compareCwdByBasename と一致する ----

test('sortCwdSubdirItems: 各グループ内の並びは compareCwdByBasename と一致する', () => {
  // 3 つのお気に入りを設定してもう少し複雑なケース
  const paths = [
    'C:\\dev\\public\\Zebra',
    'C:\\dev\\public\\apple',
    'C:\\dev\\public\\Banana',
    'C:\\dev\\public\\cherry',
    'C:\\dev\\public\\Date',
    'C:\\dev\\public\\elderberry',
  ];
  const favSet = new Set([
    'C:\\dev\\public\\Banana',
    'C:\\dev\\public\\Zebra',
    'C:\\dev\\public\\elderberry',
  ]);
  const sorted = sortCwdSubdirItems(paths, favSet);

  const favItems = sorted.filter(p => favSet.has(p));
  assert.deepEqual(favItems, favItems.slice().sort(compareCwdByBasename));

  const nonFavItems = sorted.filter(p => !favSet.has(p));
  assert.deepEqual(nonFavItems, nonFavItems.slice().sort(compareCwdByBasename));

  // 全体としてお気に入りが先頭
  assert.equal(favItems.length + nonFavItems.length, sorted.length);
});

test('sortCwdSubdirItems: 元の配列を破壊しない', () => {
  const paths = SUBDIRS.map(n => PARENT + '\\' + n);
  const original = paths.slice();
  const favSet = new Set([paths[0]]);
  sortCwdSubdirItems(paths, favSet);
  assert.deepEqual(paths, original, '入力配列は変更されない');
});

test('sortCwdSubdirItems: 空配列を安全に処理する', () => {
  assert.deepEqual(sortCwdSubdirItems([], new Set()), []);
});

// ---- splitCwdTypeahead ----
// 由来: docs/local/plan_spawn-cwd-subdir-typeahead.md

test('splitCwdTypeahead: 入力値を親・区切り・打ちかけセグメントに分割する', () => {
  // 末尾が区切りでない: 最後の区切りの位置で parent と partial に分ける
  assert.deepEqual(splitCwdTypeahead('C:\\dev\\public\\o'), { parent: 'C:\\dev\\public', sep: '\\', partial: 'o' });
  assert.deepEqual(splitCwdTypeahead('/srv/work/y'), { parent: '/srv/work', sep: '/', partial: 'y' });

  // 末尾が区切り: partial は空文字、parent は末尾の区切りを落とした値
  assert.deepEqual(splitCwdTypeahead('C:\\dev\\public\\'), { parent: 'C:\\dev\\public', sep: '\\', partial: '' });
  // 区切りが 2 つ連続して終わる場合も全部落とす
  assert.deepEqual(splitCwdTypeahead('C:\\dev\\public\\\\'), { parent: 'C:\\dev\\public', sep: '\\', partial: '' });

  // ドライブ直下の特別扱い: 落とした結果が "D:" 形式になったら区切りを 1 つ残す
  assert.deepEqual(splitCwdTypeahead('C:\\o'), { parent: 'C:\\', sep: '\\', partial: 'o' });
  assert.deepEqual(splitCwdTypeahead('C:\\'), { parent: 'C:\\', sep: '\\', partial: '' });

  // POSIX ルートの特別扱い: 落とした結果が空文字になったら "/" にする
  assert.deepEqual(splitCwdTypeahead('/y'), { parent: '/', sep: '/', partial: 'y' });
  assert.deepEqual(splitCwdTypeahead('/'), { parent: '/', sep: '/', partial: '' });

  // 区切り文字を 1 つも含まない値は null
  assert.equal(splitCwdTypeahead('proj'), null);
});

// ---- filterCwdSubdirItems ----
// 由来: docs/local/plan_spawn-cwd-subdir-typeahead.md

const TYPEAHEAD_PARENT = 'C:\\dev\\public';
const TYPEAHEAD_SUBDIRS = ['orbit', 'omega', 'motor', 'zephyr'].map(n => TYPEAHEAD_PARENT + '\\' + n);

test('filterCwdSubdirItems: partial が空文字なら sortCwdSubdirItems と一致する（不変条件）', () => {
  const favSet = new Set([TYPEAHEAD_PARENT + '\\zephyr']);
  assert.deepEqual(
    filterCwdSubdirItems(TYPEAHEAD_SUBDIRS, favSet, ''),
    sortCwdSubdirItems(TYPEAHEAD_SUBDIRS, favSet),
  );
});

test('filterCwdSubdirItems: 前方一致が部分一致より前に来る', () => {
  // "orbit" は "or" に前方一致、"motor" は "or" を途中に含むだけ
  const paths = [TYPEAHEAD_PARENT + '\\motor', TYPEAHEAD_PARENT + '\\orbit'];
  const result = filterCwdSubdirItems(paths, new Set(), 'or');
  assert.deepEqual(result, [TYPEAHEAD_PARENT + '\\orbit', TYPEAHEAD_PARENT + '\\motor']);
});

test('filterCwdSubdirItems: 同ランク内はお気に入りが先頭に来る', () => {
  // "omega" と "onyx" はどちらも "o" に前方一致。basename 昇順なら omega が先だが、
  // onyx をお気に入りにすると omega より前に来る
  const omega = TYPEAHEAD_PARENT + '\\omega';
  const onyx = TYPEAHEAD_PARENT + '\\onyx';
  const favSet = new Set([onyx]);
  const result = filterCwdSubdirItems([omega, onyx], favSet, 'o');
  assert.deepEqual(result, [onyx, omega]);
});

test('filterCwdSubdirItems: 同グループ内は compareCwdByBasename 順である', () => {
  // 3 件とも "o" に前方一致・お気に入りなし → basename 昇順のまま
  const paths = ['oscar', 'omega', 'onyx'].map(n => TYPEAHEAD_PARENT + '\\' + n);
  const result = filterCwdSubdirItems(paths, new Set(), 'o');
  assert.deepEqual(result, ['omega', 'onyx', 'oscar'].map(n => TYPEAHEAD_PARENT + '\\' + n));
});

test('filterCwdSubdirItems: 大文字小文字を無視して一致する', () => {
  const paths = [TYPEAHEAD_PARENT + '\\Orbit', TYPEAHEAD_PARENT + '\\Motor'];
  const result = filterCwdSubdirItems(paths, new Set(), 'OR');
  assert.deepEqual(result, [TYPEAHEAD_PARENT + '\\Orbit', TYPEAHEAD_PARENT + '\\Motor']);
});

test('filterCwdSubdirItems: 入力配列を破壊しない', () => {
  const paths = TYPEAHEAD_SUBDIRS.slice();
  const original = paths.slice();
  filterCwdSubdirItems(paths, new Set([paths[0]]), 'o');
  assert.deepEqual(paths, original, '入力配列は変更されない');
});

test('filterCwdSubdirItems: 一致 0 件で空配列を返す', () => {
  assert.deepEqual(filterCwdSubdirItems(TYPEAHEAD_SUBDIRS, new Set(), 'xyz123'), []);
});

// ---- joinCwdChild ----
// 由来: docs/local/plan_spawn-cwd-subdir-typeahead.md

test('joinCwdChild: 親が区切りで終わっていなければ区切りを 1 つ挟む', () => {
  assert.equal(joinCwdChild('C:\\dev\\public', '\\', 'orbit'), 'C:\\dev\\public\\orbit');
  assert.equal(joinCwdChild('/srv/work', '/', 'orbit'), '/srv/work/orbit');
});

test('joinCwdChild: 親が区切りで終わっていれば二重に置かない', () => {
  // splitCwdTypeahead はドライブ直下・POSIX ルートで区切りを残した parent を返す。
  // ここで素朴に連結すると C:\\orbit になり、表示もお気に入りの完全一致も崩れる。
  assert.equal(joinCwdChild('C:\\', '\\', 'orbit'), 'C:\\orbit');
  assert.equal(joinCwdChild('/', '/', 'orbit'), '/orbit');
  // 区切りが混在した入力（parent は \ 終わり、sep は /）でも二重にしない
  assert.equal(joinCwdChild('C:\\', '/', 'orbit'), 'C:\\orbit');
});
