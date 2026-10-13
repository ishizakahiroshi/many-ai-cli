import { test, expect, afterEach } from 'bun:test';
import { readFileSync } from 'node:fs';
import { confirmationLabels, type ConfirmationAction } from '../src/app/confirmation-labels.js';
import { uiButtonClass, uiModalClass, uiIconHtml } from '../src/app/ui-components.js';

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
function locale(lang: 'ja' | 'en' | 'vi') {
  const dict = JSON.parse(readFileSync(new URL(`../src/i18n/${lang}.json`, import.meta.url), 'utf8'));
  Object.defineProperty(globalThis, 'window', { configurable: true, value: {
    t: (key: string, vars?: Record<string, unknown>) => {
      let text = String(dict[key] ?? key);
      for (const [name, value] of Object.entries(vars || {})) text = text.replaceAll(`{${name}}`, String(value));
      return text;
    },
  } });
}
afterEach(() => {
  if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
  else Reflect.deleteProperty(globalThis, 'window');
});
test('confirmation templates share the explicit action across the three labels', () => {
  locale('ja');
  expect(confirmationLabels('update')).toEqual({ question: '更新しますか？', decline: '更新しない', accept: '更新する' });
  expect(confirmationLabels('delete').decline).toBe('削除しない');
  expect(confirmationLabels('save').accept).toBe('保存する');
  expect(confirmationLabels('stop').question).toBe('停止しますか？');
});
test('non-suru verbs use grammatical forms; shutdown choices remain distinct', () => {
  locale('ja');
  expect(confirmationLabels('close')).toEqual({ question: '閉じますか？', decline: '閉じない', accept: '閉じる' });
  expect(confirmationLabels('continue').accept).toBe('続ける');
  expect(confirmationLabels('stopAll').accept).toBe('すべて停止する');
  expect(confirmationLabels('stopHub').accept).toBe('Hubのみ停止する');
});
test('all shared actions resolve in every shipped locale at call time', () => {
  const actions: ConfirmationAction[] = ['update','delete','save','stop','stopAll','stopHub','reset','revoke','disconnect','close','continue','restore','launch','push','pull','disable','clear','remove','approve','retrieve','add','send','move','discard','overwrite','import','export','install','edit','copy'];
  for (const lang of ['ja', 'en', 'vi'] as const) {
    locale(lang);
    for (const action of actions) {
      for (const label of Object.values(confirmationLabels(action))) {
        expect(label).not.toMatch(/^ui_(?:confirm|action)_/);
        expect(label).not.toMatch(/\{action\}/);
        expect(label.length).toBeGreaterThan(0);
      }
    }
  }
  locale('ja'); expect(confirmationLabels('update').accept).toBe('更新する');
  locale('en'); expect(confirmationLabels('update').accept).not.toBe('更新する');
});
test('shared class supplier separates operation role, density and icon shape', () => {
  expect(uiButtonClass('danger', 'compact', { icon: true })).toContain('ui-button--danger');
  expect(uiButtonClass('danger', 'compact', { icon: true })).toContain('ui-button--compact');
  expect(uiButtonClass('danger', 'compact', { icon: true })).toContain('ui-button--icon');
  expect(uiModalClass('lg')).toBe('ui-modal ui-modal--lg');
});
test('general SVGs inherit text color and remain decorative', () => {
  const icon = uiIconHtml('copy');
  expect(icon).toContain('stroke="currentColor"');
  expect(icon).toContain('aria-hidden="true"');
  expect(icon).toContain('focusable="false"');
  expect(icon).not.toContain('onclick');
});
