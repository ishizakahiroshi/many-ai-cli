import { afterEach, beforeEach, describe, test } from 'node:test';
import assert from 'node:assert/strict';
import {
  fillOrchestrationProviderSelect,
  getOrchestrationCLIOptions,
  onOrchestrationProvidersChanged,
  ORCHESTRATION_CLI_OPTIONS,
} from './orchestration-roles.js';
import {
  childPermissionPreviewFor,
  effortLevelsFor,
  setLaunchOptionChoices,
} from './spawn-confirm-store.js';

// Only the DOM operations used by the shared renderer are needed. In particular,
// HTML interpolation is forbidden so labels and ids cannot become markup.
class SelectFixture {
  options: Array<{ value: string; textContent: string }> = [];
  private selected = '';
  ownerDocument = {
    createElement(tag: string) {
      assert.equal(tag, 'option');
      return { value: '', textContent: '' };
    },
  };
  get value(): string { return this.selected; }
  set value(value: string) {
    this.selected = this.options.some((option) => option.value === value) ? value : '';
  }
  set innerHTML(_value: string) { throw new Error('Provider data must not be inserted as HTML'); }
  replaceChildren(...options: Array<{ value: string; textContent: string }>): void {
    this.options = options;
    this.selected = options[0]?.value || '';
  }
  element(): HTMLSelectElement { return this as unknown as HTMLSelectElement; }
}
const translate = (key: string): string => `translated:${key}`;
const cleanups: Array<() => void> = [];
describe('orchestration custom provider choices', () => {
  beforeEach(() => { setLaunchOptionChoices({ custom_providers: [] }); });
  afterEach(() => {
    for (const cleanup of cleanups.splice(0)) cleanup();
    setLaunchOptionChoices({ custom_providers: [] });
  });

  test('uses the same /api/info custom providers for confirmation and both role selectors', () => {
    setLaunchOptionChoices({ custom_providers: [{ id: 'muse', label: 'Muse Code' }] });
    assert.deepEqual(getOrchestrationCLIOptions(), [
      ...ORCHESTRATION_CLI_OPTIONS,
      { value: 'muse', label: 'Muse Code' },
    ]);
    for (const includeUnset of [false, true, true]) {
      const select = new SelectFixture();
      fillOrchestrationProviderSelect(select.element(), translate, 'muse', includeUnset);
      assert.equal(select.value, 'muse');
      assert.equal(select.options.find((option) => option.value === 'muse')?.textContent, 'Muse Code');
      assert.equal(select.options.some((option) => option.value === ''), includeUnset);
    }
  });

  test('excludes shell, duplicate built-ins, duplicate custom ids and malformed entries', () => {
    setLaunchOptionChoices({ custom_providers: [
      null, {}, { id: 17 }, { id: ' ' },
      { id: 'shell', label: 'Shell' }, { id: ' codex ', label: 'Wrong label' },
      { id: ' muse ', label: 'Muse Code' }, { id: 'muse', label: 'Duplicate' },
      { id: 'custom-no-label' }, { id: 'custom-blank-label', label: '  ' },
    ] });
    const options = getOrchestrationCLIOptions();
    assert.equal(options.some((option) => option.value === 'shell'), false);
    assert.equal(options.filter((option) => option.value === 'codex').length, 1);
    assert.equal(options.find((option) => option.value === 'codex')?.label, 'Codex CLI');
    assert.deepEqual(options.slice(ORCHESTRATION_CLI_OPTIONS.length), [
      { value: 'muse', label: 'Muse Code' },
      { value: 'custom-no-label', label: 'custom-no-label' },
      { value: 'custom-blank-label', label: 'custom-blank-label' },
    ]);
  });

  test('missing or malformed legacy fields preserve choices; an explicit empty list removes customs', () => {
    setLaunchOptionChoices({ custom_providers: [{ id: 'muse' }] });
    setLaunchOptionChoices({});
    setLaunchOptionChoices({ custom_providers: 'invalid' });
    assert.equal(getOrchestrationCLIOptions().at(-1)?.value, 'muse');
    setLaunchOptionChoices({ custom_providers: [] });
    assert.deepEqual(getOrchestrationCLIOptions(), ORCHESTRATION_CLI_OPTIONS);
  });

  test('late API responses refresh open selectors and preserve built-in and custom selections', () => {
    const selectors = [false, true, true].map((includeUnset) => {
      const select = new SelectFixture();
      fillOrchestrationProviderSelect(select.element(), translate, 'codex', includeUnset);
      cleanups.push(onOrchestrationProvidersChanged(() => {
        fillOrchestrationProviderSelect(select.element(), translate, select.value, includeUnset);
      }));
      return select;
    });
    setLaunchOptionChoices({ custom_providers: [{ id: 'muse', label: 'Muse Code' }] });
    for (const select of selectors) {
      assert.equal(select.value, 'codex');
      assert.equal(select.options.at(-1)?.value, 'muse');
      select.value = 'muse';
    }
    setLaunchOptionChoices({ custom_providers: [
      { id: 'muse', label: 'Updated Muse' }, { id: 'another-cli', label: 'Another CLI' },
    ] });
    for (const select of selectors) {
      assert.equal(select.value, 'muse');
      assert.equal(select.options.find((option) => option.value === 'muse')?.textContent, 'Updated Muse');
    }
    setLaunchOptionChoices({ custom_providers: [] });
    assert.deepEqual(selectors.map((select) => select.value), ['claude', '', '']);
    assert.equal(selectors.some((select) => select.options.some((option) => option.value === 'muse')), false);
  });

  test('late provider notifications see the accompanying effort and permission data', () => {
    let notified = false;
    cleanups.push(onOrchestrationProvidersChanged(() => {
      notified = true;
      assert.deepEqual(effortLevelsFor('muse'), ['low']);
      const preview = childPermissionPreviewFor('muse', 'full');
      assert.equal(preview?.permissionMode, '');
      assert.equal(preview?.sandbox, '');
      assert.equal(preview?.riskConfirmed, false);
    }));
    setLaunchOptionChoices({
      custom_providers: [{ id: 'muse' }],
      effort_levels: { muse: ['low'] },
      child_permission_preview: { muse: { full: { tier: 'full', risk_confirmed: false } } },
    });
    assert.equal(notified, true);
  });

  test('provider values and labels are set as literal DOM data', () => {
    const value = 'custom"><img src=x onerror=alert(1)>';
    const label = '<script>alert("provider")</script> & label';
    setLaunchOptionChoices({ custom_providers: [{ id: value, label }] });
    const select = new SelectFixture();
    fillOrchestrationProviderSelect(select.element(), translate, value);
    assert.equal(select.value, value);
    assert.deepEqual(select.options.at(-1), { value, textContent: label });
    assert.equal(select.options[0].textContent, 'translated:spawn_role_cli_none');
  });

  test('closing a selector unsubscribes it from later API responses', () => {
    let updates = 0;
    const unsubscribe = onOrchestrationProvidersChanged(() => { updates++; });
    cleanups.push(unsubscribe);
    setLaunchOptionChoices({ custom_providers: [{ id: 'muse' }] });
    assert.equal(updates, 1);
    unsubscribe();
    setLaunchOptionChoices({ custom_providers: [] });
    assert.equal(updates, 1);
  });
});
