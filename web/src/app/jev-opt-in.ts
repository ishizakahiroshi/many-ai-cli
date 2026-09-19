import { t } from '../i18n.js';
import { apiFetch } from './util.js';

type JevResult = {
  task_type: string;
  task_type_confidence: number;
  complexity: number;
  complexity_confidence: number;
  needs_strong_model_probability: number;
  input_tokens: number;
  model: string;
};

const prefKey = 'many-ai-cli-jev-manual-opt-in';

export function initJevOptIn(): void {
  const optIn = document.getElementById('jev-opt-in') as HTMLInputElement | null;
  const input = document.getElementById('jev-text') as HTMLTextAreaElement | null;
  const button = document.getElementById('jev-evaluate') as HTMLButtonElement | null;
  const output = document.getElementById('jev-result');
  if (!optIn || !input || !button || !output) return;
  let busy = false;
  optIn.checked = localStorage.getItem(prefKey) === '1';
  const refresh = () => { button.disabled = busy || !optIn.checked || !input.value.trim(); };
  optIn.addEventListener('change', () => {
    localStorage.setItem(prefKey, optIn.checked ? '1' : '0');
    if (!optIn.checked) output.textContent = '';
    refresh();
  });
  input.addEventListener('input', refresh);
  button.addEventListener('click', async () => {
    const text = input.value.trim();
    if (busy || !optIn.checked || !text || new TextEncoder().encode(text).length > 4096) {
      if (text && new TextEncoder().encode(text).length > 4096) output.textContent = t('settings_jev_too_long');
      return;
    }
    busy = true;
    refresh();
    output.textContent = t('settings_jev_pending');
    try {
      const response = await apiFetch('/api/jev/evaluate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ text }),
      });
      if (!response.ok) {
        if (response.status === 503) throw new Error(t('settings_jev_missing_key'));
        throw new Error(t('settings_jev_failed'));
      }
      const result = await response.json() as JevResult;
      if (!Number.isFinite(result.complexity) || !Number.isFinite(result.needs_strong_model_probability)) throw new Error(t('settings_jev_failed'));
      output.textContent = `${t('settings_jev_task')}: ${result.task_type} · ${t('settings_jev_complexity')}: ${result.complexity.toFixed(1)}/4 · ${t('settings_jev_strong')}: ${(result.needs_strong_model_probability * 100).toFixed(0)}% · ${t('settings_jev_tokens')}: ${result.input_tokens}`;
    } catch (error) {
      output.textContent = error instanceof Error ? error.message : t('settings_jev_failed');
    } finally {
      busy = false;
      refresh();
    }
  });
  refresh();
}
