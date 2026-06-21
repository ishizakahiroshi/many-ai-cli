// mobile-home.ts
// スマホホーム画面（#mobile-home）の描画モジュール。
// PC には一切副作用を与えない。全エントリーポイントは isMobileViewport() で early return する。

import { t } from '../i18n.js';
import { orderSessions, sessions, approvalVisibleCache, multiQuestionVisibleCache, approvalRawOptionsCache, activeSessionId } from './state.js';
import { activateSession, providerIconHtml, stateLabel, safeClassToken } from './session-list.js';
import { filterFirstMessage } from './settings.js';
import { isBatchOptions, isMultiSelectOptions } from './approval-parser.js';
import { sessionTitle, approvalQuestionContext, renderOptionButtons, pendingSessionIds } from './approval-queue-tab.js';

// スマホ幅判定の単一情報源（このモジュール内のみ使用）
const mobileMql = (typeof window !== 'undefined' && typeof window.matchMedia === 'function')
  ? window.matchMedia('(max-width: 720px)')
  : null;
function isMobileViewport(): boolean { return !!mobileMql?.matches; }

// セッションのバケット分類
type SessionBucket = 'pending' | 'running' | 'waiting' | 'error';
function getSessionBucket(id: number): SessionBucket {
  const s = sessions.get(id);
  if (approvalVisibleCache.get(id) || multiQuestionVisibleCache.get(id)) return 'pending';
  const state = s?.state || 'standby';
  if (state === 'error' || state === 'disconnected') return 'error';
  if (state === 'running') return 'running';
  return 'waiting';
}

// 1 枚のカードを作成して返す
function buildCard(id: number): HTMLElement {
  const s = sessions.get(id);
  const bucket = getSessionBucket(id);
  const options = approvalRawOptionsCache.get(id);
  const isPending = bucket === 'pending';
  const isMultiQ = !!multiQuestionVisibleCache.get(id);
  const isBatch = Array.isArray(options) && isBatchOptions(options);
  const isMultiSel = Array.isArray(options) && isMultiSelectOptions?.(options);

  const card = document.createElement('div') as HTMLElement;
  card.className = `mh-card mh-card--${bucket}`;
  card.dataset.sessionId = String(id);

  // ── ヘッダー ──
  const header = document.createElement('div');
  header.className = 'mh-card-header';

  const iconHtml = s ? providerIconHtml(s.provider, 20) : '';
  const titleEl = document.createElement('div');
  titleEl.className = 'mh-card-title';
  titleEl.innerHTML = `${iconHtml}<span class="mh-session-id">#${id}</span> <span class="mh-session-name">${escapeHtml(sessionTitle(s))}</span>`;

  const stateBadge = document.createElement('span');
  stateBadge.className = `mh-state-badge mh-state-badge--${safeClassToken(s?.state || 'standby')}`;
  stateBadge.textContent = stateLabel(s?.state || 'standby');

  header.appendChild(titleEl);
  header.appendChild(stateBadge);
  card.appendChild(header);

  // ── 最新ひと言（last_message / first_message） ──
  const snippet = filterFirstMessage(s?.last_message || s?.first_message || '');
  if (snippet) {
    const snippetEl = document.createElement('div');
    snippetEl.className = 'mh-card-snippet';
    snippetEl.textContent = snippet;
    card.appendChild(snippetEl);
  }

  // ── 承認待ちカード: 質問テキスト + 選択肢（or フォールバック） ──
  if (isPending) {
    // batch / multiQ は inline 描画せず「承認タブで開く」フォールバック
    if (isMultiQ || isBatch || isMultiSel) {
      const fallback = document.createElement('div');
      fallback.className = 'mh-approval-fallback';
      fallback.textContent = t('approval_tab_multiq_hint');
      card.appendChild(fallback);
    } else if (Array.isArray(options) && options.length > 0) {
      // 質問テキスト
      const { preamble, question } = approvalQuestionContext(options);
      if (preamble) {
        const preEl = document.createElement('div');
        preEl.className = 'mh-card-preamble';
        preEl.textContent = preamble;
        card.appendChild(preEl);
      }
      if (question) {
        const qEl = document.createElement('div');
        qEl.className = 'mh-card-question';
        qEl.textContent = question;
        card.appendChild(qEl);
      }

      // 選択肢ボタン（既存 renderOptionButtons を流用）
      const optContainer = document.createElement('div');
      optContainer.className = 'mh-options';
      renderOptionButtons(optContainer, id, options);
      card.appendChild(optContainer);
    }
  }

  // カード全体クリック → 個別セッション画面へ遷移（ボタン領域は stopPropagation しない）
  card.addEventListener('click', (e) => {
    // ボタン自体のクリックは sendChoice 経路任せ（伝播を止めない）
    if ((e.target as HTMLElement).closest('button')) return;
    activateSession(id);
  });

  return card;
}

// セクションヘッダー生成
function buildSectionHeader(labelKey: string, count: number): HTMLElement {
  const h = document.createElement('h3');
  h.className = 'mh-section-header';
  h.textContent = `${t(labelKey)}（${count} 件）`;
  return h;
}

// ホーム全体を再描画
export function renderMobileHome() {
  if (!isMobileViewport()) return;
  const container = document.getElementById('mobile-home');
  if (!container) return;

  container.innerHTML = '';

  const allSessions = orderSessions();
  if (allSessions.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'mh-empty';
    empty.setAttribute('data-i18n', 'no_sessions');
    empty.textContent = t('no_sessions');
    container.appendChild(empty);
    return;
  }

  // バケット別に分類
  const buckets: Record<SessionBucket, number[]> = { pending: [], running: [], waiting: [], error: [] };
  for (const s of allSessions) {
    if (!s) continue;
    buckets[getSessionBucket(s.id)].push(s.id);
  }

  // pending セクション（上部固定）
  const pinnedSection = document.createElement('section');
  pinnedSection.id = 'mobile-home-pinned';
  pinnedSection.className = 'mh-section mh-section--pinned';
  if (buckets.pending.length === 0) {
    pinnedSection.hidden = true;
  } else {
    pinnedSection.appendChild(buildSectionHeader('mobile_home_section_pending', buckets.pending.length));
    for (const id of buckets.pending) {
      pinnedSection.appendChild(buildCard(id));
    }
  }
  container.appendChild(pinnedSection);

  // その他セクション
  const restBuckets: { key: SessionBucket; labelKey: string }[] = [
    { key: 'running', labelKey: 'mobile_home_section_running' },
    { key: 'waiting', labelKey: 'mobile_home_section_waiting' },
    { key: 'error', labelKey: 'mobile_home_section_error' },
  ];
  for (const { key, labelKey } of restBuckets) {
    const ids = buckets[key];
    if (ids.length === 0) continue;
    const section = document.createElement('section');
    section.className = `mh-section mh-section--${key}`;
    section.appendChild(buildSectionHeader(labelKey, ids.length));
    for (const id of ids) {
      section.appendChild(buildCard(id));
    }
    container.appendChild(section);
  }
}

// 単一カードの差分更新（全件再描画を避けてスムーズにする）
export function updateMobileHomeCard(id: number) {
  if (!isMobileViewport()) return;
  const container = document.getElementById('mobile-home');
  if (!container) return;

  const existing = container.querySelector<HTMLElement>(`.mh-card[data-session-id="${id}"]`);
  const newCard = sessions.has(id) ? buildCard(id) : null;
  const newBucket = sessions.has(id) ? getSessionBucket(id) : null;

  if (!existing) {
    // カードが存在しない → 全件再描画（セッション追加 or バケット変化）
    renderMobileHome();
    return;
  }
  if (!newCard) {
    // セッション削除 → 全件再描画
    renderMobileHome();
    return;
  }

  // 同じバケット内での更新 → カードを差し替え
  const existingBucket = Array.from(existing.classList)
    .find(c => c.startsWith('mh-card--'))?.replace('mh-card--', '') as SessionBucket | undefined;
  if (existingBucket !== newBucket) {
    // バケットが変わった → 全件再描画
    renderMobileHome();
    return;
  }

  existing.replaceWith(newCard);
}

// HTML エスケープ（XSS 防止）
function escapeHtml(str: string): string {
  return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

// approval-queue-updated イベントで再描画
window.addEventListener('approval-queue-updated', () => {
  if (!isMobileViewport()) return;
  renderMobileHome();
});

// window 経由で app.ts / session-list.ts から呼べるように登録
window.renderMobileHome = renderMobileHome;
window.updateMobileHomeCard = updateMobileHomeCard;
