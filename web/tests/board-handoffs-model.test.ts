import { describe, expect, test } from 'bun:test';
import { handedWorkFrom, handoffPrompt } from '../src/app/board-handoffs-model';

describe('work handed over from the board', () => {
  const row = { project: 'demo', item_id: 'backlog/20260102-0a0b0c0d', request_id: 'REQ-20260102-11111111', title: ' Check the price ', requested_at: 1700000000000, folder: 'C:\\work\\demo' };

  test('keeps the rows the Hub listed', () => {
    expect(handedWorkFrom({ ok: true, work: [row] })).toEqual([row]);
  });
  test('a reply of another shape lists nothing', () => {
    for (const payload of [null, 'x', {}, { work: 'x' }, { work: [null, 1, 'x'] }]) expect(handedWorkFrom(payload)).toEqual([]);
  });
  test('drops rows that cannot be started or sent back', () => {
    const list = [{ ...row, item_id: '' }, { ...row, request_id: 7 }, { ...row, folder: undefined }, { ...row, requested_at: 'soon', title: 9 }];
    expect(handedWorkFrom({ work: list })).toEqual([{ ...row, requested_at: null, title: '' }]);
  });
  test('the first instruction is the title the owner wrote', () => {
    expect(handoffPrompt(handedWorkFrom({ work: [row] })[0])).toBe('Check the price');
  });
});