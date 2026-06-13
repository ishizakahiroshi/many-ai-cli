// --- ESM imports (generated) ---
import { t } from '../i18n.js';
import { APPROVAL_PENDING_TEXT_TAIL_LIMIT, actionBarShownAt, activeSessionId, approvalConsumedSig, approvalConsumedSigDeleteTimer, approvalHintConfirmTimers, approvalHintConfirmTrusted, approvalRawOptionsCache, approvalSig, approvalSourceCache, approvalSuppressUntil, approvalSwitchCandidates, approvalVisibleCache, batchFocusIdx, batchSelections, lastActionBarRender, maybeAutoSwitchToNextApproval, multiQuestionDismissedCache, multiQuestionLatchAt, multiQuestionVisibleCache, multiSelectFocusIdx, multiSelectSelections, sequentialChoiceCache, sequentialChoiceSig, sessions, set_actionBarFocusIdx, set_batchFocusIdx, set_multiSelectFocusIdx, terminals, utf8Decoder } from './state.js';
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
// の約2秒周期チカチカになる（Claude の [ANY-AI-CLI] 一括質問で発生）。
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

// provider 別の承認 trigger phrase は ~/.any-ai-cli/approval-patterns/{provider}.json に外出し。
// Hub 起動時にデフォルトをユーザー設定ディレクトリに展開（既存ファイルは尊重）し、
// HTTP 経由で配信する。ユーザーが直接編集して文言を追加・調整できる。
// claude / codex は英語固定（Anthropic/OpenAI が国際化していない）、common は多言語混在。
export const providerApprovalTriggers = { claude: [], codex: [], copilot: [], 'cursor-agent': [], common: [] };

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
  const [claude, codex, copilot, cursorAgent, common] = await Promise.all([
    fetchJson('claude'), fetchJson('codex'), fetchJson('copilot'), fetchJson('cursor-agent'), fetchJson('common'),
  ]);
  const norm = arr => (Array.isArray(arr) ? arr : []).map(s => String(s).toLowerCase()).filter(Boolean);
  providerApprovalTriggers.claude = norm(claude);
  providerApprovalTriggers.codex  = norm(codex);
  providerApprovalTriggers.copilot = norm(copilot);
  providerApprovalTriggers['cursor-agent'] = norm(cursorAgent);
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
    // 連続して [ANY-AI-CLI] ブロックが来た場合 (例: 1質問目を回答せず 2質問目が来た) は
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

  // フォーマットベース検出（優先）: [ANY-AI-CLI] マーカーがあれば即確定
  const markerOpts = extractHubMarkerApproval(lines);
  if (markerOpts) {
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
  const isShortcutApprovalMenu = (provider === 'codex' || provider === 'copilot' || provider === 'cursor-agent') && options.some(o => o._sendText) && hasNativePromptHint;
  const approvalNear = (hasCursorOption || isShortcutApprovalMenu) &&
    ((hasApprovalLikeLabel && (hasUserSpecifies || contextLines.some((line) => matchProviderApprovalTrigger(provider, line) || matchNativeApprovalTrigger(line)))) || isHubChoice);
  const hasChoiceMenuHint = (hasCursorOption || isShortcutApprovalMenu) && options.length > 0 && hasNativePromptHint;
  const nowVisible = (options.length > 0 && approvalNear) || hasChoiceMenuHint;

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
  // sendChoice 直後の誤再表示を抑制
  const suppressUntil = approvalSuppressUntil.get(id);
  if (suppressUntil && Date.now() < suppressUntil) {
    scheduleApprovalSuppressRescan(id, suppressUntil);
    return;
  }
  approvalSuppressUntil.delete(id);

  const provider = sessions.get(id)?.provider;
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

  // [ANY-AI-CLI] マーカー検出: xterm バッファではなく pendingTextTail を使う。
  // xterm バッファは回答済みの古い [ANY-AI-CLI] ブロックを保持し続けるため、
  // suppress 期間が切れると再検出・再表示されてしまう。
  // pendingTextTail は hideActionBar でクリアされるが、Ink 再描画で同一内容が
  // 再び入ることがあるため approvalConsumedSig で二重表示を防ぐ。
  const t = terminals.get(id);
  if (t) {
    const pendingLines = (t.pendingTextTail || '').split(/\r\n|\r|\n/).slice(-40).map(l => stripAnsi(l));
    const markerOpts = extractHubMarkerApproval(pendingLines);
    if (markerOpts) {
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
  const isShortcutApprovalMenu = (provider === 'codex' || provider === 'copilot' || provider === 'cursor-agent') && options.some(o => o._sendText) && hasNativePromptHint;
  const approvalNear = (hasApprovalLikeLabel &&
    (hasUserSpecifies || hasNativePromptHint)) || isHubChoice || isShortcutApprovalMenu;
  const hasApproval = options.length > 0 && approvalNear && (hasCursorOption || isShortcutApprovalMenu);
  const hasChoiceMenu = (hasCursorOption || isShortcutApprovalMenu) && options.length > 0 && hasNativePromptHint;
  const hasPrompt = hasApproval || hasChoiceMenu;

  if (!hasPrompt) {
    // 承認プロンプトが検出できない場合は確実に閉じる。
    // ただし、approvalVisibleCache=true かつ cache が残っている場合は、
    // pendingTextTail のローテート（長考時に [ANY-AI-CLI] マーカーが押し出される）や
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
  if (bar) { bar.classList.remove('visible', 'batch'); bar.innerHTML = ''; }
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
  if (id !== undefined) multiSelectSelections.delete(id);
  if (id !== undefined) {
    cancelApprovalHintConfirm(id);
    approvalSwitchCandidates.delete(id);
    h9RestoreMisses.delete(id);
    cancelH9Revalidate(id);
    resetBgApprovalMisses(id);
    clearSequentialChoiceState(id);
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
  if (isBatchOptions(options)) {
    showBatchActionBar(bar, sessionId, options, forceStickToBottom);
    return;
  }
  if (isMultiSelectOptions(options)) {
    showMultiSelectActionBar(bar, sessionId, options, forceStickToBottom);
    return;
  }
  options = normalizeActionOptions(options);
  // 注意: 局所変数名 `t` は window.t（i18n 翻訳関数）と衝突するため使わない。
  // `term` にすることで本関数末尾の t('dismiss_title') 等が正しく i18n を参照できる。
  const term = sessionId === activeSessionId ? terminals.get(sessionId) : null;
  const shouldStickToBottom = !!(term && (forceStickToBottom || term.autoScroll || isTerminalAtBottom(term)));
  const chatTl = getChatTimelineEl();
  const chatWasAtBottom = chatTl ? chatPaneAtBottom(chatTl) : false;

  // 差分スキップ: 前回描画と同一シグネチャなら DOM を再構築しない（点滅防止）。
  // detectApproval は scheduleApprovalCheck 経由で 300ms ごとに走るため、
  // 内容が変わらない場合に bar.innerHTML を毎回作り直すとボタンが点滅する。
  // kbd-focus は外部の setActionBarFocus が触るので、ここでは options のみを sig に含める。
  const sig = JSON.stringify({
    s: sessionId,
    opts: options.map(o => ({ n: o.num, l: o.label, c: !!o.isCurrent, p: !!o.preserveOrder })),
    v: bar.classList.contains('visible'),
    col: isActionBarCollapsed(),
  });
  if (lastActionBarRender.sessionId === sessionId && lastActionBarRender.sig === sig) {
    // 承認検出は PTY write / ResizeObserver / セッション自動切替と同時に走ることがある。
    // DOM 再描画をスキップする場合でも、追従中なら最下部への再スナップは省略しない。
    if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
    if (chatWasAtBottom && chatTl) requestAnimationFrame(() => scrollChatPaneToBottom(chatTl));
    return;
  }
  lastActionBarRender.sessionId = sessionId;
  lastActionBarRender.sig = sig;
  bar.innerHTML = '';
  // バッチ→単一質問の遷移で残留する .batch クラスと選択状態を取り除く（縦スタック CSS の誤適用と
  // 後続バッチへの古いセレクション持ち越しを防ぐ）。
  bar.classList.remove('batch', 'multi-select');
  batchSelections.delete(sessionId);
  multiSelectSelections.delete(sessionId);

  // "⚠ Approval needed" ラベル
  if (options.length > 0) {
    const label = document.createElement('span');
    label.className = 'action-bar-label';
    const sequentialQuestion = options.find(o => o && o._sequentialQuestion)?._sequentialQuestion;
    label.textContent = sequentialQuestion ? `⚠ ${sequentialQuestion}` : '⚠ Approval needed';
    if (sequentialQuestion) {
      label.classList.add('sequential-question-label');
      label.title = sequentialQuestion;
    }
    bar.appendChild(label);
  }

  // "Yes, and" 系（セッション全体許可）が存在する場合はそちらを推奨扱いにする
  const hasSessionAllow = options.some(o => /during this session|allow.*session|yes.*allow/i.test(o.label));

  // 選択肢ボタン（左側）
  for (const opt of options) {
    const btn = document.createElement('button');
    const isPermanent = /don[''']t ask again/i.test(opt.label);
    const isSessionAllow = /during this session|allow.*session|yes.*allow/i.test(opt.label);
    const isRecommended = hasSessionAllow ? isSessionAllow : opt.isCurrent;
    let cls = 'action-btn';
    if (isSessionAllow) cls += ' session-allow';
    else if (isRecommended) cls += ' current';
    if (isPermanent) cls += ' permanent';
    btn.className = cls;
    btn.textContent = `${opt.num}. ${opt.label}`;
    btn.title = `${opt.num}. ${opt.label}`;
    btn.onclick = () => sendChoice(sessionId, opt.num);
    bar.appendChild(btn);
  }

  // 手動閉じボタン（誤検出時に消すため）— action-bar の右端に表示
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

  // action-bar 出現でターミナル高さが縮小し、ResizeObserver が SIGWINCH を
  // 送ると Claude Code が chrome を再描画して二重表示になる。
  // 初回表示時のみリサイズを抑制し、hideActionBar で解除することで
  // 消滅後の安定サイズで1回だけ SIGWINCH を送る。
  if (!bar.classList.contains('visible')) suppressPtyResizeForInputLayout(60000);
  bar.classList.add('visible');
  actionBarShownAt.set(sessionId, Date.now());
  if (shouldStickToBottom) {
    refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
  }
  if (chatWasAtBottom && chatTl) requestAnimationFrame(() => scrollChatPaneToBottom(chatTl));
}

export function showBatchActionBar(bar, sessionId, sections, forceStickToBottom = false) {
  const term = sessionId === activeSessionId ? terminals.get(sessionId) : null;
  const shouldStickToBottom = !!(term && (forceStickToBottom || term.autoScroll || isTerminalAtBottom(term)));
  const chatTlB = getChatTimelineEl();
  const chatWasAtBottomB = chatTlB ? chatPaneAtBottom(chatTlB) : false;

  // セクション数が変わったら選択状態をリセット（前回の sectionA→sectionB セレクションが残らないように）
  let selections = batchSelections.get(sessionId);
  if (!selections || selections.length !== sections.length) {
    selections = new Array(sections.length).fill(null);
    batchSelections.set(sessionId, selections);
    if (batchFocusIdx < 0 || batchFocusIdx >= sections.length) set_batchFocusIdx(0);
  }

  const sig = JSON.stringify({
    s: sessionId,
    mode: 'batch',
    sects: sections.map(sec => ({
      n: sec.num,
      t: sec.title,
      o: (sec.options || []).map(o => ({ n: o.num, l: o.label, c: !!o.isCurrent })),
    })),
    sel: selections,
    f: batchFocusIdx,
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
  bar.classList.add('batch');

  const label = document.createElement('span');
  label.className = 'action-bar-label';
  label.textContent = t('approval_batch_label', { n: sections.length });
  bar.appendChild(label);

  sections.forEach((sec, idx) => {
    const sectionEl = document.createElement('div');
    sectionEl.className = 'action-section';
    if (idx === batchFocusIdx) sectionEl.classList.add('focused');
    sectionEl.dataset.idx = String(idx);

    const titleEl = document.createElement('div');
    titleEl.className = 'action-section-title';
    titleEl.textContent = `${sec.num}. ${sec.title}`;
    titleEl.title = sec.title;
    sectionEl.appendChild(titleEl);

    const btnRow = document.createElement('div');
    btnRow.className = 'action-section-buttons';
    (sec.options || []).forEach((opt) => {
      const btn = document.createElement('button');
      let cls = 'action-btn batch-option';
      if (opt.isCurrent) cls += ' current';
      if (selections[idx] === opt.num) cls += ' selected';
      btn.className = cls;
      btn.textContent = `${opt.num}. ${opt.label}`;
      btn.title = `${opt.num}. ${opt.label}`;
      btn.onclick = (e) => {
        e.stopPropagation();
        selectBatchOption(sessionId, idx, opt.num);
      };
      btnRow.appendChild(btn);
    });
    sectionEl.appendChild(btnRow);
    bar.appendChild(sectionEl);
  });

  const footer = document.createElement('div');
  footer.className = 'action-bar-footer';

  const progress = document.createElement('span');
  progress.className = 'action-bar-progress';
  const done = selections.filter(v => v != null).length;
  progress.textContent = t('approval_batch_progress', { done, total: sections.length });
  footer.appendChild(progress);

  const submitBtn = document.createElement('button');
  submitBtn.className = 'action-submit-btn';
  submitBtn.textContent = t('approval_batch_submit');
  submitBtn.disabled = !selections.every(v => v != null);
  submitBtn.onclick = (e) => {
    e.stopPropagation();
    sendBatchChoices(sessionId);
  };
  footer.appendChild(submitBtn);

  const clearBtn = document.createElement('button');
  clearBtn.className = 'action-clear-btn';
  clearBtn.textContent = t('approval_batch_clear');
  clearBtn.onclick = (e) => {
    e.stopPropagation();
    clearBatchSelections(sessionId);
  };
  footer.appendChild(clearBtn);

  const closeBatchBtn = document.createElement('button');
  closeBatchBtn.className = 'action-dismiss-btn';
  closeBatchBtn.textContent = '✕';
  closeBatchBtn.title = t('dismiss_title');
  closeBatchBtn.onclick = (e) => {
    e.stopPropagation();
    hideActionBar(sessionId);
    approvalSuppressUntil.set(sessionId, Date.now() + 60000);
  };
  footer.appendChild(closeBatchBtn);

  bar.appendChild(footer);
  appendCollapseToggle(bar, sessionId);
  bar.classList.toggle('collapsed', isActionBarCollapsed());
  if (!bar.classList.contains('visible')) suppressPtyResizeForInputLayout(60000);
  bar.classList.add('visible');
  actionBarShownAt.set(sessionId, Date.now());
  if (shouldStickToBottom) refitAndStickTerminalToBottomSoon(sessionId, { force: forceStickToBottom });
  if (chatWasAtBottomB && chatTlB) requestAnimationFrame(() => scrollChatPaneToBottom(chatTlB));
}

export function selectBatchOption(sessionId, sectionIdx, optionNum) {
  const selections = batchSelections.get(sessionId);
  if (!selections) return;
  selections[sectionIdx] = optionNum;
  const cached = approvalRawOptionsCache.get(sessionId);
  if (isBatchOptions(cached)) {
    // 自動前進: 末尾セクションを選んだら -1（無効化）して Enter で送信可能にする
    set_batchFocusIdx(sectionIdx + 1 < cached.length ? sectionIdx + 1 : -1);
    const bar = document.getElementById('action-bar');
    if (bar) showBatchActionBar(bar, sessionId, cached);
  }
  setTimeout(() => inputEl.focus(), 0);
}

export function clearBatchSelections(sessionId) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isBatchOptions(cached)) return;
  batchSelections.set(sessionId, new Array(cached.length).fill(null));
  set_batchFocusIdx(0);
  const bar = document.getElementById('action-bar');
  if (bar) showBatchActionBar(bar, sessionId, cached);
  setTimeout(() => inputEl.focus(), 0);
}

export function sendBatchChoices(sessionId) {
  const selections = batchSelections.get(sessionId);
  if (!selections || selections.length === 0 || selections.some(v => v == null)) return;
  const prevOpts = approvalRawOptionsCache.get(sessionId);
  // 各行「質問番号 選択肢番号」で送る。選択肢番号はエージェントが提示した実番号
  // （ボタン表示と一致）をそのまま使い、1始まり位置への変換は行わない。
  // エージェントがグローバル連番（Q2が3,4等）を使った場合に変換すると、
  // 提示番号（3,4）と回答番号（1,2）がずれ、表示・回答・エージェント解釈の
  // 三者が食い違う。実番号を返せば、正しく1始まりのエージェントでも結果は同じ。
  const text = selections.map((sel, idx) => `${idx + 1} ${sel}`).join('\n');
  if (prevOpts) approvalConsumedSig.set(sessionId, approvalSig(prevOpts));
  sendApprovalConsumed(sessionId, prevOpts, text);
  sendSubmittedText(sessionId, `${text}\r`);
  hideActionBar(sessionId);
  approvalSuppressUntil.set(sessionId, Date.now() + 400);
  batchSelections.delete(sessionId);
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

export function moveBatchFocus(delta) {
  if (activeSessionId === null) return false;
  const cached = approvalRawOptionsCache.get(activeSessionId);
  if (!isBatchOptions(cached) || cached.length === 0) return false;
  const n = cached.length;
  const start = batchFocusIdx < 0 ? (delta > 0 ? -1 : n) : batchFocusIdx;
  set_batchFocusIdx(((start + delta) % n + n) % n);
  const bar = document.getElementById('action-bar');
  if (bar) showBatchActionBar(bar, activeSessionId, cached);
  return true;
}

export function handleBatchNumberKey(sessionId, num) {
  const cached = approvalRawOptionsCache.get(sessionId);
  if (!isBatchOptions(cached)) return false;
  if (batchFocusIdx < 0 || batchFocusIdx >= cached.length) return false;
  const section = cached[batchFocusIdx];
  if (!section) return false;
  const opt = (section.options || []).find(o => o.num === num);
  if (!opt) return false;
  selectBatchOption(sessionId, batchFocusIdx, num);
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
  bar.classList.remove('batch');
  bar.classList.add('multi-select');

  const label = document.createElement('span');
  label.className = 'action-bar-label';
  label.textContent = question ? `⚠ ${question}` : t('approval_multi_label');
  if (question) label.title = question;
  bar.appendChild(label);

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

// --- ESM window-interop publish (generated; preserves dynamic window.* lookups) ---
window.matchProviderApprovalTrigger = matchProviderApprovalTrigger;
