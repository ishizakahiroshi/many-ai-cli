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
