import { afterEach, beforeEach, describe, test } from 'node:test';
import assert from 'node:assert/strict';
import {
  fillOrchestrationProviderSelect,
  getOrchestrationCLIOptions,
  onOrchestrationProvidersChanged,
  setOrchestrationRegistryProviders,
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
  beforeEach(() => {
    setOrchestrationRegistryProviders([]);
    setLaunchOptionChoices({ custom_providers: [] });
  });
  afterEach(() => {
    for (const cleanup of cleanups.splice(0)) cleanup();
    setOrchestrationRegistryProviders([]);
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
    assert.deepEqual(selectors.map((select) => select.value), ['muse', '', '']);
    assert.equal(selectors.slice(1).some((select) => select.options.some((option) => option.value === 'muse')), false);
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

  test('Settings registry providers survive config-only refreshes in all three selectors', () => {
    const registry = [
      { id: 'ui-cli', display_name: 'UI CLI', enabled: true, capabilities: { launch: true } },
      { id: 'muse', display_name: 'Registry Muse', enabled: true, capabilities: { launch: true } },
    ];
    setLaunchOptionChoices({ custom_providers: [{ id: 'muse', label: 'Config Muse' }] });
    setOrchestrationRegistryProviders(registry);
    const selectors = [false, true, true].map((includeUnset) => {
      const select = new SelectFixture();
      fillOrchestrationProviderSelect(select.element(), translate, 'ui-cli', includeUnset);
      cleanups.push(onOrchestrationProvidersChanged(() => {
        fillOrchestrationProviderSelect(select.element(), translate, select.value, includeUnset);
      }));
      return select;
    });
    // The derive dialog can publish another /api/info independently of the
    // spawn panel's registry request; it must not erase Settings-added choices.
    setLaunchOptionChoices({ custom_providers: [{ id: 'muse', label: 'Config Muse' }] });
    setOrchestrationRegistryProviders(undefined); // Failed registry request.
    setOrchestrationRegistryProviders(null);
    for (const select of selectors) {
      assert.equal(select.value, 'ui-cli');
      assert.equal(select.options.find((option) => option.value === 'ui-cli')?.textContent, 'UI CLI');
      assert.equal(select.options.filter((option) => option.value === 'muse').length, 1);
      assert.equal(select.options.find((option) => option.value === 'muse')?.textContent, 'Registry Muse');
    }
  });

  test('registry excludes disabled and non-launch providers, including config duplicates', () => {
    setLaunchOptionChoices({ custom_providers: [{ id: 'disabled-cli' }, { id: 'no-launch' }] });
    setOrchestrationRegistryProviders([
      null, { id: 'shell', enabled: true, capabilities: { launch: true } },
      { id: 'disabled-cli', enabled: false, capabilities: { launch: true } },
      { id: 'no-launch', enabled: true, capabilities: { launch: false } },
      { id: 'missing-capability', enabled: true },
      { id: 'ui-cli', display_name: 'UI CLI', enabled: true, capabilities: { launch: true } },
      { id: 'ui-cli', display_name: 'Duplicate', enabled: true, capabilities: { launch: true } },
      { id: 'codex', display_name: 'Codex CLI', enabled: true, capabilities: { launch: true } },
    ]);
    const values = getOrchestrationCLIOptions().map((option) => option.value);
    for (const id of ['shell', 'disabled-cli', 'no-launch', 'missing-capability']) assert.equal(values.includes(id), false);
    assert.equal(values.filter((id) => id === 'ui-cli').length, 1);
    assert.equal(values.filter((id) => id === 'codex').length, 1);
  });

  test('pending registry-only provider is never replaced by a discovery response', () => {
    const select = new SelectFixture();
    // /api/info may already have arrived when the valid pending request opens.
    setLaunchOptionChoices({ custom_providers: [] });
    fillOrchestrationProviderSelect(select.element(), translate, 'ui-cli', false);
    assert.equal(select.value, 'ui-cli');
    cleanups.push(onOrchestrationProvidersChanged(() => {
      fillOrchestrationProviderSelect(select.element(), translate, select.value, false);
    }));
    setLaunchOptionChoices({ custom_providers: [] });
    assert.equal(select.value, 'ui-cli');
    setOrchestrationRegistryProviders([
      { id: 'ui-cli', display_name: 'UI CLI', enabled: true, capabilities: { launch: true } },
    ]);
    assert.equal(select.value, 'ui-cli');
    assert.equal(select.options.find((option) => option.value === 'ui-cli')?.textContent, 'UI CLI');
    // Even deletion after the request was accepted must not approve a different
    // CLI. The Hub will reject the original id if it is no longer usable.
    setOrchestrationRegistryProviders([]);
    assert.equal(select.value, 'ui-cli');
  });

  test('registry removals clear stale role choices without changing a pending confirmation', () => {
    setOrchestrationRegistryProviders([
      { id: 'ui-cli', enabled: true, capabilities: { launch: true } },
    ]);
    const selectors = [false, true, true].map((includeUnset) => {
      const select = new SelectFixture();
      fillOrchestrationProviderSelect(select.element(), translate, 'ui-cli', includeUnset);
      cleanups.push(onOrchestrationProvidersChanged(() => {
        fillOrchestrationProviderSelect(select.element(), translate, select.value, includeUnset);
      }));
      return select;
    });
    setOrchestrationRegistryProviders([]);
    assert.deepEqual(selectors.map((select) => select.value), ['ui-cli', '', '']);
  });

});
