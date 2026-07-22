import { autoExpand, doSend, inputEl, updateInputClearButton } from '../app.js';
import { activeSessionId, sessions } from './state.js';
import { showToast } from './util.js';
import { insertPromptTemplate, promptTemplateLabel, templatesForProvider } from './prompt-templates.js';

const HISTORY_DB = 'many-ai-cli-mobile-nudges';
const HISTORY_STORE = 'recent';
const HISTORY_MAX = 20;
const STORAGE_INSTANT_SEND = 'ai_cli_hub_mobile_nudge_instant_send';
// Keep the implementation for a future re-enable, but do not initialize the
// mobile voice/template controls while they are intentionally hidden.
const MOBILE_SHORT_NUDGE_ENABLED = false;
type NudgeRecord = { text: string; usedAt: number; count: number };

function isMobile(): boolean { return window.matchMedia('(max-width: 720px)').matches; }
function openHistoryDb(): Promise<IDBDatabase | null> {
  return new Promise((resolve) => {
    if (!('indexedDB' in window)) return resolve(null);
    const request = indexedDB.open(HISTORY_DB, 1);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(HISTORY_STORE)) db.createObjectStore(HISTORY_STORE, { keyPath: 'text' });
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => resolve(null);
  });
}
async function recentNudges(): Promise<NudgeRecord[]> {
  const db = await openHistoryDb();
  if (!db) return [];
  return new Promise((resolve) => {
    const request = db.transaction(HISTORY_STORE, 'readonly').objectStore(HISTORY_STORE).getAll();
    request.onsuccess = () => { db.close(); resolve((request.result as NudgeRecord[]).sort((a, b) => b.count - a.count || b.usedAt - a.usedAt).slice(0, HISTORY_MAX)); };
    request.onerror = () => { db.close(); resolve([]); };
  });
}
async function rememberNudge(rawText: string): Promise<void> {
  const text = rawText.trim(); if (!text) return;
  const db = await openHistoryDb(); if (!db) return;
  const tx = db.transaction(HISTORY_STORE, 'readwrite'); const store = tx.objectStore(HISTORY_STORE);
  const current = await new Promise<NudgeRecord | undefined>((resolve) => { const request = store.get(text); request.onsuccess = () => resolve(request.result as NudgeRecord | undefined); request.onerror = () => resolve(undefined); });
  store.put({ text, usedAt: Date.now(), count: (current?.count || 0) + 1 });
  tx.oncomplete = tx.onerror = () => db.close();
}
function appendToInput(text: string): void {
  inputEl.value += inputEl.value && !/\s$/.test(inputEl.value) ? ` ${text}` : text;
  autoExpand(); updateInputClearButton(); inputEl.focus();
}
async function insertOrSend(text: string, send: boolean): Promise<void> {
  if (activeSessionId === null) { showToast('セッションを選択してから送信してください'); return; }
  appendToInput(text); await rememberNudge(text); void renderHistory();
  if (send) await doSend(activeSessionId);
}
function renderChips(): void {
  const container = document.getElementById('mobile-nudge-chips'); if (!container) return;
  container.replaceChildren();
  const provider = activeSessionId === null ? '' : sessions.get(activeSessionId)?.provider;
  for (const template of templatesForProvider(provider)) {
    const chip = document.createElement('button'); chip.type = 'button'; chip.className = 'mobile-nudge-chip'; chip.textContent = promptTemplateLabel(template.body); chip.title = template.body;
    chip.addEventListener('click', () => { insertPromptTemplate(template); void rememberNudge(template.body); void renderHistory(); }); container.append(chip);
  }
}
async function renderHistory(): Promise<void> {
  const container = document.getElementById('mobile-nudge-history'); if (!container) return;
  const records = await recentNudges(); container.replaceChildren(); container.hidden = records.length === 0;
  for (const record of records) {
    const item = document.createElement('button'); item.type = 'button'; item.className = 'mobile-nudge-history-item'; item.textContent = record.text;
    let timer: ReturnType<typeof setTimeout> | null = null; let sent = false;
    item.addEventListener('pointerdown', () => { sent = false; timer = setTimeout(() => { sent = true; void insertOrSend(record.text, true); }, 550); });
    item.addEventListener('pointerup', () => { if (timer) clearTimeout(timer); timer = null; if (!sent) void insertOrSend(record.text, false); });
    item.addEventListener('pointercancel', () => { if (timer) clearTimeout(timer); timer = null; }); container.append(item);
  }
}
function initHoldToTalk(): void {
  const hold = document.getElementById('mobile-nudge-voice-btn') as HTMLButtonElement | null; const voice = document.getElementById('voice-btn') as HTMLButtonElement | null;
  const confirm = document.getElementById('voice-confirm-btn') as HTMLButtonElement | null;
  if (!hold || !voice || !confirm) return;
  let holding = false; let textBefore = ''; let voiceSessionId: number | null = null;
  hold.addEventListener('pointerdown', (event) => {
    if (!isMobile() || voice.hidden || activeSessionId === null) return;
    event.preventDefault(); holding = true; voiceSessionId = activeSessionId; textBefore = inputEl.value; hold.setPointerCapture?.(event.pointerId); hold.classList.add('recording'); voice.click();
  });
  const finish = () => { if (!holding) return; holding = false; hold.classList.remove('recording'); confirm.click(); };
  hold.addEventListener('pointerup', finish); hold.addEventListener('pointercancel', finish);
  document.addEventListener('voiceinput:stopped', () => {
    if (voiceSessionId === null) return;
    const startedSessionId = voiceSessionId;
    voiceSessionId = null;
    if (!isMobile() || inputEl.value.trim() === textBefore.trim()) return;
    if (startedSessionId !== activeSessionId) {
      inputEl.value = textBefore; autoExpand(); updateInputClearButton();
      showToast('セッション切替中の音声入力を取り消しました');
      return;
    }
    const added = inputEl.value.slice(textBefore.length).trim(); if (!added) return;
    void rememberNudge(added).then(renderHistory);
    if (localStorage.getItem(STORAGE_INSTANT_SEND) === '1' && activeSessionId !== null) void doSend(activeSessionId);
  });
}
export function initMobileShortNudge(): void {
  if (!MOBILE_SHORT_NUDGE_ENABLED) return;
  const instant = document.getElementById('mobile-nudge-instant-send') as HTMLInputElement | null;
  if (instant) { instant.checked = localStorage.getItem(STORAGE_INSTANT_SEND) === '1'; instant.addEventListener('change', () => localStorage.setItem(STORAGE_INSTANT_SEND, instant.checked ? '1' : '0')); }
  renderChips(); void renderHistory(); initHoldToTalk(); window.addEventListener('resize', renderChips); document.addEventListener('session:activated', renderChips); window.addEventListener('prompt-templates:changed', renderChips);
}
