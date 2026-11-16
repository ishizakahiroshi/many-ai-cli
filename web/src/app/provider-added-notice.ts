// 「AI CLI が足された」ことの知らせ（取り消し付き）。
//
// AI が `many-ai-cli provider add` で AI CLI を足したとき、Hub は providers_updated
// （reason: created）を全画面へ送る。ここでは「〇〇 を足しました（実行するコマンド: …）」と
// 出し、取り消すを押すと既存の登録 API で消す。足すたびに本人の承認を待つ形にはしない
// （本人の手順を増やさないため。2026-11-16 本人決定: 知らせて取り消せる形）。
//
// 自分のこの画面で足した場合（設定の「AI CLI を追加」）は、本人が押した操作なので知らせない。
// Hub の通知は保存の応答より先に届くことがあるため、保存を送る前に印を付ける。
import { t } from '../i18n.js';
import {
  idsToAnnounce,
  launchCommandLine,
  undoFailureMessageKey,
} from './provider-added-notice-model.js';
import { PROVIDERS_UPDATED_EVENT, type ProvidersUpdatedDetail } from './providers-updated.js';
import { loadProviderDetail, removeProvider } from './provider-store.js';
import { showToast } from './util.js';

export const NOTICE_DURATION_MS = 15_000;

export function addedNoticeText(name: string, command: string): string {
  return t('provider_added_notice', { name, command: command || '-' });
}

let undoButton: HTMLButtonElement | null = null;
let undoTimer: ReturnType<typeof setTimeout> | null = null;

function clearUndo(): void {
  if (undoTimer) clearTimeout(undoTimer);
  undoTimer = null;
  undoButton?.remove();
  undoButton = null;
}

async function undo(id: string, name: string, noticeRevision: string): Promise<void> {
  clearUndo();
  // 知らせた時点の登録の版を添えて消す。取り直した版を使うと、知らせたあとに同じ id が
  // 足し直されていても、その新しい登録を消してしまう。版が変わっていれば Hub が 409 で断る。
  const removed = await removeProvider(id, noticeRevision);
  if (removed.ok) {
    showToast(t('provider_added_undone', { name }), undefined, 2500);
    return;
  }
  showToast(t(undoFailureMessageKey(removed), { name }), undefined, 4000);
}

function announce(id: string, name: string, command: string, noticeRevision: string): void {
  clearUndo();
  showToast(addedNoticeText(name, command), undefined, NOTICE_DURATION_MS);
  // #toast は pointer-events:none。取り消すだけをクリックできるようにする既存の形
  // （approval-queue-tab.ts の Undo と同じクラス）を使う。
  const toast = document.getElementById('toast');
  if (!toast) return;
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'ui-button ui-button--secondary ui-button--regular aqt-undo-toast-btn';
  button.textContent = t('provider_added_undo');
  button.addEventListener('click', () => { void undo(id, name, noticeRevision); });
  toast.appendChild(button);
  undoButton = button;
  undoTimer = setTimeout(clearUndo, NOTICE_DURATION_MS);
}

async function onProvidersUpdated(detail: ProvidersUpdatedDetail): Promise<void> {
  for (const id of idsToAnnounce(detail)) {
    const result = await loadProviderDetail(id);
    if (!result.ok) continue;
    const name = result.provider.display_name || id;
    announce(id, name, launchCommandLine(result.provider.launch), result.revision);
  }
}

let started = false;
export function initProviderAddedNotice(): void {
  if (started || typeof document === 'undefined') return;
  started = true;
  document.addEventListener(PROVIDERS_UPDATED_EVENT, (event: Event) => {
    const detail = (event as CustomEvent<ProvidersUpdatedDetail>).detail;
    if (detail) void onProvidersUpdated(detail);
  });
}
