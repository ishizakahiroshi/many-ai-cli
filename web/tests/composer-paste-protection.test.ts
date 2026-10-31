import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const app = readFileSync(new URL('../src/app.ts', import.meta.url), 'utf8');
const attachments = readFileSync(new URL('../src/app/attachments.ts', import.meta.url), 'utf8');
function part(source: string, start: string, end: string) {
  const from = source.indexOf(start), to = source.indexOf(end, from);
  if (from < 0 || to < 0) throw new Error('source boundary missing');
  return source.slice(from, to).replace(/^export /gm, '');
}
function compile(source: string) {
  return new Bun.Transpiler({ loader: 'ts' }).transformSync(source);
}

describe('composer input protection', () => {
  test('four long pastes all remain available for file conversion', () => {
    const state: any = {
      pastedTexts: [], pasteCounter: 0, inputEl: { focus() {} },
      noteComposerDraftChanged() {}, renderPasteChips() {}, updateInputAffordance() {},
      TextEncoder, PASTE_FILE_MIN_LINES: 10, PASTE_FILE_MIN_CHARS: 1500, MAX_ATTACH_BYTES: 1000000,
    };
    state.set_pasteCounter = (n: number) => state.pasteCounter = n;
    const source = part(app, 'export function stagePastedText', 'export function expandPasteChip')
      + part(app, 'function prepareComposerSend', '// 長文ペースト（10行以上')
      + '\nglobalThis.stage = stagePastedText; globalThis.prepare = prepareComposerSend;';
    runInNewContext(compile(source), state);
    for (let i = 1; i <= 4; i++) state.stage(Array(10).fill('item' + i).join('\n'));
    const result = state.prepare({ pendingAttachFiles: [], pastedTextsData: state.pastedTexts, inputValue: '' });
    expect(result.attachments.map((a: any) => a.filename)).toEqual(['paste-1.txt', 'paste-2.txt', 'paste-3.txt', 'paste-4.txt']);
    expect(state.stage('short')).toBe(false);
  });

  test('a failed upload returns no injects and preserves thumbnails', async () => {
    let count = 0, removed = 0, history = 0;
    const state: any = {
      pendingAttachFiles: [], remoteAttachTarget: () => '', remoteAttachIdentity: () => '', token: '', FormData, Blob, URL,
      apiFetch: async () => ++count === 1 ? { ok: true, json: async () => ({ inject: '@first' }) } : { ok: false, status: 500 },
      showToast() {}, t: (x: string) => x, pushMessage() { history++; },
    };
    runInNewContext(compile(part(attachments, 'export async function flushPendingAttach', 'export function addAttachThumbnail')) + '\nthis.flush = flushPendingAttach;', state);
    const files = ['a.txt', 'b.txt'].map(filename => ({ filename, buf: new Uint8Array([1]).buffer, wrapper: { remove() { removed++; } } }));
    expect(await state.flush(1, files)).toBeNull();
    expect(removed).toBe(0);
    expect(history).toBe(0);
    expect(files.length).toBe(2);
  });
});
