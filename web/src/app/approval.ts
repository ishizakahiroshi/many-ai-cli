// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { APPROVAL_PENDING_TEXT_TAIL_LIMIT, actionBarShownAt, activeSessionId, isAnsweredMarkerSig, recordAnsweredMarkerSig, approvalConsumedSig, approvalConsumedSigDeleteTimer, approvalHintConfirmTimers, approvalHintConfirmTrusted, approvalRawOptionsCache, approvalSig, approvalSourceCache, approvalSuppressUntil, approvalSwitchCandidates, approvalVisibleCache, batchActiveQ, batchFreeText, batchSelections, lastActionBarRender, maybeAutoSwitchToNextApproval, multiQuestionDismissedCache, multiQuestionLatchAt, multiQuestionVisibleCache, multiSelectFocusIdx, multiSelectSelections, sequentialChoiceCache, sequentialChoiceSig, sessions, set_actionBarFocusIdx, set_batchFocusIdx, set_multiSelectFocusIdx, terminals, utf8Decoder } from './state.js';
import { inputEl, sendSubmittedText } from '../app.js';
import { clearSuppressPtyResize, isTerminalAtBottom, refitAndStickTerminalToBottomSoon, scanBuffer, scrollTerminalToBottomSoon, suppressPtyResizeForInputLayout } from './terminal.js';
import { stripAnsi } from './settings.js';
import { ws } from './ws-client.js';
import { approvalContextLines, approvalLinesHaveHint, extractApprovalOptions, extractHubMarkerApproval, extractPlainYesNoApproval, extractSequentialChoicePrompts, hasApprovalLikeLabel, isBatchOptions, isHubChoicePrompt, isMultiQuestionPrompt, isMultiSelectOptions, markHubChoiceDefault, matchNativeApprovalTrigger } from './approval-parser.js';
import { approvalUiAdapter, setMultiQuestionBannerVisible } from './approval-ui.js';
import { chatHistoryCommitOutput, chatPaneAtBottom, getChatTimelineEl, pushMessage, scrollChatPaneToBottom } from './chat-history.js';
import { token } from './util.js';
import { isActionBarCollapsed, setActionBarCollapsed } from './user-prefs.js';

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- 承認検出 / action-bar ----
// Pure parsing lives in approval-parser.js. This file orchestrates terminal
// tails/buffers and delegates cache/DOM/Hub side effects to approval-ui.js.

// H9 キャッシュ復元の妥当性検証用（C3: 保留中バッジ固着対応）。
// ターミナル直接入力で承認が解決されると approvalConsumedSig が設定されず
// （UI の sendChoice/doSend 経由でしか設定されない）、cache 復元経路が
// action-bar と approvalVisible=true を復元し続ける。scanBuffer から
// キャッシュ済み選択肢が消えた状態が連続 H9_RESTORE_MISS_LIMIT 回続いたら
// 解決済みとみなして復元を打ち切る。Ink 再描画で一瞬選択肢が消えるフレームが
// あるため、1 回のミスでは閉じない（H9 本来の誤消去防止を維持）。
const H9_RESTORE_MISS_LIMIT = 3;
const h9RestoreMisses = new Map();

// 手動「✕ 承認」（消す）で一時的に隠した承認の sig（sessionId → approvalSig）。
// approvalRawOptionsCache / approvalVisibleCache は保持したまま action-bar の描画だけ抑制する。
// showActionBar の choke point で「同一 sig の承認は描画スキップ・別 sig の新しい承認は抑制解除して描画」する。
// 「↻ 承認」（再表示）でこのエントリを消し、承認解決（hideActionBar）でも確実に消す。
const manualHideSig = new Map();

// 複数質問 UI（AskUserQuestion 等）検出の窓と取りこぼし対策。
// scanBuffer の固定 40 行窓だと、端末行数(term.rows)が 40 を超える縦長ターミナルで
// プロンプトがビューポート全体を占め、上端のタブ行（←…→/Submit）が下端 40 行より
// 上に来て検出から外れる。窓をビューポート高さ（=現在画面ぶん）まで広げて必ず含める。
// scrollback の古い残骸は viewport の外（上）へスクロールアウトするため拾わない。
function multiQuestionScanCount(t) {
  const rows = t && t.term && t.term.rows ? t.term.rows : 0;
  return Math.max(40, rows);
}
// タブ行を最後にライブ検出してからこの時間内は、単発ポーリングでタブ行が窓から
// 一瞬外れても multiQ 終了に倒さない（Ink 部分再描画の隙で action-bar に固着するのを防ぐ）。
const MULTIQ_GRACE_MS = 2000;
function multiQuestionRecentlyLive(id) {
  return Date.now() - (multiQuestionLatchAt.get(id) || 0) < MULTIQ_GRACE_MS;
}

// C5: H9 復元ミス時の自走再評価タイマー（保留中バッジ固着対応の補完）。
// detectApproval は PTY チャンク受信時・セッション切替時にしか走らないため、
// 承認解決直後に出力が静止するとミスカウンタが H9_RESTORE_MISS_LIMIT に届く前に
// 評価が止まり、復元ループ（approvalVisible=true 維持 → reassert でリース延命 →
// waiting 固着）になる。ミスを数えた直後に自前で再評価を予約し、
// 出力が無くてもカウンタが上限まで進んで閉じられるようにする。
const H9_REVALIDATE_DELAY_MS = 700;
const h9RevalidateTimers = new Map();

function cancelH9Revalidate(id) {
  const timer = h9RevalidateTimers.get(id);
  if (timer) {
    clearTimeout(timer);
    h9RevalidateTimers.delete(id);
  }
}

function scheduleH9Revalidate(id) {
  cancelH9Revalidate(id);
  h9RevalidateTimers.set(id, setTimeout(() => {
    h9RevalidateTimers.delete(id);
    if (id === activeSessionId) detectApproval(id);
  }, H9_REVALIDATE_DELAY_MS));
}

// C5: 非アクティブセッションの保留中固着対策。
// trackApprovalHintFromChunk は仕様として非アクティブセッションへ false を送らない
// （断片再描画によるチラつき防止）ため、ターミナル直接入力で解決された承認や
// ペーストエコー由来の誤検出は、セッションを開くまで approvalVisible=true が残り、
// reassertApprovalHints（5s 間隔）が Hub のリース（15s）を延命し続けて
// waiting（保留中）が固着する。
// キャッシュ済み選択肢が pendingTextTail 末尾 BG_APPROVAL_TAIL_LINES 行から消えた
// チャンクが BG_APPROVAL_MISS_LIMIT 回連続し、さらに BG_APPROVAL_SETTLE_MS の
// 静定待ちでも再検出されなかった場合のみ解決済みとみなして閉じる。
// 承認待ちのまま入力欄まわりの再描画が続くケースでは、再描画に選択肢が
// 含まれて再検出（resetBgApprovalMisses）されるため誤クリアしない。
const BG_APPROVAL_MISS_LIMIT = 8;
const BG_APPROVAL_SETTLE_MS = 2500;
const BG_APPROVAL_TAIL_LINES = 80;

// [MANY-AI-CLI] マーカーブロックは pendingTextTail 全体から抽出する（固定行数で切らない）。
// 複数質問一括（バッチ）や #multi で選択肢が多く・日本語ラベルが端末幅で折り返されると
// ブロックは容易に数十行へ膨らむ。末尾 N 行で切ると開きマーカーが窓から外れ、閉じマーカー側の
// 質問だけを単問承認として誤描画する（行数依存の構造的バグ）。マーカーは明示デリミタ済みで
// scrollback 誤検出の懸念が無いため、ヒューリスティック scanner（40 行）と分離して全文を渡し、
// 取りこぼし上限は pendingTextTail の保持量（APPROVAL_PENDING_TEXT_TAIL_LIMIT 文字）に一本化する。
function markerLinesFromTail(tail) {
  return String(tail || '').split(/\r\n|\r|\n/).map(l => stripAnsi(l));
}
const bgApprovalMisses = new Map();
const bgApprovalClearTimers = new Map();

function resetBgApprovalMisses(id) {
  bgApprovalMisses.delete(id);
  const timer = bgApprovalClearTimers.get(id);
  if (timer) {
    clearTimeout(timer);
    bgApprovalClearTimers.delete(id);
  }
}

function trackBgApprovalMiss(id, tailLines) {
  const cached = approvalRawOptionsCache.get(id);
  // cache が無い（multiQ 等で visible だけ立った）場合は画面照合できないため対象外
  if (!cached || cached.length === 0) return;
  if (cachedOptionsOnScreen(tailLines, cached)) {
    resetBgApprovalMisses(id);
    return;
  }
  const misses = (bgApprovalMisses.get(id) || 0) + 1;
  bgApprovalMisses.set(id, misses);
  if (misses < BG_APPROVAL_MISS_LIMIT) return;
  if (bgApprovalClearTimers.has(id)) return; // 静定待ち中
  bgApprovalClearTimers.set(id, setTimeout(() => {
    bgApprovalClearTimers.delete(id);
    bgApprovalMisses.delete(id);
    // 静定待ちの間に再検出されていれば resetBgApprovalMisses がタイマーごと
    // 取り消すためここには来ない。アクティブ化されていたら detectApproval
    //（H9 復元 + 自走再評価）に委ねる。
    if (!approvalVisibleCache.get(id) || id === activeSessionId) return;
    approvalUiAdapter.clearApprovalOptions(id);
    approvalSourceCache.delete(id);
    approvalUiAdapter.setApprovalVisible(id, false);
  }, BG_APPROVAL_SETTLE_MS));
}

// キャッシュ済み選択肢のいずれかが描画済み行（scanBuffer）にまだ存在するか。
// 「番号トークン + ラベル先頭 12 文字」が同一行にあることを条件にする
// （ラベル単独だと本文中の同語にマッチしやすく、番号単独だと箇条書きに誤マッチするため）。
// バッチ（複数質問）形式はセクション {num, title, options} の配列で label を持たないため、
// 照合前にフラットな選択肢へ展開する。展開しないと照合が必ず失敗し、H9 復元ミスが
// 上限（3回 × 700ms ≒ 2.1s）に達するたび hideActionBar → マーカー再検出で再表示、
// の約2秒周期チカチカになる（Claude の [MANY-AI-CLI] 一括質問で発生）。
function cachedOptionsOnScreen(lines, cached) {
  const flat = isBatchOptions(cached)
    ? cached.flatMap((s) => s.options || [])
    : cached;
  return flat.some((o) => {
    const frag = String(o.label || '').slice(0, 12).trim();
    if (!frag) return false;
    const numToken = `${o.num}.`;
    return lines.some((line) => {
      const s = String(line || '');
      return s.includes(numToken) && s.includes(frag);
    });
  });
}

export function isUserSpecifiesText(text) {
  const re = globalThis.approvalParser && globalThis.approvalParser.userSpecifiesRe;
  return !!(re && re.test(String(text || '')));
}

export function getSequentialChoiceState(id, prompts) {
  if (!prompts || prompts.length < 2) return null;
  const sig = sequentialChoiceSig(prompts);
  let state = sequentialChoiceCache.get(id);
  if (!state || state.sig !== sig) {
    state = { sig, prompts, answers: new Map(), index: 0 };
    sequentialChoiceCache.set(id, state);
  } else {
    state.prompts = prompts;
  }
  while (state.index < state.prompts.length && state.answers.has(state.prompts[state.index].key)) {
    state.index++;
  }
  return state.index < state.prompts.length ? state : null;
}

export function sequentialChoiceOptionsForState(state) {
  if (!state) return [];
  const prompt = state.prompts[state.index];
  const question = `${prompt.key}: ${prompt.question}`;
  return prompt.options.map((opt, idx) => ({
    ...opt,
    isCurrent: idx === 0,
    _sequentialChoice: true,
    _sequentialKey: prompt.key,
    _sequentialQuestion: question,
  }));
}

export function clearSequentialChoiceState(id) {
  sequentialChoiceCache.delete(id);
}

// ---- 承認検出 (xterm.js バッファスキャン) ----

// ---- provider 分類 helper ----
// Go 側の isAIProvider と対応する。承認検出・chat history・done summary 等の
// AI 固有機能を適用するかどうかの判定に使う。Shell provider は対象外。
export function isAIProvider(provider: string): boolean {
  switch (provider) {
    case 'claude':
    case 'codex':
    case 'copilot':
    case 'cursor-agent':
    case 'opencode':
      return true;
    default:
      return false;
  }
}

export function isShellProvider(provider: string): boolean {
  return provider === 'shell';
}

// provider 別の承認 trigger phrase は ~/.many-ai-cli/approval-patterns/{provider}.json に外出し。
// Hub 起動時にデフォルトをユーザー設定ディレクトリに展開（既存ファイルは尊重）し、
// HTTP 経由で配信する。ユーザーが直接編集して文言を追加・調整できる。
// claude / codex は英語固定（Anthropic/OpenAI が国際化していない）、common は多言語混在。
export const providerApprovalTriggers = { claude: [], codex: [], copilot: [], 'cursor-agent': [], opencode: [], common: [] };

(async function loadApprovalPatterns() {
  const fetchJson = async (name) => {
    try {
      const res = await fetch(`approval-patterns/${encodeURIComponent(name)}.json?token=${encodeURIComponent(token || '')}`);
      if (!res.ok) return [];
      return await res.json();
    } catch (e) {
      console.warn(`approval-patterns/${name}.json load failed`, e);
      return [];
    }
  };
  const [claude, codex, copilot, cursorAgent, opencode, common] = await Promise.all([
    fetchJson('claude'), fetchJson('codex'), fetchJson('copilot'), fetchJson('cursor-agent'), fetchJson('opencode'), fetchJson('common'),
  ]);
  const norm = arr => (Array.isArray(arr) ? arr : []).map(s => String(s).toLowerCase()).filter(Boolean);
  providerApprovalTriggers.claude = norm(claude);
  providerApprovalTriggers.codex  = norm(codex);
  providerApprovalTriggers.copilot = norm(copilot);
  providerApprovalTriggers['cursor-agent'] = norm(cursorAgent);
  providerApprovalTriggers.opencode = norm(opencode);
  providerApprovalTriggers.common = norm(common);
})();

export function matchProviderApprovalTrigger(provider, line) {
  if (!line) return false;
  const lower = String(line).toLowerCase();
  if (provider === 'codex' && isCodexModelSelectorHint(lower)) return false;
  const list = providerApprovalTriggers[provider] || [];
  for (const s of list) if (lower.includes(s)) return true;
  for (const s of providerApprovalTriggers.common) if (lower.includes(s)) return true;
  return false;
}

function isCodexModelSelectorHint(lower) {
  return lower.includes('select model') ||
    lower.includes('select effort') ||
    lower.includes('model and effort') ||
    lower.includes('reasoning effort') ||
    lower.includes('esc to go back') ||
    lower.includes('↑/↓ to change') ||
    lower.includes('arrow keys');
}

function isCodexModelSelectorContext(provider, lines) {
  if (provider !== 'codex') return false;
  const text = (lines || []).map(line => String(line || '').toLowerCase()).join('\n');
  return text.includes('select model') ||
    text.includes('select effort') ||
    text.includes('model and effort') ||
    text.includes('reasoning effort') ||
    ((text.includes('gpt-') || text.includes('effort')) && (text.includes('esc to go back') || text.includes('press enter to confirm')));
}

// /model 等のカーソル駆動 TUI 選択メニュー（承認ではない）を action-bar に出す際の
// タイトル抽出。選択肢クラスタの直上から、選択肢行・フッターヒント・長い説明文を除いた
// 最初の短い見出し行を採用する（例: claude /model の "Select model"）。
function extractSelectMenuTitle(lines, cluster) {
  if (!cluster || !Array.isArray(lines)) return null;
  const limit = Math.max(0, cluster.start - 8);
  for (let i = cluster.start - 1; i >= limit; i--) {
    const ln = String(lines[i] || '').trim();
    if (!ln) continue;
    if (/^[>❯›❱]?\s*\d{1,2}\.\s/.test(ln)) continue;       // 選択肢行
    if (matchNativeApprovalTrigger(ln)) continue;          // フッターヒント行
    if (ln.length > 40 && /[.。]\s*$/.test(ln)) continue;  // 長い説明文（タイトルではない）
    return ln.replace(/\s{2,}.*$/, '').slice(0, 60);
  }
  return null;
}

// カーソル駆動だが承認ではない選択メニュー（claude /model 等）の options に
// _selectMenu / _menuTitle を付与する。承認との見分け（ラベル表示）と、メニュー表示中の
// チャット入力ガード（doSend が末尾 \r で現在選択を誤確定するのを防ぐ）に使う。
function tagSelectMenuOptions(options, isSelectMenu, contextSourceLines, contextCluster) {
  if (!isSelectMenu || !Array.isArray(options) || options.length === 0) return;
  const title = extractSelectMenuTitle(contextSourceLines, contextCluster);
  for (const o of options) {
    if (!o) continue;
    o._selectMenu = true;
    if (title) o._menuTitle = title;
  }
}

// メニュー表示中ガード用: action-bar に表示中の選択肢が「承認ではない選択メニュー」かを返す。
// doSend はこれが true の間、プレーンテキスト注入（末尾 \r）を保留する。
export function isSelectMenuActive(id) {
  if (!approvalVisibleCache.get(id)) return false;
  const cached = approvalRawOptionsCache.get(id);
  return Array.isArray(cached) && cached.some(o => o && o._selectMenu);
}

export const approvalCheckTimers = new Map(); // セッション別タイマー（マルチペインで単一タイマーに上書きされる問題を解消）

export function cancelApprovalHintConfirm(id) {
  const timer = approvalHintConfirmTimers.get(id);
  if (timer) {
    clearTimeout(timer);
    approvalHintConfirmTimers.delete(id);
    approvalHintConfirmTrusted.delete(id);
  }
}

export const approvalSuppressRescanTimers = new Map();

export function scheduleApprovalSuppressRescan(id, suppressUntil) {
  const prev = approvalSuppressRescanTimers.get(id);
  if (prev) clearTimeout(prev);
  const delay = Math.max(0, suppressUntil - Date.now()) + 30;
  approvalSuppressRescanTimers.set(id, setTimeout(() => {
    approvalSuppressRescanTimers.delete(id);
    detectApproval(id);
    maybeAutoSwitchToNextApproval();
  }, delay));
}

export function scheduleApprovalHintConfirm(id, options) {
  if (!options || options.length === 0) return;
  // 承認 UI が生きている証拠なので、非アクティブ固着判定のミスカウンタを取り消す
  resetBgApprovalMisses(id);
  const sig = approvalSig(options);
  cancelApprovalHintConfirm(id);
  approvalHintConfirmTimers.set(id, setTimeout(() => {
    approvalHintConfirmTimers.delete(id);
    approvalHintConfirmTrusted.delete(id);
    const cached = approvalRawOptionsCache.get(id);
    if (!cached || cached.length === 0) return;
    const cachedSig = approvalSig(cached);
    if (cachedSig !== sig) return;
    const wasVisible = approvalVisibleCache.get(id);
    if (!wasVisible) {
      approvalUiAdapter.setApprovalVisible(id, true, { sound: true });
    }
    // 連続して [MANY-AI-CLI] ブロックが来た場合 (例: 1質問目を回答せず 2質問目が来た) は
    // 既に approvalVisible=true でも action-bar を最新オプションに張り替える。
    if (id === activeSessionId) {
      const bar = document.getElementById('action-bar');
      if (bar) approvalUiAdapter.showOptions(bar, id, cached, !wasVisible);
    }
  }, 350));
}

export function trackApprovalHintFromChunk(id, bytes, decodedText) {
  const t = terminals.get(id);
  if (!t) return;
  const provider = sessions.get(id)?.provider;
  // Shell session は approval parser の対象外
  if (!isAIProvider(provider || '')) return;
  const text = decodedText !== undefined ? decodedText : (t.textDecoder || utf8Decoder).decode(bytes, { stream: true });
  t.pendingTextTail = (t.pendingTextTail + text).slice(-APPROVAL_PENDING_TEXT_TAIL_LIMIT);

  // sendChoice 直後の誤再表示を抑制
  const suppressUntil = approvalSuppressUntil.get(id);
  if (suppressUntil && Date.now() < suppressUntil) {
    scheduleApprovalSuppressRescan(id, suppressUntil);
    return;
  }
  approvalSuppressUntil.delete(id);

  const rawLines = t.pendingTextTail.split(/\r\n|\r|\n/).slice(-40);
  const lines = rawLines.map(l => stripAnsi(l));

  // 複数質問 UI を最優先で判定 — Hub の action-bar では正しく駆動できないので
  // 検出したら通常の承認検出をスキップする。スクロールバック残骸での誤検出を避けるため
  // ターミナル末尾 40 行に限定する（AskUserQuestion UI は通常 ~20 行以内に収まる）。
  let multiQContext = lines;
  // scanBuffer は active セッションか、pending に承認/multiQ ヒントがある場合のみ呼ぶ。
  // 非アクティブセッションの全チャンクに対して scanBuffer を走らせると多セッション時の CPU 負荷が増大する。
  const pendingHasMultiQHint = isMultiQuestionPrompt(lines) || approvalLinesHaveHint(provider, lines);
  if (t && t.everAttached && (id === activeSessionId || pendingHasMultiQHint)) {
    multiQContext = lines.concat(scanBuffer(id, multiQuestionScanCount(t)));
  }
  if (isMultiQuestionPrompt(multiQContext)) {
    multiQuestionLatchAt.set(id, Date.now()); // タブ行のライブ検出を記録（grace デバウンス基準）
    // ユーザーが ✕ で手動 dismiss した場合は再表示しない（誤検出を尊重）
    const dismissed = multiQuestionDismissedCache.get(id);
    // regular approval が確認済みなら false positive として扱い、multiQuestion 検出を完全にスキップする。
    // xterm scrollback に前回 AskUserQuestion の「Review your answers」等が残ると誤検出し、
    // ここで cache を削除すると detectApproval の H9 救済が機能せずチカチカする。
    // sendChoice / doSend 経路では確実に cache.delete されるため、この保護は安全。
    const hasCachedApproval = approvalVisibleCache.get(id) && (approvalRawOptionsCache.get(id)?.length > 0);
    if (!dismissed && !hasCachedApproval) {
      cancelApprovalHintConfirm(id);
      approvalUiAdapter.clearApprovalOptions(id);
      if (!multiQuestionVisibleCache.get(id)) {
        multiQuestionVisibleCache.set(id, true);
        if (id === activeSessionId) setMultiQuestionBannerVisible(true);
        // 待機通知を Hub と同期（auto-switch とサウンドは action-bar と同等の扱い）
        if (!approvalVisibleCache.get(id)) {
          approvalUiAdapter.setApprovalVisible(id, true, { sound: true });
        }
      } else if (id === activeSessionId) {
        setMultiQuestionBannerVisible(true);
      }
      return;
    }
  } else if (multiQuestionVisibleCache.get(id) && multiQuestionRecentlyLive(id)) {
    // タブ行を直前までライブ検出していた multiQ 中で、今回のチャンクだけ Ink 部分再描画で
    // タブ行が窓から外れたケース。grace 期間内は通常承認としてキャッシュせず据え置く。
    // これを通すと選択肢リストが単問承認として確定し、hasCachedApproval ガードで
    // 以後 multiQ 判定が恒久スキップされて action-bar に固着する。
    return;
  }

  // フォーマットベース検出（優先）: [MANY-AI-CLI] マーカーがあれば即確定。
  // マーカーブロックは大きくなり得る（複数質問・長い日本語ラベルの折り返し）ため、
  // 40 行の lines ではなく pendingTextTail 全体（markerLinesFromTail）から抽出する。
  const markerOpts = extractHubMarkerApproval(markerLinesFromTail(t.pendingTextTail));
  if (markerOpts) {
    // 回答済みの [MANY-AI-CLI] ブロックは恒久的に承認 UI を出さない（タブ切替の SIGWINCH
    // 再描画で画面に残った回答済みブロックが再流入しても再表示しない）。質問文込みのハッシュ
    // で判定するため、別質問を誤って抑制することはない。
    if (isAnsweredMarkerSig(id, markerOpts)) return;
    // doSend でテキスト送信済みの承認が Ink 再描画で再検出された場合はスキップ
    const consumed = approvalConsumedSig.get(id);
    const sig = approvalSig(markerOpts);
    if (consumed === sig) {
      // Ink 再描画で同一ブロックが再送されている — タイマーをリセットして
      // ブロックが届かなくなるまで sig を保持し続ける（debounce 型削除）
      const prev = approvalConsumedSigDeleteTimer.get(id);
      if (prev) clearTimeout(prev);
      approvalConsumedSigDeleteTimer.set(id, setTimeout(() => {
        approvalConsumedSig.delete(id);
        approvalConsumedSigDeleteTimer.delete(id);
      }, 5000));
      return;
    }
    // 異なる選択肢 → 新しい質問なのでリセット
    const prevTimer = approvalConsumedSigDeleteTimer.get(id);
    if (prevTimer) { clearTimeout(prevTimer); approvalConsumedSigDeleteTimer.delete(id); }
    approvalConsumedSig.delete(id);
    approvalUiAdapter.cacheApprovalOptions(id, markerOpts);
    approvalHintConfirmTrusted.set(id, true); // 信頼できる検出としてマーク: fallback による cancel/clear を防ぐ
    scheduleApprovalHintConfirm(id, markerOpts);
    return;
  }

  const plainYesNoOpts = extractPlainYesNoApproval(lines);
  if (plainYesNoOpts) {
    const consumed = approvalConsumedSig.get(id);
    const sig = approvalSig(plainYesNoOpts);
    if (consumed === sig) {
      const prev = approvalConsumedSigDeleteTimer.get(id);
      if (prev) clearTimeout(prev);
      approvalConsumedSigDeleteTimer.set(id, setTimeout(() => {
        approvalConsumedSig.delete(id);
        approvalConsumedSigDeleteTimer.delete(id);
      }, 5000));
      return;
    }
    const prevTimer = approvalConsumedSigDeleteTimer.get(id);
    if (prevTimer) { clearTimeout(prevTimer); approvalConsumedSigDeleteTimer.delete(id); }
    approvalConsumedSig.delete(id);
    approvalUiAdapter.cacheApprovalOptions(id, plainYesNoOpts);
    approvalHintConfirmTrusted.set(id, true); // 信頼できる検出としてマーク
    scheduleApprovalHintConfirm(id, plainYesNoOpts);
    return;
  }

  const seqPrompts = extractSequentialChoicePrompts(multiQContext);
  const seqState = getSequentialChoiceState(id, seqPrompts);
  if (seqState) {
    const seqOpts = sequentialChoiceOptionsForState(seqState);
    approvalUiAdapter.cacheApprovalOptions(id, seqOpts);
    scheduleApprovalHintConfirm(id, seqOpts);
    return;
  }
  if (!seqPrompts) clearSequentialChoiceState(id);

  if (isGoNativeApprovalActive(id)) {
    return;
  }

  // フォールバック検出（既存）
  let extraction = extractApprovalOptions(lines);
  const options = extraction.options;
  // cluster の index は展開後の行（Ink 連結を再分割した結果）が基準。
  // approvalContextLines で同じ配列を使わないと index ずれでコンテキストを取り違える。
  let contextSourceLines = extraction.lines || lines;
  let contextCluster = extraction.cluster;

  // 非アクティブセッション含めて xterm の解釈済みバッファ (scanBuffer) も参照する。
  // Ink/Codex のカーソル位置制御による再描画は pendingTextTail を行分割しても
  // カーソル付き選択肢を取り出せないため、xterm 解釈済みのバッファのほうが
  // より正確に行構造とカーソル位置を保持している。
  // ただし scanBuffer は履歴を保持し続けるため、承認解決後の応答チャンクで
  // 古い選択肢が再検出される（approvalConsumedSig は label 差異で抑止が外れる）。
  // pendingTextTail に承認系の手がかりが無く、かつ既に visible でも無い場合は scanBuffer を見ない。
  const pendingHasApprovalHint = approvalLinesHaveHint(provider, lines);
  // allowBufferFallback を先に計算し、条件が揃った場合のみ scanBuffer を呼ぶ。
  // 全チャンク毎に scanBuffer を走らせると多セッション時の JS 処理負荷が増大するため、
  // 手がかりがない場合はスキップする。
  const allowBufferFallback = approvalVisibleCache.get(id) || options.length > 0 || pendingHasApprovalHint;
  const visibleRows = t?.term?.rows || 40;
  const bufferTail = (t && t.everAttached && allowBufferFallback) ? scanBuffer(id, Math.max(120, visibleRows + 60)) : [];
  if (t && t.everAttached && allowBufferFallback) {
    const bufExtraction = extractApprovalOptions(bufferTail);
    const bufOpts = bufExtraction.options;
    const pendingHasCursor = options.some(o => o.isCurrent);
    const bufHasCursor = bufOpts.some(o => o.isCurrent);
    if (bufOpts.length > options.length || (!pendingHasCursor && bufHasCursor && bufOpts.length >= options.length)) {
      options.length = 0;
      options.push(...bufOpts);
      contextSourceLines = bufExtraction.lines || bufferTail;
      contextCluster = bufExtraction.cluster;
    }
    // option 1 が pendingTextTail から欠落している場合（保持上限）に補完
    if (options.length >= 1 && !options.some(o => o.num === 1)) {
      const maxNum = Math.max(...options.map(o => o.num));
      if (bufOpts.length > 0 && Math.max(...bufOpts.map(o => o.num)) === maxNum) {
        for (const bo of bufOpts) {
          if (!options.some(o => o.num === bo.num)) options.push(bo);
        }
        options.sort((a, b) => a.num - b.num);
        contextSourceLines = bufExtraction.lines || bufferTail;
        contextCluster = bufExtraction.cluster;
      }
    }
  }
  const contextLines = approvalContextLines(contextSourceLines, contextCluster);

  markHubChoiceDefault(options, contextLines);
  const lastOpt = options[options.length - 1];
  const hasUserSpecifies = (lastOpt && isUserSpecifiesText(lastOpt.label)) || contextLines.some(line => isUserSpecifiesText(line));
  // Ink UI は常に選択中の項目に > / ❯ カーソルを付ける（isCurrent: true）。
  // カーソル付き選択肢がない場合は AI の通常応答の箇条書きとみなして無視する。
  const hasCursorOption = options.some(o => o.isCurrent);
  // 実際のCLI承認プロンプトは yes/no/allow/deny/proceed 等を含む。
  // Claude の通常回答（「パネル幅拡大...」等）との誤検出を防ぐため、
  // ラベル内容が承認系のときのみ approvalNear を評価する。
  const approvalLabelRe = /\b(yes|no|allow|deny|proceed|abort|don[''']t ask|cancel|once|always|permission|confirm|details)\b/i;
  const hasApprovalLikeLabel = options.some((opt) => approvalLabelRe.test(opt.label));
  const isHubChoice = isHubChoicePrompt(contextLines, options);
  const suppressCodexModelSelector = isCodexModelSelectorContext(provider, contextLines);
  const hasNativePromptHint = !suppressCodexModelSelector && contextLines.some((line) => !String(line || '').toLowerCase().includes('esc to go back') && (matchProviderApprovalTrigger(provider, line) || matchNativeApprovalTrigger(line)));
  const isShortcutApprovalMenu = (provider === 'codex' || provider === 'copilot' || provider === 'cursor-agent' || provider === 'opencode') && options.some(o => o._sendText) && hasNativePromptHint;
  const approvalNear = (hasCursorOption || isShortcutApprovalMenu) &&
    ((hasApprovalLikeLabel && (hasUserSpecifies || contextLines.some((line) => matchProviderApprovalTrigger(provider, line) || matchNativeApprovalTrigger(line)))) || isHubChoice);
  const hasChoiceMenuHint = (hasCursorOption || isShortcutApprovalMenu) && options.length > 0 && hasNativePromptHint;
  const nowVisible = (options.length > 0 && approvalNear) || hasChoiceMenuHint;
  // 承認ではないカーソル駆動の選択メニュー（claude /model 等）にタグ付けする。
  // 入力ガード（isSelectMenuActive）とメニュータイトル表示に使う。
  const isSelectMenu = hasChoiceMenuHint && !approvalNear && hasCursorOption && !isShortcutApprovalMenu;
  tagSelectMenuOptions(options, isSelectMenu, contextSourceLines, contextCluster);

  // doSend / sendChoice で消費済みの選択肢が xterm scanBuffer に残っているため
  // フォールバック検出で再抽出されるケースを抑止する（marker 検出と同じ debounce 戦略）。
  if (nowVisible) {
    const consumed = approvalConsumedSig.get(id);
    const sig = approvalSig(options);
    if (consumed === sig) {
      const prev = approvalConsumedSigDeleteTimer.get(id);
      if (prev) clearTimeout(prev);
      approvalConsumedSigDeleteTimer.set(id, setTimeout(() => {
        approvalConsumedSig.delete(id);
        approvalConsumedSigDeleteTimer.delete(id);
      }, 5000));
      return;
    }
  }

  if (nowVisible) {
    // 承認 UI が描画され続けている証拠なので、非アクティブ固着判定のミスカウンタを取り消す
    resetBgApprovalMisses(id);
    // Anti-flicker: 表示中の承認と異なる選択肢が検出されたときはキャッシュを更新しない。
    // Codex/Copilot の TUI 再描画で pendingTextTail に部分的・交互のオプションが混入する問題への対処。
    // 切り替えは detectApproval の安定性ガード（700ms）が担う。
    const wasVisible = approvalVisibleCache.get(id);
    const existingCached = wasVisible && approvalRawOptionsCache.get(id);
    const skipCacheUpdate = !!(
      wasVisible &&
      existingCached && existingCached.length > 0 &&
      !isBatchOptions(existingCached) &&
      !isBatchOptions(options) &&
      approvalSig(existingCached) !== approvalSig(options)
    );
    if (!skipCacheUpdate) {
      approvalUiAdapter.cacheApprovalOptions(id, options);
    }
  } else {
    // 信頼できる検出（marker / plainYesNo）の confirm タイマーが pending の間は
    // cancel も cache 削除も行わない。マルチペインで非アクティブセッションに後続の
    // PTY チャンクが届き fallback 検出が nowVisible=false になっても、350ms タイマーが
    // 発火して approvalVisibleCache をセットするまでキャッシュを守る。
    // approvalVisibleCache=true になった後は既存の保護（下の if 条件）が引き継ぐ。
    const isTrustedPending = approvalHintConfirmTrusted.get(id);
    if (!isTrustedPending) {
      cancelApprovalHintConfirm(id);
    }
    // approvalVisibleCache=true の間（= Hub marker / plainYesNo / フォールバックいずれかで
    // 既に承認 UI を表示中）は cache を保護する。Claude Code の thinking スピナー
    // ("Worked for Xs") 等で pendingTextTail がローテートし `(Y:1/N:0)` 行が末尾 20-40 行
    // から押し出されると、フォールバック検出が「実行内容: 1. ... 2. ..." 等の番号付き本文を
    // 拾うが承認ラベルが無いため nowVisible=false になる。ここで cache を削除すると
    // detectApproval の復元経路 (action-bar 消失防止) が動かず、action-bar が
    // 表示・非表示を高頻度で繰り返す（画面チカチカ）症状になる。
    // sendChoice / doSend / hideActionBar の経路では確実に cache.delete されるため
    // この保護は安全（解決済み承認の残留は起きない）。
    if (!approvalVisibleCache.get(id) && !isTrustedPending) {
      approvalUiAdapter.clearApprovalOptions(id);
    }
    // C5: 非アクティブセッションで解決済み承認が残留した場合の自動クリア判定。
    // キャッシュ済み選択肢が末尾 BG_APPROVAL_TAIL_LINES 行から消えた状態の
    // チャンクを数え、連続ミス + 静定待ちで approvalVisible=false に倒す。
    if (id !== activeSessionId && approvalVisibleCache.get(id) && !isTrustedPending) {
      const bgTail = t.pendingTextTail.split(/\r\n|\r|\n/).slice(-BG_APPROVAL_TAIL_LINES).map(l => stripAnsi(l));
      trackBgApprovalMiss(id, bgTail);
    }
  }

  // true への遷移は短時間 debounce してから送信する。PTY 生バイト上だけの一瞬の誤検出で
  // サイドバーが waiting/running を往復するのを防ぐ。
  if (nowVisible && !approvalVisibleCache.get(id)) {
    scheduleApprovalHintConfirm(id, options);
  }
  // 非アクティブセッションでは false を送らない。
  // PTY の断片的な再描画で nowVisible が一時的に false になると、
  // waiting -> running/standby のチラつきが起きるため、保留状態を維持する。
  // false への遷移はアクティブ時の detectApproval/hideActionBar で確定させる。
}

export function scheduleApprovalCheck(id) {
  // セッション別タイマーを使い、マルチペインで他セッションの呼び出しに上書きされないようにする。
  // detectApproval はグローバル action-bar を操作するためアクティブセッション専用。
  // 非アクティブセッションの状態は trackApprovalHintFromChunk + approvalHintConfirmTrusted が管理する。
  const prev = approvalCheckTimers.get(id);
  if (prev) clearTimeout(prev);
  approvalCheckTimers.set(id, setTimeout(() => {
    approvalCheckTimers.delete(id);
    if (id === activeSessionId) detectApproval(id);
  }, 300));
}

export function normalizeGoApprovalOptions(rawOptions) {
  return (Array.isArray(rawOptions) ? rawOptions : [])
    .map((opt) => ({
      num: Number(opt.num),
      label: String(opt.label || '').trim(),
      isCurrent: !!opt.is_current,
      preserveOrder: !!opt.preserve_order,
      _sendText: opt.send_text || undefined,
    }))
    .filter((opt) => Number.isFinite(opt.num) && opt.label);
}

export function isGoNativeApprovalActive(id) {
  const src = approvalSourceCache.get(id);
  return !!(src && src.source === 'go_vt' && approvalVisibleCache.get(id));
}

export function handleGoApprovalDetected(message) {
  const id = message && message.session_id;
  if (!id) return;
  const options = normalizeGoApprovalOptions(message.approval_options);
  if (options.length === 0) return;
  const sig = String(message.approval_sig || approvalSig(options));
  options.forEach((opt: any) => {
    opt._approvalSource = 'go_vt';
    opt._approvalSig = sig;
  });

  cancelApprovalHintConfirm(id);
  approvalSwitchCandidates.delete(id);
  approvalConsumedSig.delete(id);
  resetBgApprovalMisses(id);
  approvalUiAdapter.cacheApprovalOptions(id, options);
  approvalSourceCache.set(id, {
    source: 'go_vt',
    sig,
    kind: message.approval_kind || 'native',
    detectedAt: message.detected_at || '',
  });

  const wasVisible = !!approvalVisibleCache.get(id);
  approvalUiAdapter.setApprovalVisible(id, true, { sound: !wasVisible });
  if (id === activeSessionId) {
    const bar = document.getElementById('action-bar');
    if (bar) approvalUiAdapter.showOptions(bar, id, options, !wasVisible);
  }
}

export function handleGoApprovalCleared(message) {
  const id = message && message.session_id;
  if (!id) return;
  const src = approvalSourceCache.get(id);
  if (!src || src.source !== 'go_vt') return;
  if (message.approval_sig && src.sig && message.approval_sig !== src.sig) return;
  approvalSourceCache.delete(id);
  approvalUiAdapter.clearApprovalOptions(id);
  approvalSwitchCandidates.delete(id);
  cancelApprovalHintConfirm(id);
  resetBgApprovalMisses(id);
  if (approvalVisibleCache.get(id)) {
    approvalUiAdapter.setApprovalVisible(id, false);
  }
  if (id === activeSessionId) {
    hideActionBar(id);
  }
}

export function sendApprovalConsumed(sessionId, options, sentText) {
  const src = approvalSourceCache.get(sessionId);
  if (!src || src.source !== 'go_vt' || !src.sig) return;
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  ws.send(JSON.stringify({
    type: 'approval_consumed',
    session_id: sessionId,
    approval_sig: src.sig,
    approval_source: 'go_vt',
    sent_text: sentText || '',
  }));
  if (options) approvalConsumedSig.set(sessionId, approvalSig(options));
}

export function maybeSendDirectApprovalConsumed(sessionId, rawText, sentText) {
  const src = approvalSourceCache.get(sessionId);
  if (!src || src.source !== 'go_vt') return;
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!Array.isArray(cached) || isBatchOptions(cached)) return;
  const trimmed = String(rawText || '').trim();
  const matched = cached.find((opt) => {
    if (!opt) return false;
    const optSend = opt._sendText || `${opt.num}`;
    return trimmed === String(opt.num) || trimmed === String(optSend).trim();
  });
  if (matched) sendApprovalConsumed(sessionId, cached, sentText);
}

export function detectApproval(id) {
  // Shell session は approval parser の対象外
  const provider = sessions.get(id)?.provider;
  if (!isAIProvider(provider || '')) return;

  // sendChoice 直後の誤再表示を抑制
  const suppressUntil = approvalSuppressUntil.get(id);
  if (suppressUntil && Date.now() < suppressUntil) {
    scheduleApprovalSuppressRescan(id, suppressUntil);
    return;
  }
  approvalSuppressUntil.delete(id);

  const bar = document.getElementById('action-bar');
  if (!bar) return;

  const tEarly = terminals.get(id);
  // 複数質問 UI（AskUserQuestion 等）の判定を最優先。末尾 40 行に限定して scrollback 残骸を除外。
  // scanBuffer は active セッションのみ（非アクティブは pendingTextTail の末尾で代替）。
  const mqScanCount = multiQuestionScanCount(tEarly);
  const mqPending = (tEarly?.pendingTextTail || '').split(/\r\n|\r|\n/).slice(-mqScanCount).map(l => stripAnsi(l));
  const mqLines = (tEarly?.everAttached && id === activeSessionId)
    ? mqPending.concat(scanBuffer(id).slice(-mqScanCount))
    : mqPending;
  if (isMultiQuestionPrompt(mqLines)) {
    multiQuestionLatchAt.set(id, Date.now()); // タブ行のライブ検出を記録（grace デバウンス基準）
    // 確認済みの regular approval がある場合は false positive として扱い、
    // action-bar を消さず通常の承認検出にフォールスルーする。
    // xterm scrollback に前回 AskUserQuestion の残骸が残ると誤検出しチカチカする。
    // また ✕ で手動 dismiss された場合も再表示しない。
    const hasCachedApproval = approvalVisibleCache.get(id) && (approvalRawOptionsCache.get(id)?.length > 0);
    const dismissed = multiQuestionDismissedCache.get(id);
    if (!hasCachedApproval && !dismissed) {
      // action-bar は出さない（誤誘導防止）
      approvalUiAdapter.clearActionBarDom();
      if (!multiQuestionVisibleCache.get(id)) {
        multiQuestionVisibleCache.set(id, true);
        if (!approvalVisibleCache.get(id)) {
          approvalUiAdapter.setApprovalVisible(id, true);
        }
      }
      if (id === activeSessionId) setMultiQuestionBannerVisible(true);
      return;
    }
    // false positive: 残存 multiQuestionVisibleCache があれば除去して
    // transition path が後で approvalVisibleCache を誤クリアしないようにする
    if (multiQuestionVisibleCache.get(id)) {
      multiQuestionVisibleCache.delete(id);
      if (id === activeSessionId) setMultiQuestionBannerVisible(false);
    }
    // fall through to regular approval detection
  } else if (multiQuestionVisibleCache.get(id)) {
    if (multiQuestionRecentlyLive(id)) {
      // 直前までタブ行をライブ検出していた。今回だけ Ink 部分再描画でタブ行が窓から
      // 外れた transient miss とみなし、banner を据え置いて通常承認へ倒さない。
      // これが無いと単発ミスで banner→action-bar に転移し、以後 hasCachedApproval
      // ガードで multiQ が恒久スキップされ action-bar に固着する。
      if (id === activeSessionId) setMultiQuestionBannerVisible(true);
      return;
    }
    // grace を超えてタブ行が消えた = multiQ が genuinely 終了: approvalVisibleCache を false に戻す
    multiQuestionVisibleCache.delete(id);
    multiQuestionDismissedCache.delete(id);
    multiQuestionLatchAt.delete(id);
    if (id === activeSessionId) setMultiQuestionBannerVisible(false);
    if (approvalVisibleCache.get(id)) {
      approvalUiAdapter.setApprovalVisible(id, false);
    }
  }
  // 検出側もマッチしない & state も無い場合でも dismissed フラグはここでクリアしない。
  // Ink の全画面再描画で誤検出元の行が末尾40行の窓を出入りすると、ここでクリアすると
  // ✕ で dismiss しても窓への再入で banner が即復活してしまう（バツボタンが効かない）。
  // dismissed は送信時（doSend）と一括回答確定時に確実にクリアされるため、それで十分。

  // [MANY-AI-CLI] マーカー検出: xterm バッファではなく pendingTextTail を使う。
  // xterm バッファは回答済みの古い [MANY-AI-CLI] ブロックを保持し続けるため、
  // suppress 期間が切れると再検出・再表示されてしまう。
  // pendingTextTail は hideActionBar でクリアされるが、Ink 再描画で同一内容が
  // 再び入ることがあるため approvalConsumedSig で二重表示を防ぐ。
  const t = terminals.get(id);
  if (t) {
    const pendingLines = markerLinesFromTail(t.pendingTextTail);
    const markerOpts = extractHubMarkerApproval(pendingLines);
    if (markerOpts) {
      // 回答済みブロックは恒久的にスキップ（タブ切替の再描画で再流入しても出さない）
      if (isAnsweredMarkerSig(id, markerOpts)) return;
      const consumed = approvalConsumedSig.get(id);
      const sig = approvalSig(markerOpts);
      if (consumed === sig) return; // 消費済み承認の再表示をスキップ（タイマーは trackApprovalHintFromChunk 側で管理）
      const prevTimer2 = approvalConsumedSigDeleteTimer.get(id);
      if (prevTimer2) { clearTimeout(prevTimer2); approvalConsumedSigDeleteTimer.delete(id); }
      approvalConsumedSig.delete(id);
      approvalUiAdapter.cacheApprovalOptions(id, markerOpts);
      const wasVisible = !!approvalVisibleCache.get(id);
      approvalUiAdapter.showOptions(bar, id, markerOpts, !wasVisible);
      if (!wasVisible) {
        cancelApprovalHintConfirm(id);
        approvalUiAdapter.setApprovalVisible(id, true);
      }
      return;
    }

    const plainYesNoOpts = extractPlainYesNoApproval(pendingLines);
    if (plainYesNoOpts) {
      const consumed = approvalConsumedSig.get(id);
      const sig = approvalSig(plainYesNoOpts);
      if (consumed === sig) return;
      const prevTimer2 = approvalConsumedSigDeleteTimer.get(id);
      if (prevTimer2) { clearTimeout(prevTimer2); approvalConsumedSigDeleteTimer.delete(id); }
      approvalConsumedSig.delete(id);
      approvalUiAdapter.cacheApprovalOptions(id, plainYesNoOpts);
      const wasVisible = !!approvalVisibleCache.get(id);
      approvalUiAdapter.showOptions(bar, id, plainYesNoOpts, !wasVisible);
      if (!wasVisible) {
        cancelApprovalHintConfirm(id);
        approvalUiAdapter.setApprovalVisible(id, true);
      }
      return;
    }
  }

  // scanBuffer は active セッションのみ（非アクティブは pendingTextTail の末尾で代替）。
  const seqPending = (t?.pendingTextTail || '').split(/\r\n|\r|\n/).slice(-80).map(l => stripAnsi(l));
  const seqLines = (t?.everAttached && id === activeSessionId)
    ? seqPending.concat(scanBuffer(id).slice(-80))
    : seqPending;
  const seqPrompts = extractSequentialChoicePrompts(seqLines);
  const seqState = getSequentialChoiceState(id, seqPrompts);
  if (seqState) {
    const seqOpts = sequentialChoiceOptionsForState(seqState);
    approvalUiAdapter.cacheApprovalOptions(id, seqOpts);
    const wasVisible = !!approvalVisibleCache.get(id);
    approvalUiAdapter.showOptions(bar, id, seqOpts, !wasVisible);
    if (!wasVisible) {
      cancelApprovalHintConfirm(id);
      approvalUiAdapter.setApprovalVisible(id, true);
    }
    return;
  }
  if (!seqPrompts) clearSequentialChoiceState(id);

  if (isGoNativeApprovalActive(id)) {
    const cached = approvalRawOptionsCache.get(id);
    if (cached && cached.length > 0) {
      approvalUiAdapter.showOptions(bar, id, cached);
      return;
    }
  }

  // フォールバック検出: pendingTextTail を使う（scanBuffer は履歴を保持するため
  // hideActionBar 後も古い選択肢を再検出してしまう）
  const tail = (t ? t.pendingTextTail || '' : '').split(/\r\n|\r|\n/).slice(-120).map(l => stripAnsi(l));

  // フォールバック検出（既存）
  let extraction = extractApprovalOptions(tail);
  const options = extraction.options;
  // cluster の index は展開後の行（Ink 連結を再分割した結果）が基準。
  // approvalContextLines で同じ配列を使わないと index ずれでコンテキストを取り違える。
  let contextSourceLines = extraction.lines || tail;
  let contextCluster = extraction.cluster;
  // ターミナル高さぶんの空白行 + 余裕60行を確保（ダイアログが画面上部にある場合に備える）
  const visibleRows = t?.term?.rows || 40;
  // Ink.js（Claude Code）はカーソル位置制御シーケンスで各行を描画するため \r\n がなく、
  // pendingTextTail の split で全テキストが1行に連結されて options が空になるか、
  // "Yes2.No" のように選択肢が連結されて1行に結合されることがある（concat artifact）。
  // xterm バッファ（行分割済み）がより多くの選択肢を持つ場合はそちらを優先する。
  // ただし scanBuffer は履歴を保持し続けるため、承認解決後の応答チャンクで
  // 古い選択肢が再検出される（approvalConsumedSig は label 差異で抑止が外れる）。
  // pendingTextTail に承認系の手がかりが無く、かつ既に visible でも無い場合は scanBuffer を見ない。
  // active セッションのみ scanBuffer を呼ぶ（非アクティブは pendingTextTail で代替）。
  const pendingHasApprovalHint = approvalLinesHaveHint(provider, tail);
  const allowBufferFallback = approvalVisibleCache.get(id) || options.length > 0 || pendingHasApprovalHint;
  const bufferTail = (t && allowBufferFallback && id === activeSessionId)
    ? scanBuffer(id).slice(-Math.max(120, visibleRows + 60))
    : [];
  if (t && bufferTail.length > 0) {
    const bufExtraction = extractApprovalOptions(bufferTail);
    const bufOpts = bufExtraction.options;
    const pendingHasCursor = options.some(o => o.isCurrent);
    const bufHasCursor = bufOpts.some(o => o.isCurrent);
    if (bufOpts.length > options.length || (!pendingHasCursor && bufHasCursor && bufOpts.length >= options.length)) {
      options.length = 0;
      options.push(...bufOpts);
      options.sort((a, b) => a.num - b.num);
      contextSourceLines = bufExtraction.lines || bufferTail;
      contextCluster = bufExtraction.cluster;
    }
  }

  // pendingTextTail は保持上限があるため option 1 が欠落することがある。
  // 2択プロンプト（Yes/No）では option 2 だけ pendingTextTail に入り options.length=1 のまま
  // 補完が発動しないケースも含め、option 1 が未検出なら常に xterm バッファで補完する。
  if (t && options.length >= 1 && !options.some(o => o.num === 1)) {
    const maxNum = Math.max(...options.map(o => o.num));
    const bufExtraction = extractApprovalOptions(bufferTail);
    const bufOpts = bufExtraction.options;
    if (bufOpts.length > 0 && Math.max(...bufOpts.map(o => o.num)) === maxNum) {
      for (const bo of bufOpts) {
        if (!options.some(o => o.num === bo.num)) options.push(bo);
      }
      options.sort((a, b) => a.num - b.num);
      contextSourceLines = bufExtraction.lines || bufferTail;
      contextCluster = bufExtraction.cluster;
    }
  }

  const contextLines = approvalContextLines(contextSourceLines, contextCluster);
  markHubChoiceDefault(options, contextLines);
  const lastOpt = options[options.length - 1];
  const hasUserSpecifies = (lastOpt && isUserSpecifiesText(lastOpt.label)) || contextLines.some(line => isUserSpecifiesText(line));
  const hasCursorOption = options.some(o => o.isCurrent);
  const approvalLabelRe = /\b(yes|no|allow|deny|proceed|abort|don[''']t ask|cancel)\b/i;
  const hasApprovalLikeLabel = options.some((opt) => approvalLabelRe.test(opt.label));
  const isHubChoice = isHubChoicePrompt(contextLines, options);
  const suppressCodexModelSelector = isCodexModelSelectorContext(provider, contextLines);
  const hasNativePromptHint = !suppressCodexModelSelector && contextLines.some((line) => !String(line || '').toLowerCase().includes('esc to go back') && (matchProviderApprovalTrigger(provider, line) || matchNativeApprovalTrigger(line)));
  const isShortcutApprovalMenu = (provider === 'codex' || provider === 'copilot' || provider === 'cursor-agent' || provider === 'opencode') && options.some(o => o._sendText) && hasNativePromptHint;
  const approvalNear = (hasApprovalLikeLabel &&
    (hasUserSpecifies || hasNativePromptHint)) || isHubChoice || isShortcutApprovalMenu;
  const hasApproval = options.length > 0 && approvalNear && (hasCursorOption || isShortcutApprovalMenu);
  const hasChoiceMenu = (hasCursorOption || isShortcutApprovalMenu) && options.length > 0 && hasNativePromptHint;
  const hasPrompt = hasApproval || hasChoiceMenu;
  // 承認ではないカーソル駆動の選択メニュー（claude /model 等）にタグ付けし、
  // action-bar にメニュータイトルを出す。承認は常に優先（hasApproval が真なら対象外）。
  const isSelectMenu = hasChoiceMenu && !hasApproval && hasCursorOption && !isShortcutApprovalMenu;
  tagSelectMenuOptions(options, isSelectMenu, contextSourceLines, contextCluster);

  if (!hasPrompt) {
    // 承認プロンプトが検出できない場合は確実に閉じる。
    // ただし、approvalVisibleCache=true かつ cache が残っている場合は、
    // pendingTextTail のローテート（長考時に [MANY-AI-CLI] マーカーが押し出される）や
    // 一時的なフォールバック検出失敗で action-bar を誤って消さないよう、
    // cache から action-bar を復元する（H9: 非対称スタック対策 — plan_action-bar-not-showing.md §7.1）。
    // sendChoice / doSend / closeBtn は hideActionBar を直接呼ぶため、ここの復元経路は通らない。
    // 解決済み承認の残留は approvalConsumedSig（sendChoice/doSend で sig 保存）で抑止される。
    if (approvalVisibleCache.get(id)) {
      const cached = approvalRawOptionsCache.get(id);
      if (cached && cached.length > 0) {
        // C3: 復元前に scanBuffer 妥当性を検証する（保留中バッジ固着対応）。
        // active セッションでキャッシュ済み選択肢が描画済み行から消えた状態が
        // 連続 H9_RESTORE_MISS_LIMIT 回続いたら、ターミナル直接入力で解決済みと
        // みなして復元せず閉じる（→ setApprovalVisible(false) が Hub に届く）。
        // 非アクティブセッションは scanBuffer を読めないため従来通り復元する
        // （誤維持しても Hub 側 approvalVisibleLease が最終回復する）。
        let stillOnScreen = true;
        if (id === activeSessionId && t?.everAttached) {
          const lines = bufferTail.length > 0
            ? bufferTail
            : scanBuffer(id).slice(-Math.max(120, visibleRows + 60));
          stillOnScreen = cachedOptionsOnScreen(lines, cached);
        }
        if (stillOnScreen) {
          h9RestoreMisses.delete(id);
          cancelH9Revalidate(id);
          approvalUiAdapter.showOptions(bar, id, cached);
          return;
        }
        const misses = (h9RestoreMisses.get(id) || 0) + 1;
        if (misses < H9_RESTORE_MISS_LIMIT) {
          h9RestoreMisses.set(id, misses);
          // C5: PTY 出力が静止していてもカウンタが上限まで進むよう自前で再評価を予約する。
          // （detectApproval はチャンク駆動のため、解決直後に出力が止まると
          //   ここで止まったまま復元ループが固着していた）
          scheduleH9Revalidate(id);
          approvalUiAdapter.showOptions(bar, id, cached);
          return;
        }
        h9RestoreMisses.delete(id);
        cancelH9Revalidate(id);
      }
    }
    hideActionBar(id);
    return;
  }

  // doSend / sendChoice で消費済みの選択肢が xterm scanBuffer に残っているため
  // フォールバック検出で再抽出されるケースを抑止する（marker 検出と同じ debounce 戦略）。
  if (hasPrompt) {
    const consumed = approvalConsumedSig.get(id);
    const sig = approvalSig(options);
    if (consumed === sig) {
      const prev = approvalConsumedSigDeleteTimer.get(id);
      if (prev) clearTimeout(prev);
      approvalConsumedSigDeleteTimer.set(id, setTimeout(() => {
        approvalConsumedSig.delete(id);
        approvalConsumedSigDeleteTimer.delete(id);
      }, 5000));
      hideActionBar(id);
      return;
    }
  }

  // Anti-flicker: 既に別の選択肢を表示中の場合、700ms 安定して検出されるまで切り替えを保留する。
  // Codex の Ink 再描画で pendingTextTail に複数レンダリング状態が混入し、
  // extractApprovalOptions が poll ごとに異なる選択肢を返すことでチカチカする問題への対処。
  if (hasPrompt && !isBatchOptions(options) && approvalVisibleCache.get(id)) {
    const existingCached = approvalRawOptionsCache.get(id);
    if (existingCached && existingCached.length > 0 && !isBatchOptions(existingCached)) {
      const newSig = approvalSig(options);
      const oldSig = approvalSig(existingCached);
      if (newSig !== oldSig) {
        const candidate = approvalSwitchCandidates.get(id);
        if (!candidate || candidate.sig !== newSig) {
          approvalSwitchCandidates.set(id, { sig: newSig, options: options.slice(), firstSeenAt: Date.now() });
          showActionBar(bar, id, existingCached, false);
          return;
        }
        if (Date.now() - candidate.firstSeenAt < 700) {
          showActionBar(bar, id, existingCached, false);
          return;
        }
        // 700ms 安定 — 新しい選択肢に切り替える
        approvalSwitchCandidates.delete(id);
        approvalUiAdapter.cacheApprovalOptions(id, options);
      } else {
        approvalSwitchCandidates.delete(id);
      }
    } else {
      approvalSwitchCandidates.delete(id);
    }
  }

  // 実プロンプト検出に成功したら H9 復元の連続ミスカウンタをリセットする
  h9RestoreMisses.delete(id);
  cancelH9Revalidate(id);

  const wasVisibleBeforeShow = !!approvalVisibleCache.get(id);
  showActionBar(bar, id, options, !wasVisibleBeforeShow);

  // session_hint: 承認 UI の可視状態を Hub に通知
  const nowVisible = hasPrompt;
  if (nowVisible !== !!approvalVisibleCache.get(id)) {
    if (nowVisible) cancelApprovalHintConfirm(id);
    approvalUiAdapter.setApprovalVisible(id, nowVisible);
  }
}

export function getActionBarButtons() {
  const bar = document.getElementById('action-bar');
  if (!bar) return [];
  return Array.from(bar.querySelectorAll('.action-btn'));
}

export function setActionBarFocus(idx) {
  set_actionBarFocusIdx(idx);
  getActionBarButtons().forEach((btn, i) => btn.classList.toggle('kbd-focus', i === idx));
}

export function hideActionBar(id) {
  const bar = document.getElementById('action-bar');
  const wasVisible = !!(bar && bar.classList.contains('visible'));
  if (bar) { bar.classList.remove('visible', 'batch', 'multi-select', 'single-tabs'); bar.innerHTML = ''; }
  // action-bar 出現時に設定した PTY リサイズ抑制を解除する。
  // これにより消滅後のターミナル拡大に対して ResizeObserver が
  // 正しい行数で SIGWINCH を1回だけ送れるようになる。
  if (wasVisible) clearSuppressPtyResize();
  // 差分スキップ用キャッシュをリセット（次回 showActionBar が同一シグネチャでも再描画されるように）
  lastActionBarRender.sessionId = null;
  lastActionBarRender.sig = null;
  set_actionBarFocusIdx(-1);
  set_batchFocusIdx(-1);
  set_multiSelectFocusIdx(-1);
  if (id !== undefined) actionBarShownAt.delete(id);
  if (id !== undefined) batchSelections.delete(id);
  if (id !== undefined) batchFreeText.delete(id);
  if (id !== undefined) batchActiveQ.delete(id);
  if (id !== undefined) multiSelectSelections.delete(id);
  removeBatchConfirmModal();
  if (id !== undefined) {
    manualHideSig.delete(id); // 承認が解決したら手動抑制も解除する
    cancelApprovalHintConfirm(id);
    approvalSwitchCandidates.delete(id);
    h9RestoreMisses.delete(id);
    cancelH9Revalidate(id);
    resetBgApprovalMisses(id);
    clearSequentialChoiceState(id);
    clearSingleTabState(id);
    const wasVisible = !!approvalVisibleCache.get(id);
    if (wasVisible) {
      approvalUiAdapter.setApprovalVisible(id, false);
    }
    // approvalRawOptionsCache / pendingTextTail のクリアは、approvalVisibleCache が
    // true → false へ実際に遷移する時（= sendChoice / doSend / closeBtn / detectApproval が
    // cache 復元できず最終的に閉じた時）のみ実行する。
    // wasVisible=false で呼ばれた場合は race 条件 or 重複呼び出しなので、cache を保護する。
    // （H9: 非対称スタック対策 — plan_action-bar-not-showing.md §7.1）
    if (wasVisible) {
      approvalUiAdapter.clearApprovalOptions(id);
      approvalSourceCache.delete(id);
      const t = terminals.get(id);
      if (t) t.pendingTextTail = '';
    }
    // approvalConsumedSig は debounce 型タイマーで削除する。
    // trackApprovalHintFromChunk が同一ブロックを再検出するたびにタイマーをリセットし、
    // ブロックが届かなくなってから 5 秒後に削除する。
    // ここでは Ink が再描画を開始する前の初期タイマーを 10 秒で設定する（フォールバック）。
    const prevTimer = approvalConsumedSigDeleteTimer.get(id);
    if (prevTimer) clearTimeout(prevTimer);
    approvalConsumedSigDeleteTimer.set(id, setTimeout(() => {
      approvalConsumedSig.delete(id);
      approvalConsumedSigDeleteTimer.delete(id);
    }, 10000));
    // action-bar 消失でターミナル領域の高さが拡張されるため、追従中なら最下部へ再スナップする。
    // showActionBar が plan_approval-bar-scroll-resnap.md で同等の処理を持つので、その対称ケース。
    if (wasVisible && id === activeSessionId) {
      const term = terminals.get(id);
      const shouldStickToBottom = !!(term && (term.autoScroll || isTerminalAtBottom(term)));
      if (shouldStickToBottom) scrollTerminalToBottomSoon(id);
    }
    maybeAutoSwitchToNextApproval();
  }
}

// 「✕ 承認」（消す）: action-bar の描画だけを一時的に消す。
// approvalRawOptionsCache / approvalVisibleCache はあえて保持し、承認は保留のまま（waiting も残す）。
// 「↻ 承認」（reshowActionBar）で元に戻せる。承認が解決すれば hideActionBar が manualHideSig を消す。
export function manuallyHideActionBar(id) {
  if (id === undefined || id === null) return;
  const cached = approvalRawOptionsCache.get(id);
  if (Array.isArray(cached) && cached.length > 0) {
    manualHideSig.set(id, approvalSig(cached));
  }
  const bar = document.getElementById('action-bar');
  const wasVisible = !!(bar && bar.classList.contains('visible'));
  if (bar) { bar.classList.remove('visible', 'batch', 'multi-select', 'single-tabs'); bar.innerHTML = ''; }
  // action-bar 出現時の PTY リサイズ抑制を解除（ターミナルが正しい行数へ拡張できるように）
  if (wasVisible) clearSuppressPtyResize();
  lastActionBarRender.sessionId = null;
  lastActionBarRender.sig = null;
  set_actionBarFocusIdx(-1);
  set_batchFocusIdx(-1);
  set_multiSelectFocusIdx(-1);
  // ターミナル領域が拡張されるので追従中なら最下部へ再スナップ（hideActionBar と対称）。
  if (wasVisible && id === activeSessionId) {
    const term = terminals.get(id);
    const shouldStickToBottom = !!(term && (term.autoScroll || isTerminalAtBottom(term)));
    if (shouldStickToBottom) scrollTerminalToBottomSoon(id);
  }
}

// 「↻ 承認」（再表示）: 手動抑制を解除して通常の検出を再実行する。
// detectApproval が pendingTextTail / cache / scanBuffer から再判定するため、
// 本当に保留中の承認だけが復活し、すでに解決済みなら何も表示されない（安全な再判定）。
export function reshowActionBar(id) {
  if (id === undefined || id === null) return;
  manualHideSig.delete(id);
  detectApproval(id);
}

export function normalizeActionOptions(options) {
  const byNum = new Map();
  for (const opt of options || []) {
    if (!opt || typeof opt.num !== 'number') continue;
    const prev = byNum.get(opt.num);
    if (!prev) {
      byNum.set(opt.num, { ...opt });
      continue;
    }
    // 同一番号が再描画ノイズで重複した場合は、現在選択中フラグと長いラベルを優先する。
    byNum.set(opt.num, {
      ...prev,
      isCurrent: !!(prev.isCurrent || opt.isCurrent),
      preserveOrder: !!(prev.preserveOrder || opt.preserveOrder),
      label: (opt.label && opt.label.length > (prev.label || '').length) ? opt.label : prev.label,
    });
  }
  const normalized = Array.from(byNum.values());
  if (normalized.some(opt => opt.preserveOrder)) return normalized;
  return normalized.sort((a, b) => a.num - b.num);
}

// 折りたたみトグル（全文⇄コンパクト切替）。3 経路（単問/一括/複数選択）共通で使う。
// position:absolute なので bar 直下に append すれば footer の有無に関わらず同じ位置に出る。
function appendCollapseToggle(bar, sessionId) {
  const btn = document.createElement('button');
  btn.className = 'action-collapse-btn';
  const collapsed = isActionBarCollapsed();
  btn.textContent = collapsed ? '⊞' : '⊟';
  btn.title = collapsed ? t('action_bar_expand') : t('action_bar_collapse');
  btn.onclick = (e) => {
    e.stopPropagation();
    toggleActionBarCollapsed(sessionId);
  };
  bar.appendChild(btn);
}

export function toggleActionBarCollapsed(sessionId) {
  setActionBarCollapsed(!isActionBarCollapsed());
  const bar = document.getElementById('action-bar');
  if (!bar) return;
  // collapsed を各 show* の sig に含めているため、再描画させるには sig を無効化する。
  lastActionBarRender.sessionId = null;
  lastActionBarRender.sig = null;
  const cached = approvalRawOptionsCache.get(sessionId);
  if (cached && cached.length > 0) {
    showActionBar(bar, sessionId, cached, false);
  } else {
    bar.classList.toggle('collapsed', isActionBarCollapsed());
  }
  setTimeout(() => inputEl.focus(), 0);
}

export function showActionBar(bar, sessionId, options, forceStickToBottom = false) {
  // 手動「✕ 承認」で抑制中は描画しない。同一承認のみ抑制し、別 sig の新しい承認は抑制解除して描画する。
  const suppressedSig = manualHideSig.get(sessionId);
  if (suppressedSig !== undefined) {
    if (suppressedSig === approvalSig(options)) return;
    manualHideSig.delete(sessionId);
  }
  if (isBatchOptions(options)) {
    showBatchActionBar(bar, sessionId, options, forceStickToBottom);
    return;
  }
  if (isMultiSelectOptions(options)) {
    showMultiSelectActionBar(bar, sessionId, options, forceStickToBottom);
    return;
  }
  // 単一質問（YES/NO・単一選択・選択メニュー・順次質問）も質問タブUI（1タブ）へ統合する。
  // plan_choice-tab-ui.md C5。flat options を 1 セクション（_single）へ変換し、
  // 既存の showBatchActionBar に同じ params 形で渡す（描画は同関数の単一モード分岐で行う）。
  // approvalRawOptionsCache は flat のまま保持する（sendChoice の _sendText 等が機能するため）。
  const opts = normalizeActionOptions(options);
  const sequentialQuestion = opts.find(o => o && o._sequentialQuestion)?._sequentialQuestion;
  const menuTitle = opts.find(o => o && o._menuTitle)?._menuTitle;
  const isSelectMenu = opts.some(o => o && o._selectMenu);
  // [MANY-AI-CLI] 単一ブロックの先頭にあった質問文（parseHubBlock が配列プロパティ _question に格納）。
  // これを承認ポップアップ内に表示し、AI が何を聞いているのかを CLI 画面を見ずに把握できるようにする。
  const hubQuestion = (options && (options as any)._question) ? String((options as any)._question).trim() : '';
  const title = sequentialQuestion
    ? sequentialQuestion
    : (isSelectMenu ? (menuTitle || 'Select an option') : (hubQuestion || 'Approval needed'));
  const section = {
    num: 1,
    title,
    options: opts,
    _single: true,
    _freeInput: !!(options && (options as any)._freeInput),
    _labelKind: sequentialQuestion ? 'sequential' : (isSelectMenu ? 'select-menu' : 'approval'),
    _labelTitle: sequentialQuestion || menuTitle || '',
    // 質問本文（承認系のみ；sequential/select-menu は title 側で表示済み）。
    _question: hubQuestion,
    // 配列プロパティの _preamble は [section] 変換で失われるためセクションへも引き継ぐ。
    _preamble: (options && (options as any)._preamble) ? (options as any)._preamble : '',
  };
  // 単一質問は [section] の 1 要素配列へ変換するため、配列プロパティの _preamble が
  // 失われる。明示的に引き継いでポップアップ先頭の前置き表示を効かせる。
  const sectionArr: any[] = [section];
  if (options && (options as any)._preamble) (sectionArr as any)._preamble = (options as any)._preamble;
  showBatchActionBar(bar, sessionId, sectionArr, forceStickToBottom);
}

// 単一質問タブUI（plan_choice-tab-ui.md C5）の自由入力状態。バッチの batchSelections/
// batchFreeText とは別管理（単一は selections 機構を使わず即送信のため）。
const singleFreeText = new Map<number, string>();   // sessionId → 自由入力テキスト（再描画跨ぎで保持）
const singleFreeActive = new Map<number, boolean>(); // sessionId → 自由入力欄を開いているか

function clearSingleTabState(id) {
  singleFreeText.delete(id);
  singleFreeActive.delete(id);
}

// 自由入力欄を開く（再描画して入力欄を出しフォーカスする）。
export function activateSingleFree(sessionId) {
  singleFreeActive.set(sessionId, true);
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!Array.isArray(cached) || isBatchOptions(cached) || isMultiSelectOptions(cached)) return;
  const bar = document.getElementById('action-bar');
  if (bar) showActionBar(bar, sessionId, cached); // router 経由で 1 セクションへ再変換
}

// 自由入力テキストをそのまま送信する（確認モーダルなし）。送信後は UI を消して会話へ戻る。
export function sendSingleFreeText(sessionId) {
  const text = (singleFreeText.get(sessionId) || '').trim();
  if (!text) return;
  const cachedOpts = approvalRawOptionsCache.get(sessionId);
  chatHistoryCommitOutput(sessionId);
  pushMessage(sessionId, {
    role: 'system',
    kind: 'approval',
    rawText: text,
    meta: { kind: 'single', answer: null, label: text },
  });
  if (cachedOpts) approvalConsumedSig.set(sessionId, approvalSig(cachedOpts));
  recordAnsweredMarkerSig(sessionId, cachedOpts);
  sendApprovalConsumed(sessionId, cachedOpts, `${text}\r`);
  sendSubmittedText(sessionId, `${text}\r`);
  hideActionBar(sessionId);
  approvalSuppressUntil.set(sessionId, Date.now() + 400);
  multiQuestionDismissedCache.delete(sessionId);
  setTimeout(() => {
    detectApproval(sessionId);
    maybeAutoSwitchToNextApproval();
  }, 450);
  setTimeout(() => inputEl.focus(), 0);
}

// ---- 一括承認: 質問タブUI（plan_choice-tab-ui.md）----
// 質問そのものを横タブにし、選択中の 1 問分の選択肢だけを下のパネルに出す。
// 質問数が増えても縦の高さが一定で、上のターミナル（文脈）が隠れない。
// 全問回答後に「送信確認」→ モーダルで内容＋実送信文字列を確認 →「送信」で確定する。

const BATCH_FREE = -1; // 自由入力肢を選択中であることを示す selections センチネル

// アクティブな質問タブ index を範囲内に正規化して返す（未設定/範囲外は 0）。
function getBatchActiveQ(sessionId, n) {
  let idx = batchActiveQ.get(sessionId);
  if (idx == null || idx < 0 || idx >= n) { idx = 0; batchActiveQ.set(sessionId, idx); }
  return idx;
}

// 承認ブロック直前の前置き説明（_preamble）をポップアップ先頭へ表示する。
// 既定はヘッダ＋高さ上限つきスクロール枠で文脈を即見せ（CLI をスクロールしなくて済む）、
// ヘッダクリックで枠を広げて全文を読めるようにする（'expanded' クラスのトグル）。
function appendApprovalPreamble(bar, preamble) {
  const text = String(preamble || '').trim();
  if (!text) return;
  const wrap = document.createElement('div');
  wrap.className = 'action-preamble';
  const head = document.createElement('button');
  head.type = 'button';
  head.className = 'action-preamble-head';
  head.textContent = t('approval_preamble_label');
  head.onclick = (e) => { e.stopPropagation(); wrap.classList.toggle('expanded'); };
  const body = document.createElement('div');
  body.className = 'action-preamble-body';
  body.textContent = text;
  wrap.appendChild(head);
  wrap.appendChild(body);
  bar.appendChild(wrap);
}

// タブ/選択肢ボタンの圧縮表示テキスト。shortLabel を優先し、無ければ label 先頭を
// 全角8字で自動短縮する（最終的な伸縮は CSS の max-width + ellipsis が担う）。
function batchShortText(opt) {
  if (opt && opt.shortLabel) return String(opt.shortLabel);
  const s = String((opt && opt.label) || '').trim();
  return s.length > 8 ? s.slice(0, 8) + '…' : s;
}

function batchSectionAnswered(sessionId, idx) {
  const selections = batchSelections.get(sessionId);
  if (!selections) return false;
  const sel = selections[idx];
  if (sel == null) return false;
  if (sel === BATCH_FREE) {
    const ft = batchFreeText.get(sessionId) || [];
    return (ft[idx] || '').trim().length > 0;
  }
  return true;
}

function batchAllAnswered(sessionId, sections) {
  if (!sections || sections.length === 0) return false;
  for (let i = 0; i < sections.length; i++) {
    if (!batchSectionAnswered(sessionId, i)) return false;
  }
  return true;
}

// 単一質問モードの描画（showBatchActionBar から呼ばれる分岐実体・plan_choice-tab-ui.md C5）。
// タブは常に1つ。選択肢ボタン押下で確認モーダルを挟まず sendChoice で即送信する（1クリック）。
// 単一質問は横幅に余裕があるので短ラベル圧縮・詳細パネルは使わず、ボタンに全文を表示する
// （プレビュー不要で 1 クリック・誤クリック防止）。自由入力肢（「N. User specifies」）は
// 入力欄を開き、Enter で入力テキストをそのまま送る。
function showSingleSectionBar(bar, sessionId, section, ctx) {
  const { shouldStickToBottom, chatTlB, chatWasAtBottomB, forceStickToBottom } = ctx;
  const options = (section.options || []);
  const allowFree = !!section._freeInput;
  const title = section.title || 'Approval needed';
  const labelKind = section._labelKind || 'approval';
  const labelTitle = section._labelTitle || '';
  // 承認系（Yes/No・AI の番号付き選択肢）の質問本文。sequential/select-menu は title 側に出すため除外。
  const question = (labelKind === 'approval' && section._question) ? String(section._question).trim() : '';
  const preamble = section._preamble ? String(section._preamble).trim() : '';
  const freeActive = allowFree && !!singleFreeActive.get(sessionId);

  const sig = JSON.stringify({
    s: sessionId,
    mode: 'single-tabs',
    title,
    lk: labelKind,
    q: question,
    pre: preamble,
    opts: options.map(o => ({ n: o.num, l: o.label, c: !!o.isCurrent, p: !!o.preserveOrder })),
    free: allowFree,
    fa: freeActive,
    v: bar.classList.contains('visible'),
    col: isActionBarCollapsed(),
  });
  if (lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig) {
    if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
    if (chatWasAtBottomB && chatTlB) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlB));
    return;
  }
  lastActionBarRender.sessionId = sessionId;
  lastActionBarRender.sig = sig;
  bar.innerHTML = '';
  bar.classList.remove('batch', 'multi-select');
  bar.classList.add('single-tabs');
  batchSelections.delete(sessionId);
  multiSelectSelections.delete(sessionId);

  if (options.length === 0 && !allowFree) return;

  // ===== ラベル =====
  const label = document.createElement('span');
  label.className = 'action-bar-label';
  if (labelKind === 'sequential') {
    label.textContent = `⚠ ${labelTitle || title}`;
    label.classList.add('sequential-question-label');
    label.title = labelTitle || title;
  } else if (labelKind === 'select-menu') {
    label.textContent = labelTitle ? `📋 ${labelTitle}` : '📋 Select an option';
    label.classList.add('select-menu-label');
    if (labelTitle) label.title = labelTitle;
  } else {
    label.textContent = '⚠ Approval needed';
  }
  bar.appendChild(label);

  // 承認ブロック直前の地の文（前置き説明）があれば先頭に表示する（バッチ/複数選択と対称）。
  // 単一質問だけここが抜けていたため、AI の質問文・文脈がポップアップに出ず CLI 画面を
  // 見ないと内容が分からなかった。
  appendApprovalPreamble(bar, preamble);

  // ===== パネル（全文選択肢 + 自由入力欄） =====
  // 単一質問モードでは質問タブ列（常に1タブで無意味）と質問見出しを出さない。
  // 質問文/承認文言は上の黄色ヘッダー（action-bar-label）に集約済みで、見出し・タブに
  // 同じ文字列を重ねると「Approval needed」が 3 連発して冗長になるため（plan_choice-tab-ui.md C5 改）。
  const pane = document.createElement('div');
  pane.className = 'action-qpane';

  // AI の質問本文を選択肢の上に全文表示する（折り返し可・バッチの action-qhead と同じ見た目）。
  // ラベル（黄色ヘッダ）は単行省略のため、長い質問はここで読めるようにする。
  if (question) {
    const qhead = document.createElement('div');
    qhead.className = 'action-qhead action-single-qhead';
    qhead.textContent = question;
    qhead.title = question;
    pane.appendChild(qhead);
  }

  // "Yes, and" 系（セッション全体許可）があればそれを推奨扱いにする（既存ロジック踏襲）。
  const isSessionAllowLabel = (s) => /during this session|allow.*session|yes.*allow/i.test(s);
  const hasSessionAllow = options.some(o => isSessionAllowLabel(o.label));
  const isRecommendedOpt = (o) => hasSessionAllow ? isSessionAllowLabel(o.label) : o.isCurrent;

  // 選択肢ボタンは全文表示（短ラベル圧縮なし）。.action-btn の通常スタイルで折り返す
  //（詳細パネルでのプレビューが不要になり、見て 1 クリックで送れる）。
  const optsEl = document.createElement('div');
  optsEl.className = 'action-qopts action-qopts-full';
  for (const opt of options) {
    const btn = document.createElement('button');
    const isPermanent = /don[''']t ask again/i.test(opt.label);
    const isSessionAllow = isSessionAllowLabel(opt.label);
    let cls = 'action-btn';
    if (isSessionAllow) cls += ' session-allow';
    else if (isRecommendedOpt(opt)) cls += ' current';
    if (isPermanent) cls += ' permanent';
    btn.className = cls;
    btn.textContent = `${opt.num}. ${opt.label}` + (isRecommendedOpt(opt) ? ` (${t('approval_recommended')})` : '');
    btn.title = `${opt.num}. ${opt.label}`;
    btn.onclick = () => sendChoice(sessionId, opt.num); // 即送信（確認モーダルなし）
    optsEl.appendChild(btn);
  }

  // 自由入力肢（あれば）。クリックで入力欄を開き、入力後 Enter で即送信する。
  if (allowFree) {
    const fbtn = document.createElement('button');
    fbtn.className = 'action-btn' + (freeActive ? ' current' : '');
    fbtn.textContent = `N. ${t('approval_free_input')}`;
    fbtn.title = t('approval_free_input');
    fbtn.onclick = (e) => { e.stopPropagation(); activateSingleFree(sessionId); };
    optsEl.appendChild(fbtn);
  }
  pane.appendChild(optsEl);

  // 自由入力中だけ入力欄を出す（詳細パネルは廃止）。
  // 「N. 自由入力」ボタンの直後（optsEl 内）に入れて同じ行の右側に並べる。
  // pane 下へ縦積みすると余分な行高が増えるため、横並びで高さを抑える。
  if (freeActive) {
    const inp = document.createElement('input');
    inp.className = 'action-qfreein action-qfreein-inline';
    inp.type = 'text';
    inp.placeholder = t('approval_free_input_placeholder');
    inp.value = singleFreeText.get(sessionId) || '';
    inp.oninput = () => singleFreeText.set(sessionId, inp.value);
    inp.onkeydown = (e) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        e.stopPropagation();
        sendSingleFreeText(sessionId);
      }
    };
    optsEl.appendChild(inp);
    setTimeout(() => inp.focus(), 0);
  }
  bar.appendChild(pane);

  // 手動閉じボタン（誤検出時に消すため）
  const closeBtn = document.createElement('button');
  closeBtn.className = 'action-dismiss-btn';
  closeBtn.textContent = '✕';
  closeBtn.title = t('dismiss_title');
  closeBtn.onclick = (e) => {
    e.stopPropagation();
    hideActionBar(sessionId);
    approvalSuppressUntil.set(sessionId, Date.now() + 60000);
  };
  bar.appendChild(closeBtn);
  appendCollapseToggle(bar, sessionId);
  bar.classList.toggle('collapsed', isActionBarCollapsed());

  if (!bar.classList.contains('visible')) suppressPtyResizeForInputLayout(60000);
  bar.classList.add('visible');
  actionBarShownAt.set(sessionId, Date.now());
  if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
  if (chatWasAtBottomB && chatTlB) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlB));
}

export function showBatchActionBar(bar, sessionId, sections, forceStickToBottom = false) {
  const term = sessionId === activeSessionId ? terminals.get(sessionId) : null;
  const shouldStickToBottom = !!(term && (forceStickToBottom || term.autoScroll || isTerminalAtBottom(term)));
  const chatTlB = getChatTimelineEl();
  const chatWasAtBottomB = chatTlB ? chatPaneAtBottom(chatTlB) : false;

  // 単一質問モード（plan_choice-tab-ui.md C5）: showActionBar が flat options を
  // [{_single:true,...}] 1 セクションへ変換して渡す。バッチと同じタブ/パネル/詳細/自由入力の
  // 見た目を踏襲しつつ、タブは常に1つ・確認モーダルを出さず即送信する点だけが異なる。
  if (sections.length === 1 && (sections[0] as any)._single) {
    showSingleSectionBar(bar, sessionId, sections[0], { shouldStickToBottom, chatTlB, chatWasAtBottomB, forceStickToBottom });
    return;
  }

  // セクション数が変わったら選択状態・自由入力をリセット（前回セレクションの持ち越し防止）
  let selections = batchSelections.get(sessionId);
  if (!selections || selections.length !== sections.length) {
    selections = new Array(sections.length).fill(null);
    batchSelections.set(sessionId, selections);
  }
  let freeTexts = batchFreeText.get(sessionId);
  if (!freeTexts || freeTexts.length !== sections.length) {
    freeTexts = new Array(sections.length).fill('');
    batchFreeText.set(sessionId, freeTexts);
  }
  const activeQ = getBatchActiveQ(sessionId, sections.length);

  const sig = JSON.stringify({
    s: sessionId,
    mode: 'batch-tabs',
    sects: sections.map(sec => ({
      n: sec.num, t: sec.title, f: !!sec._freeInput,
      o: (sec.options || []).map(o => ({ n: o.num, l: o.label, s: o.shortLabel || '', c: !!o.isCurrent })),
    })),
    sel: selections,
    ft: freeTexts,
    aq: activeQ,
    v: bar.classList.contains('visible'),
    col: isActionBarCollapsed(),
  });
  if (lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig) {
    if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
    if (chatWasAtBottomB && chatTlB) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlB));
    return;
  }
  lastActionBarRender.sessionId = sessionId;
  lastActionBarRender.sig = sig;
  bar.innerHTML = '';
  bar.classList.remove('single-tabs', 'multi-select');
  bar.classList.add('batch');

  const label = document.createElement('span');
  label.className = 'action-bar-label';
  label.textContent = t('approval_batch_label', { n: sections.length });
  bar.appendChild(label);

  // 承認ブロック直前の地の文（前置き説明）があれば先頭に表示する。
  appendApprovalPreamble(bar, (sections as any)._preamble);

  // ===== 質問タブ列（横スクロール・✓/未 ステータス付き） =====
  const tabsEl = document.createElement('div');
  tabsEl.className = 'action-qtabs';
  const statusEls: any[] = [];
  sections.forEach((sec, idx) => {
    const tab = document.createElement('button');
    tab.className = 'action-qtab' + (idx === activeQ ? ' active' : '');
    tab.onclick = (e) => { e.stopPropagation(); setBatchActiveQ(sessionId, idx); };
    const qn = document.createElement('span');
    qn.className = 'qn';
    // 質問番号は `Q1` 表記（選択肢番号 `1.` `2.` と視覚的に区別する）
    qn.textContent = `Q${idx + 1}`;
    const txt = document.createElement('span');
    txt.className = 'qlabel';
    txt.textContent = sec.title;
    txt.title = sec.title;
    const st = document.createElement('span');
    st.className = 'st';
    statusEls[idx] = st;
    tab.appendChild(qn); tab.appendChild(txt); tab.appendChild(st);
    tabsEl.appendChild(tab);
  });
  bar.appendChild(tabsEl);

  // ===== アクティブ質問のパネル（選択肢 + 詳細 + 自由入力） =====
  const pane = document.createElement('div');
  pane.className = 'action-qpane';
  const activeSec = sections[activeQ];

  const head = document.createElement('div');
  head.className = 'action-qhead';
  head.textContent = `Q${activeQ + 1} ${activeSec.title}`;
  head.title = activeSec.title;
  pane.appendChild(head);

  // 選択肢（+ 自由入力肢）
  const choices = (activeSec.options || []).slice();
  if (activeSec._freeInput) {
    choices.push({ num: BATCH_FREE, label: t('approval_free_input_full'), shortLabel: t('approval_free_input'), _free: true });
  }

  const optsEl = document.createElement('div');
  optsEl.className = 'action-qopts';
  choices.forEach((opt) => {
    const btn = document.createElement('button');
    let cls = 'action-btn batch-option action-qopt';
    if (opt.isCurrent) cls += ' current';
    if (selections[activeQ] === opt.num) cls += ' selected';
    btn.className = cls;
    const nEl = document.createElement('span');
    nEl.className = 'n';
    nEl.textContent = opt._free ? 'N' : `${opt.num}`;
    const lEl = document.createElement('span');
    lEl.className = 'opt-label';
    lEl.textContent = batchShortText(opt);
    btn.appendChild(nEl); btn.appendChild(lEl);
    btn.title = opt._free ? t('approval_free_input') : `${opt.num}. ${opt.label}`;
    btn.onclick = (e) => { e.stopPropagation(); selectBatchOption(sessionId, activeQ, opt.num); };
    optsEl.appendChild(btn);
  });
  pane.appendChild(optsEl);

  // 詳細パネル（選択中の全文 / 自由入力欄）
  const detail = document.createElement('div');
  const sel = selections[activeQ];
  // status/progress/submit を入力中に再構築せず更新する closure（後段で定義）
  let updateBatchStatus = () => {};
  if (sel != null) {
    const opt = sel === BATCH_FREE
      ? { num: BATCH_FREE, label: t('approval_free_input_full'), _free: true } as any
      : choices.find((o) => o.num === sel);
    if (opt) {
      detail.className = 'action-qdetail';
      const lab = document.createElement('span');
      lab.className = 'detail-lab';
      lab.textContent = opt._free ? t('approval_free_input') : `${opt.num}. ${batchShortText(opt)}`;
      detail.appendChild(lab);
      if (!opt._free) {
        const body = document.createElement('div');
        body.className = 'detail-body';
        body.textContent = opt.label + (opt.isCurrent ? ` (${t('approval_recommended')})` : '');
        detail.appendChild(body);
      } else {
        const inp = document.createElement('input');
        inp.className = 'action-qfreein';
        inp.type = 'text';
        inp.placeholder = t('approval_free_input_placeholder');
        inp.value = freeTexts[activeQ] || '';
        inp.oninput = () => {
          freeTexts[activeQ] = inp.value;
          // 入力中は full rebuild せずステータス/進捗/送信のみ更新（フォーカス維持）
          updateBatchStatus();
        };
        detail.appendChild(inp);
        setTimeout(() => inp.focus(), 0);
      }
    } else {
      detail.className = 'action-qdetail empty';
      detail.textContent = t('approval_batch_detail_empty');
    }
  } else {
    detail.className = 'action-qdetail empty';
    detail.textContent = t('approval_batch_detail_empty');
  }
  // ===== 詳細メッセージ＋送信バーを横並びに =====
  // 左に「送信確認 / クリア」、右にメッセージ詳細を置く。縦積みの送信バー行を無くし、
  // 承認ポップアップの高さを抑える（CLI 表示欄を広く保ち、メッセージ確認をしやすくする）。
  const detailRow = document.createElement('div');
  detailRow.className = 'action-qdetail-row';

  const actions = document.createElement('div');
  actions.className = 'action-qdetail-actions';

  const progress = document.createElement('span');
  progress.className = 'action-bar-progress';
  actions.appendChild(progress);

  const actionBtns = document.createElement('div');
  actionBtns.className = 'action-qdetail-btns';

  const submitBtn = document.createElement('button');
  submitBtn.className = 'action-submit-btn';
  submitBtn.textContent = t('approval_batch_confirm');
  submitBtn.onclick = (e) => { e.stopPropagation(); openBatchConfirm(sessionId); };
  actionBtns.appendChild(submitBtn);

  const clearBtn = document.createElement('button');
  clearBtn.className = 'action-clear-btn';
  clearBtn.textContent = t('approval_batch_clear');
  clearBtn.onclick = (e) => { e.stopPropagation(); clearBatchSelections(sessionId); };
  actionBtns.appendChild(clearBtn);

  actions.appendChild(actionBtns);
  detailRow.appendChild(actions);
  detailRow.appendChild(detail);
  pane.appendChild(detailRow);
  bar.appendChild(pane);

  // 閉じ（✕）は position:absolute なので bar 直下に置けば右上に固定表示される。
  const closeBatchBtn = document.createElement('button');
  closeBatchBtn.className = 'action-dismiss-btn';
  closeBatchBtn.textContent = '✕';
  closeBatchBtn.title = t('dismiss_title');
  closeBatchBtn.onclick = (e) => {
    e.stopPropagation();
    hideActionBar(sessionId);
    approvalSuppressUntil.set(sessionId, Date.now() + 60000);
  };
  bar.appendChild(closeBatchBtn);

  // タブ ✓/未・進捗・送信ボタン活性を一括更新（自由入力の oninput からも呼ぶ）
  updateBatchStatus = () => {
    let done = 0;
    sections.forEach((sec, idx) => {
      const ok = batchSectionAnswered(sessionId, idx);
      if (ok) done++;
      const st = statusEls[idx];
      if (st) { st.textContent = ok ? '✓' : '未'; st.className = 'st ' + (ok ? 'done' : 'todo'); }
    });
    progress.textContent = t('approval_batch_progress', { done, total: sections.length });
    submitBtn.disabled = done < sections.length;
  };
  updateBatchStatus();

  appendCollapseToggle(bar, sessionId);
  bar.classList.toggle('collapsed', isActionBarCollapsed());
  if (!bar.classList.contains('visible')) suppressPtyResizeForInputLayout(60000);
  bar.classList.add('visible');
  actionBarShownAt.set(sessionId, Date.now());
  if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
  if (chatWasAtBottomB && chatTlB) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlB));
}

export function setBatchActiveQ(sessionId, idx) {
  batchActiveQ.set(sessionId, idx);
  const cached = approvalRawOptionsCache.get(sessionId);
  if (isBatchOptions(cached)) {
    const bar = document.getElementById('action-bar');
    if (bar) showBatchActionBar(bar, sessionId, cached);
  }
}

export function selectBatchOption(sessionId, sectionIdx, optionNum) {
  const selections = batchSelections.get(sessionId);
  if (!selections) return;
  selections[sectionIdx] = optionNum;
  batchActiveQ.set(sessionId, sectionIdx); // 選んだ質問をアクティブに保つ（自動で別タブに飛ばさない）
  const cached = approvalRawOptionsCache.get(sessionId);
  if (isBatchOptions(cached)) {
    const bar = document.getElementById('action-bar');
    if (bar) showBatchActionBar(bar, sessionId, cached);
  }
  // 自由入力肢は再描画後に入力欄へフォーカスする（showBatchActionBar 内）。それ以外は本体入力へ。
  if (optionNum !== BATCH_FREE) setTimeout(() => inputEl.focus(), 0);
}

export function clearBatchSelections(sessionId) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isBatchOptions(cached)) return;
  batchSelections.set(sessionId, new Array(cached.length).fill(null));
  batchFreeText.set(sessionId, new Array(cached.length).fill(''));
  batchActiveQ.set(sessionId, 0);
  const bar = document.getElementById('action-bar');
  if (bar) showBatchActionBar(bar, sessionId, cached);
  setTimeout(() => inputEl.focus(), 0);
}

// 実送信文字列を組み立てる。各行「質問番号 選択肢番号」。自由入力の行は入力テキストを送る。
// 選択肢番号はエージェントが提示した実番号（ボタン表示と一致）をそのまま使う
// （1始まり位置への変換はしない。グローバル連番の場合に表示・回答・解釈がずれるため）。
function buildBatchPayload(sessionId) {
  const selections = batchSelections.get(sessionId) || [];
  const freeTexts = batchFreeText.get(sessionId) || [];
  return selections.map((sel, idx) =>
    sel === BATCH_FREE ? `${idx + 1} ${(freeTexts[idx] || '').trim()}` : `${idx + 1} ${sel}`
  ).join('\n');
}

// 確認モーダル用の人が読む形（質問タイトル＋選んだラベル/全文、自由入力は入力テキスト）。
function buildBatchReadable(sessionId, sections) {
  const selections = batchSelections.get(sessionId) || [];
  const freeTexts = batchFreeText.get(sessionId) || [];
  return sections.map((sec, idx) => {
    const sel = selections[idx];
    let val;
    if (sel === BATCH_FREE) {
      val = `${t('approval_free_input')}「${(freeTexts[idx] || '').trim()}」`;
    } else {
      const opt = (sec.options || []).find((o) => o.num === sel);
      val = opt ? `${opt.shortLabel ? opt.shortLabel + ' — ' : ''}${opt.label}` : `${sel}`;
    }
    return `Q${idx + 1} ${sec.title}\n   → ${val}`;
  }).join('\n');
}

function removeBatchConfirmModal() {
  const m = document.getElementById('action-confirm-mask');
  if (m && m.parentNode) m.parentNode.removeChild(m);
}

// 「送信確認」: 全問回答済みのときだけ、内容＋実送信文字列を確認するモーダルを開く。
export function openBatchConfirm(sessionId) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isBatchOptions(cached)) return;
  if (!batchAllAnswered(sessionId, cached)) return;
  removeBatchConfirmModal();

  const mask = document.createElement('div');
  mask.className = 'action-confirm-mask';
  mask.id = 'action-confirm-mask';
  const modal = document.createElement('div');
  modal.className = 'action-confirm-modal';

  const h = document.createElement('h3');
  h.textContent = t('approval_confirm_title');
  modal.appendChild(h);

  const p1 = document.createElement('p');
  p1.textContent = t('approval_confirm_readable_label');
  modal.appendChild(p1);
  const readable = document.createElement('div');
  readable.className = 'action-confirm-readable';
  readable.textContent = buildBatchReadable(sessionId, cached);
  modal.appendChild(readable);

  const p2 = document.createElement('p');
  p2.textContent = t('approval_confirm_payload_label');
  modal.appendChild(p2);
  const payload = document.createElement('div');
  payload.className = 'action-confirm-payload';
  payload.textContent = buildBatchPayload(sessionId);
  modal.appendChild(payload);

  const row = document.createElement('div');
  row.className = 'action-confirm-row';
  const back = document.createElement('button');
  back.className = 'action-confirm-back';
  back.textContent = t('approval_confirm_back');
  back.onclick = (e) => { e.stopPropagation(); removeBatchConfirmModal(); setTimeout(() => inputEl.focus(), 0); };
  const go = document.createElement('button');
  go.className = 'action-confirm-go';
  go.textContent = t('approval_confirm_send');
  go.onclick = (e) => { e.stopPropagation(); removeBatchConfirmModal(); sendBatchChoices(sessionId); };
  row.appendChild(back); row.appendChild(go);
  modal.appendChild(row);

  mask.appendChild(modal);
  // 背景クリックで閉じる（戻る相当）
  mask.onclick = (e) => { if (e.target === mask) { removeBatchConfirmModal(); setTimeout(() => inputEl.focus(), 0); } };
  document.body.appendChild(mask);
  setTimeout(() => go.focus(), 0);
}

// 確定送信（モーダルの「送信」から呼ぶ）。送信後は完了メッセージを出さず UI を消して会話に戻る。
export function sendBatchChoices(sessionId) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isBatchOptions(cached)) return;
  if (!batchAllAnswered(sessionId, cached)) return;
  const text = buildBatchPayload(sessionId);
  approvalConsumedSig.set(sessionId, approvalSig(cached));
  recordAnsweredMarkerSig(sessionId, cached);
  sendApprovalConsumed(sessionId, cached, text);
  sendSubmittedText(sessionId, `${text}\r`);
  removeBatchConfirmModal();
  hideActionBar(sessionId);
  approvalSuppressUntil.set(sessionId, Date.now() + 400);
  batchSelections.delete(sessionId);
  batchFreeText.delete(sessionId);
  batchActiveQ.delete(sessionId);
  multiQuestionDismissedCache.delete(sessionId);
  setTimeout(() => {
    detectApproval(sessionId);
    maybeAutoSwitchToNextApproval();
  }, 450);
  setTimeout(() => inputEl.focus(), 0);
}

export function isBatchActionBarVisible() {
  const bar = document.getElementById('action-bar');
  return !!(bar && bar.classList.contains('visible') && bar.classList.contains('batch'));
}

// 質問タブの移動（Tab / ←→）。タブを巡回するだけで選択は変えない。
export function moveBatchFocus(delta) {
  if (activeSessionId === null) return false;
  const cached = approvalRawOptionsCache.get(activeSessionId);
  if (!isBatchOptions(cached) || cached.length === 0) return false;
  const n = cached.length;
  const cur = getBatchActiveQ(activeSessionId, n);
  setBatchActiveQ(activeSessionId, ((cur + delta) % n + n) % n);
  return true;
}

export function handleBatchNumberKey(sessionId, num) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isBatchOptions(cached)) return false;
  const idx = getBatchActiveQ(sessionId, cached.length);
  const section = cached[idx];
  if (!section) return false;
  const opt = (section.options || []).find(o => o.num === num);
  if (!opt) return false;
  selectBatchOption(sessionId, idx, num);
  return true;
}

// ---- 複数選択（#multi）: 1 問で任意個 ON/OFF できるチェックボックス UI ----

export function showMultiSelectActionBar(bar, sessionId, options, forceStickToBottom = false) {
  const term = sessionId === activeSessionId ? terminals.get(sessionId) : null;
  const shouldStickToBottom = !!(term && (forceStickToBottom || term.autoScroll || isTerminalAtBottom(term)));
  const chatTlM = getChatTimelineEl();
  const chatWasAtBottomM = chatTlM ? chatPaneAtBottom(chatTlM) : false;

  let selected = multiSelectSelections.get(sessionId);
  if (!selected) {
    selected = new Set();
    multiSelectSelections.set(sessionId, selected);
    if (multiSelectFocusIdx < 0 || multiSelectFocusIdx >= options.length) set_multiSelectFocusIdx(0);
  }
  const question = (options[0] && options[0]._question) || '';

  const sig = JSON.stringify({
    s: sessionId,
    mode: 'multi',
    q: question,
    opts: options.map(o => ({ n: o.num, l: o.label })),
    sel: Array.from(selected).sort((a, b) => a - b),
    f: multiSelectFocusIdx,
    v: bar.classList.contains('visible'),
    col: isActionBarCollapsed(),
  });
  if (lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig) {
    if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
    if (chatWasAtBottomM && chatTlM) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlM));
    return;
  }
  lastActionBarRender.sessionId = sessionId;
  lastActionBarRender.sig = sig;
  bar.innerHTML = '';
  bar.classList.remove('batch', 'single-tabs');
  bar.classList.add('multi-select');

  const label = document.createElement('span');
  label.className = 'action-bar-label';
  label.textContent = question ? `⚠ ${question}` : t('approval_multi_label');
  if (question) label.title = question;
  bar.appendChild(label);

  // 承認ブロック直前の地の文（前置き説明）があれば先頭に表示する。
  appendApprovalPreamble(bar, (options as any)._preamble);

  const btnRow = document.createElement('div');
  btnRow.className = 'action-section-buttons';
  options.forEach((opt, idx) => {
    const btn = document.createElement('button');
    let cls = 'action-btn multi-option';
    const checked = selected.has(opt.num);
    if (checked) cls += ' selected';
    if (idx === multiSelectFocusIdx) cls += ' kbd-focus';
    btn.className = cls;
    btn.textContent = `${checked ? '☑' : '☐'} ${opt.num}. ${opt.label}`;
    btn.title = `${opt.num}. ${opt.label}`;
    btn.onclick = (e) => {
      e.stopPropagation();
      set_multiSelectFocusIdx(idx);
      toggleMultiSelectOption(sessionId, opt.num);
    };
    btnRow.appendChild(btn);
  });
  bar.appendChild(btnRow);

  const footer = document.createElement('div');
  footer.className = 'action-bar-footer';

  const progress = document.createElement('span');
  progress.className = 'action-bar-progress';
  progress.textContent = t('approval_multi_progress', { n: selected.size });
  footer.appendChild(progress);

  const submitBtn = document.createElement('button');
  submitBtn.className = 'action-submit-btn';
  submitBtn.textContent = t('approval_batch_submit');
  submitBtn.disabled = selected.size === 0;
  submitBtn.onclick = (e) => {
    e.stopPropagation();
    sendMultiSelectChoices(sessionId);
  };
  footer.appendChild(submitBtn);

  const clearBtn = document.createElement('button');
  clearBtn.className = 'action-clear-btn';
  clearBtn.textContent = t('approval_batch_clear');
  clearBtn.onclick = (e) => {
    e.stopPropagation();
    clearMultiSelectSelections(sessionId);
  };
  footer.appendChild(clearBtn);

  const closeMultiBtn = document.createElement('button');
  closeMultiBtn.className = 'action-dismiss-btn';
  closeMultiBtn.textContent = '✕';
  closeMultiBtn.title = t('dismiss_title');
  closeMultiBtn.onclick = (e) => {
    e.stopPropagation();
    hideActionBar(sessionId);
    approvalSuppressUntil.set(sessionId, Date.now() + 60000);
  };
  footer.appendChild(closeMultiBtn);

  bar.appendChild(footer);
  appendCollapseToggle(bar, sessionId);
  bar.classList.toggle('collapsed', isActionBarCollapsed());
  if (!bar.classList.contains('visible')) suppressPtyResizeForInputLayout(60000);
  bar.classList.add('visible');
  actionBarShownAt.set(sessionId, Date.now());
  if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
  if (chatWasAtBottomM && chatTlM) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlM));
}

export function isMultiSelectActionBarVisible() {
  const bar = document.getElementById('action-bar');
  return !!(bar && bar.classList.contains('visible') && bar.classList.contains('multi-select'));
}

export function toggleMultiSelectOption(sessionId, num) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isMultiSelectOptions(cached)) return false;
  if (!cached.some(o => o.num === num)) return false;
  let selected = multiSelectSelections.get(sessionId);
  if (!selected) { selected = new Set(); multiSelectSelections.set(sessionId, selected); }
  if (selected.has(num)) selected.delete(num);
  else selected.add(num);
  const bar = document.getElementById('action-bar');
  if (bar) showMultiSelectActionBar(bar, sessionId, cached);
  setTimeout(() => inputEl.focus(), 0);
  return true;
}

export function clearMultiSelectSelections(sessionId) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isMultiSelectOptions(cached)) return;
  multiSelectSelections.set(sessionId, new Set());
  set_multiSelectFocusIdx(0);
  const bar = document.getElementById('action-bar');
  if (bar) showMultiSelectActionBar(bar, sessionId, cached);
  setTimeout(() => inputEl.focus(), 0);
}

export function moveMultiSelectFocus(delta) {
  if (activeSessionId === null) return false;
  const cached = approvalRawOptionsCache.get(activeSessionId);
  if (!isMultiSelectOptions(cached) || cached.length === 0) return false;
  const n = cached.length;
  const start = multiSelectFocusIdx < 0 ? (delta > 0 ? -1 : 0) : multiSelectFocusIdx;
  set_multiSelectFocusIdx(((start + delta) % n + n) % n);
  const bar = document.getElementById('action-bar');
  if (bar) showMultiSelectActionBar(bar, activeSessionId, cached);
  return true;
}

// フォーカス中の選択肢を Space でトグルする
export function toggleMultiSelectFocused(sessionId) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isMultiSelectOptions(cached)) return false;
  if (multiSelectFocusIdx < 0 || multiSelectFocusIdx >= cached.length) return false;
  const opt = cached[multiSelectFocusIdx];
  if (!opt) return false;
  return toggleMultiSelectOption(sessionId, opt.num);
}

export function handleMultiSelectNumberKey(sessionId, num) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isMultiSelectOptions(cached)) return false;
  const idx = cached.findIndex(o => o.num === num);
  if (idx < 0) return false;
  set_multiSelectFocusIdx(idx);
  return toggleMultiSelectOption(sessionId, num);
}

export function sendMultiSelectChoices(sessionId) {
  const selected = multiSelectSelections.get(sessionId);
  if (!selected || selected.size === 0) return;
  const prevOpts = approvalRawOptionsCache.get(sessionId);
  const nums = Array.from(selected).sort((a, b) => a - b);
  // 選択番号をカンマ連結で返す（例 "1,3"）。エージェントが提示した実番号をそのまま使う。
  const text = nums.join(',');
  const labelMap = new Map((isMultiSelectOptions(prevOpts) ? prevOpts : []).map(o => [o.num, o.label]));
  chatHistoryCommitOutput(sessionId);
  pushMessage(sessionId, {
    role: 'system',
    kind: 'approval',
    rawText: nums.map(n => labelMap.get(n) || `#${n}`).join(', '),
    meta: {
      kind: 'multi',
      answers: nums,
      labels: nums.map(n => labelMap.get(n) || null),
    },
  });
  if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
  recordAnsweredMarkerSig(sessionId, prevOpts);
  sendApprovalConsumed(sessionId, prevOpts, text);
  multiQuestionDismissedCache.delete(sessionId);
  multiQuestionLatchAt.delete(sessionId);
  sendSubmittedText(sessionId, `${text}\r`);
  hideActionBar(sessionId);
  approvalSuppressUntil.set(sessionId, Date.now() + 400);
  multiSelectSelections.delete(sessionId);
  setTimeout(() => {
    detectApproval(sessionId);
    maybeAutoSwitchToNextApproval();
  }, 450);
  setTimeout(() => inputEl.focus(), 0);
}

export function sendChoice(sessionId, targetNum) {
  const seqState = sequentialChoiceCache.get(sessionId);
  if (seqState && seqState.index < seqState.prompts.length) {
    const prompt = seqState.prompts[seqState.index];
    seqState.answers.set(prompt.key, targetNum);
    seqState.index++;
    while (seqState.index < seqState.prompts.length && seqState.answers.has(seqState.prompts[seqState.index].key)) {
      seqState.index++;
    }

    const bar = document.getElementById('action-bar');
    if (seqState.index < seqState.prompts.length && bar) {
      const nextOpts = sequentialChoiceOptionsForState(seqState);
      approvalUiAdapter.cacheApprovalOptions(sessionId, nextOpts);
      approvalUiAdapter.showOptions(bar, sessionId, nextOpts);
      setTimeout(() => inputEl.focus(), 0);
      return;
    }

    const response = seqState.prompts
      .map(p => `${p.key}: ${seqState.answers.get(p.key)}`)
      .join('\n') + '\r';
    // chatHistory: 複数質問への一括回答を system/approval として push
    chatHistoryCommitOutput(sessionId);
    pushMessage(sessionId, {
      role: 'system',
      kind: 'approval',
      rawText: response.replace(/\r$/, ''),
      meta: {
        kind: 'batch',
        answers: seqState.prompts.map(p => ({
          key: p.key,
          question: p.question,
          answer: seqState.answers.get(p.key),
        })),
      },
    });
    clearSequentialChoiceState(sessionId);
    const prevOpts = approvalRawOptionsCache.get(sessionId);
    if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
    recordAnsweredMarkerSig(sessionId, prevOpts);
    multiQuestionDismissedCache.delete(sessionId);
    multiQuestionLatchAt.delete(sessionId);
    sendSubmittedText(sessionId, response);
    hideActionBar(sessionId);
    approvalSuppressUntil.set(sessionId, Date.now() + 400);
    setTimeout(() => {
      detectApproval(sessionId);
      maybeAutoSwitchToNextApproval();
    }, 450);
    setTimeout(() => inputEl.focus(), 0);
    return;
  }

  // 矢印移動ではなく番号直接入力で確定する（誤選択防止）
  const cachedOpts = approvalRawOptionsCache.get(sessionId);
  const targetOpt = Array.isArray(cachedOpts) && !isBatchOptions(cachedOpts)
    ? cachedOpts.find(o => o && o.num === targetNum)
    : null;
  const choiceText = targetOpt && targetOpt._sendText ? targetOpt._sendText : `${targetNum}\r`;
  // chatHistory: 単問への回答を system/approval として push
  chatHistoryCommitOutput(sessionId);
  pushMessage(sessionId, {
    role: 'system',
    kind: 'approval',
    rawText: targetOpt ? (targetOpt.label || `#${targetNum}`) : `#${targetNum}`,
    meta: {
      kind: 'single',
      answer: targetNum,
      label: targetOpt ? (targetOpt.label || null) : null,
    },
  });
  // doSend と同様に消費済み署名を記録（Ink 再描画による同一ブロックの再検出・再表示を防ぐ）
  const prevOpts = cachedOpts;
  if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
  recordAnsweredMarkerSig(sessionId, prevOpts);
  sendApprovalConsumed(sessionId, prevOpts, choiceText);
  sendSubmittedText(sessionId, choiceText);
  hideActionBar(sessionId);
  // PTY エコーバックによる誤再表示を短時間抑制（approvalConsumedSig が同一選択肢の再検出を防ぐため短くてよい）
  approvalSuppressUntil.set(sessionId, Date.now() + 400);
  // suppress 解除後に pendingTextTail を再スキャン（suppress 中に届いた次の承認を検出するため）
  setTimeout(() => {
    detectApproval(sessionId);
    maybeAutoSwitchToNextApproval();
  }, 450);
  setTimeout(() => inputEl.focus(), 0);
}

// 承認ポップアップ 再表示/消す ボタン（↓ down の左の余白に常時表示）。
// type=module は defer 実行のため DOM は構築済み。
document.getElementById('approval-reshow-btn')?.addEventListener('click', () => {
  if (activeSessionId === null) return;
  reshowActionBar(activeSessionId);
});
document.getElementById('approval-dismiss-btn')?.addEventListener('click', () => {
  if (activeSessionId === null) return;
  manuallyHideActionBar(activeSessionId);
});

// --- ESM window-interop publish (generated; preserves dynamic window.* lookups) ---
window.matchProviderApprovalTrigger = matchProviderApprovalTrigger;
