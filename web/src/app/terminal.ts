// --- ESM imports (generated) ---
import { cleanCopiedText, cleanOneLineText, showToast } from './util.js';
import { t as ti18n } from '../i18n.js';
import { FONTSIZE_MAP, STORAGE_FONTSIZE_KEY } from './user-prefs.js';
import { activeSessionId, approvalRawOptionsCache, approvalVisibleCache, sessions, terminals, utf8Decoder } from './state.js';
import { autoExpand, inputEl, sendText, updateInputClearButton } from '../app.js';
import { ABS_UNIX_PATH_RE, ABS_WIN_PATH_RE, REL_PATH_RE, isLikelyRelPath, isTerminalPathStartBoundary, resolveTerminalPathCandidate, scheduleHidePathPopup, showPathPopup, trimTerminalPathCandidate } from './path-links.js';
import { ws } from './ws-client.js';
import { scheduleApprovalCheck } from './approval.js';
import { handleCrunchLinkClick } from './expand-popup.js';
import { resetHistoryViewerForSessionChange, updateHistoryHint } from './history-viewer.js';

// Claude Code の折りたたみマーカー: "… +23 lines (ctrl+o to expand)"。
// サブエージェント実行行・ツール要約行は "+N lines" 無しで "(ctrl+o to expand)" 単独で
// 出るため（例: "Done (5 tool uses · 43.9k tokens · 32s)" 直下、"Read 2 files (ctrl+o to expand)"）、
// プレフィックスはオプションにして単独マーカーも拾う。
const CRUNCH_LINK_RE = /(?:[…\.]{1,3}\s*\+\d+\s*lines?\s*)?\(ctrl\+o to expand\)/gi;

// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- xterm.js 管理 ----

// 旧 app.js 時代は 5000 行、モジュール分割時に 2000 行へ縮んでいた。
// 長時間セッションで過去ターンが押し出される苦情を受けて 10000 行へ拡大。
// それ以前の出力は過去ログビューア（history-viewer.ts）で生ログから遡る。
export const TERMINAL_SCROLLBACK_LINES = 10000;
// 非アクティブセッションの未描画 PTY chunk は末尾だけ保持し、
// 長時間放置後のセッション切替で UI スレッドを詰まらせない。
export const TERMINAL_PENDING_MAX_BYTES = 100 * 1024;
export const TERMINAL_WRITE_FLUSH_WATCHDOG_MS = 5000;

async function copyTerminalSelectionText(text, opts: any = {}) {
  const cleaned = opts.oneLine ? cleanOneLineText(text) : cleanCopiedText(text);
  if (!cleaned) return;
  await navigator.clipboard.writeText(cleaned);
  showToast(ti18n('copied_to_clipboard'), opts.anchor);
}

// 選択範囲をクリップボード経由ではなく入力欄へ直接追記する。
// コピー → 貼り付けだと複数行は「Pasted text」添付になってしまうため、
// 添付化を避けて本文として入力したいケース向け。
function addSelectionToInput(text, opts: any = {}) {
  const cleaned = cleanCopiedText(text);
  if (!cleaned) return;
  inputEl.value = inputEl.value ? inputEl.value + '\n' + cleaned : cleaned;
  autoExpand({ suppressPtyResize: true });
  updateInputClearButton();
  inputEl.focus();
  showToast(ti18n('added_to_input'), opts.anchor);
}

// モーダル/オーバーレイ表示中は、ターミナル上でのホイール操作を xterm に処理させない。
// overscroll-behavior はスクロールコンテナのチェーンしか止められず、モーダルの非スクロール領域
// （タイトルバー・余白・背景）でのホイールは xterm が拾って背後の端末がスクロールしてしまう。
// xterm 側でホイールを無視させることで、どの経路で来ても背後の端末が動かないようにする。
function isModalOverlayOpen() {
  const ids = ['settings-panel', 'about-panel', 'model-picker-overlay', 'new-session-panel', 'slash-picker', 'expand-capture-popup', 'tsb-sent-modal'];
  for (const id of ids) {
    const el = document.getElementById(id);
    if (!el || el.hidden) continue;
    if (getComputedStyle(el).display !== 'none') return true;
  }
  // ファイルプレビューモーダル（path-links.ts）はクラスのみで id を持たないため別途検出する。
  if (document.querySelector('.aac-file-modal-overlay')) return true;
  return false;
}

export function ensureTerminal(id) {
  if (terminals.has(id)) return;
  const provider = sessions.get(id)?.provider;
  const term = new Terminal({
    cursorBlink: false,
    scrollback: TERMINAL_SCROLLBACK_LINES,
    // xterm はセル幅ベースで描画するため、絵文字フォント混在でグリフが巨大化/崩れする環境がある。
    // 端末領域は等幅フォントのみを使い、見た目の安定性を優先する。
    // 丸数字・ローマ数字（East Asian Ambiguous 幅）は "MS Gothic" 等の全角グリフで描き、
    // xterm 側の wcwidth を width=2 に上書きして 2 セル枠へ収める（下の unicode 設定を参照）。
    fontFamily: '"MS Gothic", "BIZ UDGothic", "BIZ UDゴシック", "Segoe UI Symbol", "Cascadia Mono", "Cascadia Code", Consolas, "Courier New", monospace',
    fontSize: FONTSIZE_MAP[localStorage.getItem(STORAGE_FONTSIZE_KEY)] || 13,
    // 一部フォントで大文字上端がクリップされるため、行高を少し広げて回避する。
    // さらに Claude Code TUI の進捗バー（1行ぶんの高さで描かれる黒帯）が細く見える問題の緩和も兼ねる。
    lineHeight: 1.35,
    windowsPty: { backend: 'conpty' },
    // cursor を背景色と同色にしてブロックカーソルを不可視化する。
    // 'transparent' はブラウザ実装によって輪郭線だけ □ として描画されることがある。
    theme: { background: '#0d1117', cursor: '#0d1117', cursorAccent: '#e6edf3' },
    disableStdin: true,
    allowProposedApi: true,
  });
  term.attachCustomKeyEventHandler((event) => {
    if (event.type !== 'keydown') return true;
    if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== 'c') return true;
    if (!term.hasSelection()) return true;
    copyTerminalSelectionText(term.getSelection()).catch(() => {});
    return false;
  });
  // モーダル表示中はホイールで背後の端末がスクロールしないよう、xterm にホイールを無視させる。
  // vendored xterm には attachCustomWheelEventHandler が存在しないビルドがあるため存在確認必須
  //（存在しないまま呼ぶと ensureTerminal が毎回 TypeError → terminals 未登録 →
  //  ターミナルが一切描画されず、activateSession も途中で死ぬ）。
  // 無い場合は whenLayoutReady 側の capture リスナーで同等の抑止を行う。
  if (typeof term.attachCustomWheelEventHandler === 'function') {
    term.attachCustomWheelEventHandler(() => !isModalOverlayOpen());
  }
  // マウストラッキング (DECSET 9/1000/1002/1003) を常時無効化する。
  // disableStdin:true のため xterm はマウスレポートを PTY へ送れず tracking ON の利点が無い一方、
  // xterm は tracking ON 中はドラッグ選択を無効化する（Shift 押下時のみ選択可になる）。
  // SSH 経由の Copilot CLI / cursor-agent 等の TUI が tracking を有効化すると
  // テキスト選択・コピーができなくなるため、parse 完了ごとに検出してローカルでリセットする。
  // （Windows ローカルは ConPTY が DECSET を吸収するため元々発生しない。
  //   リセットの write が再度 onWriteParsed を発火するが、mode が 'none' になるためループしない）
  term.onWriteParsed(() => {
    try {
      if (term.modes.mouseTrackingMode !== 'none') {
        term.write('\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l');
      }
    } catch (_) { /* modes 未対応ビルドでも選択以外の動作は維持 */ }
  });
  const fitAddon = new FitAddon.FitAddon();
  term.loadAddon(fitAddon);
  if (typeof WebLinksAddon !== 'undefined') {
    const webLinks = new WebLinksAddon.WebLinksAddon((event, uri) => {
      event.preventDefault();
      window.open(uri, '_blank', 'noopener,noreferrer');
    });
    term.loadAddon(webLinks);
  }
  term.registerLinkProvider({
    provideLinks(y, callback) {
      const buf = term.buffer.active;
      const thisLine = buf.getLine(y - 1);
      if (!thisLine) { callback([]); return; }

      // wrapped 継続行の場合、先頭物理行まで遡って論理行全体を処理する
      let startY = y;
      let startLine = thisLine;
      if (thisLine.isWrapped) {
        let cur = y - 1;
        while (cur > 0) {
          const candidate = buf.getLine(cur - 1);
          if (!candidate) break;
          if (!candidate.isWrapped) { startY = cur; startLine = candidate; break; }
          cur--;
        }
        if (startY === y) { callback([]); return; }
      }

      // 論理行を構成する物理行（先頭行 + 後続の wrapped 継続行）を収集する
      const buildCellMap = (line) => {
        const cm = [];
        for (let x = 0; x < line.length; x++) {
          const cell = line.getCell(x);
          if (cell && cell.getWidth() !== 0) cm.push(x);
        }
        return cm;
      };
      const physRows = [{ y1: startY, text: startLine.translateToString(true), cellMap: buildCellMap(startLine) }];
      let peek = startY; // getLine は 0-based。peek=startY は 1-based の startY+1 行目
      while (true) {
        const next = buf.getLine(peek);
        if (!next || !next.isWrapped) break;
        physRows.push({ y1: peek + 1, text: next.translateToString(true), cellMap: buildCellMap(next) });
        peek++;
      }

      // 行テキストを結合し、各行の開始オフセットを記録する
      const rowOffsets = [];
      let off = 0;
      for (const r of physRows) { rowOffsets.push(off); off += r.text.length; }
      const combined = physRows.map(r => r.text).join('');

      // combined 上の charIndex → xterm の { x (1-based), y (1-based) }
      const ciToXY = (ci) => {
        let ri = physRows.length - 1;
        for (let i = 0; i < physRows.length - 1; i++) {
          if (ci < rowOffsets[i + 1]) { ri = i; break; }
        }
        const r = physRows[ri];
        const charInRow = ci - rowOffsets[ri];
        return { x: (r.cellMap[charInRow] ?? charInRow) + 1, y: r.y1 };
      };

      const links = [];
      const occupiedRanges = [];
      const overlapsExistingLink = (start, end) => occupiedRanges.some(r => start <= r.end && end >= r.start);
      const addPathLink = (rawPath, startCI) => {
        const pathStr = trimTerminalPathCandidate(rawPath);
        if (pathStr.length < 3) return;
        const endCI = startCI + pathStr.length - 1;
        if (overlapsExistingLink(startCI, endCI)) return;
        occupiedRanges.push({ start: startCI, end: endCI });
        const capturedPath = resolveTerminalPathCandidate(pathStr, id);
        const startPos = ciToXY(startCI);
        const endPos = ciToXY(endCI);
        links.push({
          range: { start: startPos, end: endPos },
          text: pathStr,
          hover() {
            // ホバーではポップアップを開かない（クリック起動のみ）。
            // xterm のリンク下線表示は維持される。
          },
          leave() {
            scheduleHidePathPopup();
          },
          activate(_event, _text) {
            showPathPopup(capturedPath, _event.clientX, _event.clientY, id);
          }
        });
      };

      for (const re of [ABS_WIN_PATH_RE, ABS_UNIX_PATH_RE]) {
        re.lastIndex = 0;
        let m;
        while ((m = re.exec(combined)) !== null) {
          if (re === ABS_UNIX_PATH_RE && !isTerminalPathStartBoundary(combined, m.index)) continue;
          addPathLink(m[1], m.index);
        }
      }
      let m;
      REL_PATH_RE.lastIndex = 0;
      while ((m = REL_PATH_RE.exec(combined)) !== null) {
        const rawPath = m[2];
        const trimmed = trimTerminalPathCandidate(rawPath);
        if (!isLikelyRelPath(trimmed)) continue;
        addPathLink(rawPath, m.index + m[1].length);
      }
      // 折りたたみマーカーをクリック可能にする（ctrl+o キャプチャ → ポップアップ表示）
      CRUNCH_LINK_RE.lastIndex = 0;
      let cm;
      while ((cm = CRUNCH_LINK_RE.exec(combined)) !== null) {
        const startCI = cm.index;
        const endCI = cm.index + cm[0].length - 1;
        if (overlapsExistingLink(startCI, endCI)) continue;
        occupiedRanges.push({ start: startCI, end: endCI });
        links.push({
          range: { start: ciToXY(startCI), end: ciToXY(endCI) },
          text: cm[0],
          hover() {},
          leave() {},
          activate(event) {
            handleCrunchLinkClick(id, event.clientX, event.clientY);
          },
        });
      }
      callback(links);
    }
  });
  if (typeof Unicode11Addon !== 'undefined') {
    const u11 = new Unicode11Addon.Unicode11Addon();
    term.loadAddon(u11);
    term.unicode.activeVersion = '11';
    // 丸数字(U+2460-24FF)・ローマ数字(U+2160-217F)は East Asian Ambiguous 幅。
    // unicode11 はこれらを width=1 と判定するが、MS Gothic 等は全角グリフで描くため
    // 1 セル枠からはみ出して隣の文字と重なる。これらだけ width=2 に上書きし、
    // 全角グリフを 2 セル枠へぴったり収める。
    // 幅テーブルは unicode11 本体を流用する（捕捉用ダミーへ activate して provider を取り出す）。
    let v11 = null;
    try {
      new Unicode11Addon.Unicode11Addon().activate({ unicode: { register(p) { v11 = p; } } } as any);
    } catch (_) { /* 捕捉失敗時は version '11' のまま（重なりは残るが描画は維持） */ }
    if (v11 && typeof v11.wcwidth === 'function') {
      const isAmbiguousWide = (cp) =>
        (cp >= 0x2160 && cp <= 0x217F) || (cp >= 0x2460 && cp <= 0x24FF);
      term.unicode.register({
        version: '11-aacli',
        wcwidth(cp) { return isAmbiguousWide(cp) ? 2 : v11.wcwidth(cp); },
      } as any);
      term.unicode.activeVersion = '11-aacli';
    }
  }
  terminals.set(id, {
    term,
    fitAddon,
    webglAddon: null,
    container: null,
    pendingChunks: [],
    pendingTotalBytes: 0,
    pendingFlushActive: false,
    pendingFlushSeq: 0,
    pendingFlushWatchdog: null,
    pendingTextTail: '',
    textDecoder: new TextDecoder('utf-8'),
    markerFilterCarry: new Uint8Array(0),
    screenClearSeqCarry: new Uint8Array(0),
    eraseScrollbackFilterCarry: new Uint8Array(0),
    crFilterCarry: new Uint8Array(0),
    cursorHideFilterCarry: new Uint8Array(0),
    inCursorHideBlock: false,
    cursorHideBlockBuf: [] as number[],
    cursorHideHasAbsPos: false,
    cursorHideHasNewline: false,
    liveStatusText: '',
    liveStatusHideTimer: null,
    liveLineRow: null,
    liveLineCells: [],
    compactingSince: null,
    compactSeenAt: 0,
    autoScroll: true,
    everAttached: false,
  });
}

export function attachTerminal(id) {
  const area = document.getElementById('terminal-area');
  if (!area) return;
  const t = terminals.get(id);
  if (!t) return;
  // セッション切替・タブ復帰時は過去ログビューアを閉じる（別セッションの誤表示防止）
  resetHistoryViewerForSessionChange();
  // 切替先セッションの最新ライブ進捗を反映（無ければ hidden に戻す）
  syncLiveStatusDomForActive();
  if (t.container) {
    // DOM 再配置で WebGL canvas の描画バッファが失われるため、移動前に破棄する
    disableWebglRenderer(t);
    area.innerHTML = '';
    area.appendChild(t.container);
    releaseHiddenWebglRenderers();
    t.autoScroll = true;
    updateScrollLockBtn(false);
    requestAnimationFrame(() => {
      if (!terminals.has(id)) return;
      // 非アクティブ中の chunks は、その間 PTY が使っていた旧 cols/rows のまま先に反映する。
      // 先に fit すると、TUI の古い幅の再描画フレームが別幅で解釈されて上部に残像が出る。
      flushPending(id);
      const prevCols = t.term.cols;
      const prevRows = t.term.rows;
      fitTerminalPreservingBottom(t, id);
      // 寸法が実際に変わった場合のみ送信（不要な SIGWINCH → 再描画 → 空白行挿入を防ぐ）
      if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
        sendResize(id, t.term.cols, t.term.rows);
      }
      // 配置・fit 確定後に WebGL レンダラを再生成する
      enableWebglRenderer(t);
    });
    return;
  }
  const container = document.createElement('div');
  container.style.width = '100%';
  container.style.height = '100%';
  t.container = container;
  area.innerHTML = '';
  area.appendChild(container);
  releaseHiddenWebglRenderers();
  whenLayoutReady(id, container);
}

// WebGL レンダラを有効化する。既定の DOM レンダラはグリフをフォントの自然な字送りで
// 流し込むため、全角グリフの字送りとセル幅（fontSize:13 では半角 6.5px と端数）の差が
// 行内で蓄積し、選択ハイライト（セルグリッド基準）と文字の見た目がずれる。
// WebGL レンダラはグリフを 1 セルずつグリッドへ描画するためずれない。
// WebGL 非対応・コンテキストロスト時は addon を破棄して DOM レンダラへ戻す。
export function enableWebglRenderer(t) {
  if (typeof WebglAddon === 'undefined' || t.webglAddon) return;
  try {
    const addon = new WebglAddon.WebglAddon();
    addon.onContextLoss(() => {
      try { addon.dispose(); } catch (_) {}
      t.webglAddon = null;
      // DOM レンダラへ戻った直後は何も再描画されないため全行 repaint する
      refreshAllRows(t);
    });
    t.term.loadAddon(addon);
    t.webglAddon = addon;
    // addon ロード直後に全行 repaint し、ロード前の内容も WebGL canvas へ載せる
    refreshAllRows(t);
  } catch (err) {
    console.warn('[terminal] WebGL renderer unavailable; falling back to DOM renderer', err);
  }
}

// WebGL レンダラを破棄して DOM レンダラへ戻す。
// container を DOM 上で再配置（タブ切替・マルチペインへの移動）すると WebGL canvas の
// 描画バッファが失われ、addon は全面再描画しないため大半の行が空白になる。
// VS Code と同様「移動前に破棄 → 配置確定後に再生成」で対処する。
export function disableWebglRenderer(t) {
  if (!t || !t.webglAddon) return;
  try { t.webglAddon.dispose(); } catch (_) {}
  t.webglAddon = null;
}

// 画面に表示されていないターミナルの WebGL コンテキストを解放する。
// ブラウザは同時 WebGL コンテキスト数に上限があり（Chrome で約16）、超えると
// 古いコンテキストから強制喪失されるため、DOM に接続中のペイン分だけ保持する。
export function releaseHiddenWebglRenderers() {
  terminals.forEach((t) => {
    if (t.webglAddon && (!t.container || !t.container.isConnected)) disableWebglRenderer(t);
  });
}

function refreshAllRows(t) {
  try { t.term.refresh(0, t.term.rows - 1); } catch (_) { /* 未 open 時は無視 */ }
}

export function whenLayoutReady(id, container) {
  const t = terminals.get(id);
  if (!t) return;
  if (container.clientWidth > 0 && container.clientHeight > 0) {
    t.term.open(container);
    enableWebglRenderer(t);
    fitTerminalPreservingBottom(t, id);
    if (!t.scrollHandlerInstalled) {
      t.scrollHandlerInstalled = true;
      t.scrollDisposable = t.term.onScroll(() => {
        const atBottom = isTerminalAtBottom(t);
        // autoScroll を倒すのは「ユーザーが直前に手動スクロールした」ときだけにする。
        // CLI(Claude Code 等)の TUI は頻繁に再描画し、その programmatic scroll でも
        // onScroll が発火する。これを無条件で autoScroll=false にしていたため、ユーザーが
        // 触っていないのに最下部吸着が外れ、CLI 内容の下にターミナルの空行帯が残っていた。
        // 手動操作は lastTerminalManualScrollAt(wheel/pointerdown で更新)で判定する。
        const manualRecently = Date.now() - lastTerminalManualScrollAt < 400;
        if (atBottom) {
          // 最下部に居るなら常に吸着 ON（down ボタン表示とも整合）。
          t.autoScroll = true;
        } else if (manualRecently) {
          // ユーザーが上へスクロールして履歴を読み始めた → 吸着 OFF にして位置を保つ。
          t.autoScroll = false;
        } else if (t.autoScroll && !t._snappingToBottom) {
          // 手動意図のない programmatic scroll(CLI の TUI 再描画)で最下部から外れた場合は、
          // 吸着 ON のまま直ちに最下部へ戻す。これにより CLI 内容の下に空行帯が残らない。
          // autoScroll=false(ユーザーが履歴閲覧中)のときは何もせず現在位置を保つ。
          // 再入は _snappingToBottom でガード。
          t._snappingToBottom = true;
          try { t.term.scrollToBottom(); syncViewportScrollbarToBottom(t); }
          finally { t._snappingToBottom = false; }
        }
        if (id === activeSessionId) updateScrollLockBtn(!t.autoScroll);
        updateHistoryHint(id);
      });
    }
    const viewport = t.term.element?.querySelector('.xterm-viewport');
    if (viewport && !t.viewportScrollIntentInstalled) {
      t.viewportScrollIntentInstalled = true;
      viewport.addEventListener('pointerdown', () => {
        markTerminalManualScrollIntent();
      });
    }
    // attachCustomWheelEventHandler が無い xterm ビルド向けのフォールバック。
    // capture 段階で遮断することで、xterm 内部のホイールリスナーと
    // viewport のネイティブスクロールの両方へ届く前に止める（モーダル表示中のみ）。
    if (typeof t.term.attachCustomWheelEventHandler !== 'function' && !t.modalWheelBlockerInstalled) {
      t.modalWheelBlockerInstalled = true;
      container.addEventListener('wheel', (e) => {
        if (isModalOverlayOpen()) {
          e.preventDefault();
          e.stopImmediatePropagation();
        }
      }, { capture: true, passive: false });
    }
    flushPending(id);
    t.everAttached = true;
    if (!isPtyResizeSuppressed()) {
      sendResize(id, t.term.cols, t.term.rows);
    }
    container.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      const sel = t.term.getSelection();
      if (sel) openTermCtxMenu(e.clientX, e.clientY, sel);
    });
  } else {
    requestAnimationFrame(() => whenLayoutReady(id, container));
  }
}

// ---- 選択範囲コピーの右クリックメニュー ----
// TUI（Claude Code 等）が画面幅で再描画した出力はハード改行混じりでコピーされるため、
// 通常コピーに加えて「1行コピー」（改行除去 → スペース join）を選べるメニューを出す。
let termCtxMenuEl = null;
let termCtxSelection = '';

function ensureTermCtxMenu() {
  if (termCtxMenuEl) return termCtxMenuEl;
  const m = document.createElement('div');
  m.className = 'term-ctx-menu';
  const mkItem = (i18nKey, fallback, icon, handler) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    const ico = document.createElement('span');
    ico.className = 'ctx-icon';
    ico.textContent = icon;
    const label = document.createElement('span');
    label.className = 'ctx-label';
    label.dataset.i18nKey = i18nKey;
    label.dataset.i18nFallback = fallback;
    btn.appendChild(ico);
    btn.appendChild(label);
    btn.addEventListener('click', () => {
      const sel = termCtxSelection;
      // closeTermCtxMenu() で display:none になると getBoundingClientRect() が
      // 全て 0 を返しトーストが左上に飛ぶため、閉じる前に座標を確保しておく
      const r = btn.getBoundingClientRect();
      const anchor = { clientX: r.right, clientY: r.top + r.height / 2 };
      closeTermCtxMenu();
      if (sel) handler(sel, anchor);
    });
    m.appendChild(btn);
  };
  mkItem('term_ctx_copy', 'コピー', '⎘', (sel, anchor) => copyTerminalSelectionText(sel, { anchor }).catch(() => {}));
  mkItem('term_ctx_copy_oneline', '1行コピー', '⇥', (sel, anchor) => copyTerminalSelectionText(sel, { oneLine: true, anchor }).catch(() => {}));
  mkItem('term_ctx_add_to_input', '入力欄に追加', '＋', (sel, anchor) => addSelectionToInput(sel, { anchor }));
  document.body.appendChild(m);
  document.addEventListener('click', (e) => {
    if (!m.contains(e.target)) closeTermCtxMenu();
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') closeTermCtxMenu();
  });
  window.addEventListener('blur', closeTermCtxMenu);
  termCtxMenuEl = m;
  return m;
}

function openTermCtxMenu(x, y, selection) {
  const m = ensureTermCtxMenu();
  termCtxSelection = selection;
  // 言語切替に追従するため表示のたびにラベルを引き直す
  m.querySelectorAll('.ctx-label').forEach((el) => {
    const v = ti18n(el.dataset.i18nKey);
    el.textContent = (v && v !== el.dataset.i18nKey) ? v : el.dataset.i18nFallback;
  });
  m.classList.add('open');
  const r = m.getBoundingClientRect();
  m.style.left = Math.max(0, Math.min(x, window.innerWidth - r.width - 4)) + 'px';
  m.style.top = Math.max(0, Math.min(y, window.innerHeight - r.height - 4)) + 'px';
}

function closeTermCtxMenu() {
  if (!termCtxMenuEl) return;
  termCtxMenuEl.classList.remove('open');
  termCtxSelection = '';
}

export function queuePendingTerminalChunk(id, bytes) {
  const t = terminals.get(id);
  if (!t || !bytes) return;
  t.pendingChunks.push(bytes);
  t.pendingTotalBytes = (t.pendingTotalBytes || 0) + bytes.length;
  while (t.pendingTotalBytes > TERMINAL_PENDING_MAX_BYTES && t.pendingChunks.length > 1) {
    t.pendingTotalBytes -= t.pendingChunks[0].length;
    t.pendingChunks.shift();
  }
}

export function flushPending(id) {
  const t = terminals.get(id);
  if (!t) return;
  const chunks = t.pendingChunks;
  t.pendingChunks = [];
  t.pendingTotalBytes = 0;
  if (chunks.length === 0) {
    if (t.autoScroll) { t.term.scrollToBottom(); syncViewportScrollbarToBottom(t); }
    scheduleApprovalCheck(id);
    return;
  }
  const seq = (t.pendingFlushSeq || 0) + 1;
  t.pendingFlushSeq = seq;
  t.pendingFlushActive = true;
  if (t.pendingFlushWatchdog) {
    clearTimeout(t.pendingFlushWatchdog);
    t.pendingFlushWatchdog = null;
  }

  let finished = false;
  const finish = () => {
    if (finished) return;
    finished = true;
    const latest = terminals.get(id);
    if (!latest || latest.pendingFlushSeq !== seq) return;
    if (latest.pendingFlushWatchdog) {
      clearTimeout(latest.pendingFlushWatchdog);
      latest.pendingFlushWatchdog = null;
    }
    latest.pendingFlushActive = false;
    if (latest.pendingChunks.length > 0) {
      requestAnimationFrame(() => flushPending(id));
      return;
    }
    if (latest.autoScroll) { latest.term.scrollToBottom(); syncViewportScrollbarToBottom(latest); }
    scheduleApprovalCheck(id);
  };
  t.pendingFlushWatchdog = setTimeout(finish, TERMINAL_WRITE_FLUSH_WATCHDOG_MS);

  // 溜まったチャンクを 1 つに結合して一括書き込みする。
  // 以前は 8 chunks / 24KB ずつ rAF で逐次再生していたが、PTY 由来の細切れ
  // チャンクが数千件溜まると切替後の「再生待ち」が数秒以上になり、その間
  // scrollToBottom も走らず途中経過が流れ続けて見えていた。
  // 各フィルタ（マーカー除去・reverse-video 変換等）は carry 付きの
  // ストリーム処理なので結合しても結果は変わらず、xterm.js 側も write を
  // 内部で時分割パースするため UI ブロックは起きない。
  let totalBytes = 0;
  for (const c of chunks) totalBytes += c.length;
  const merged = new Uint8Array(totalBytes);
  let offset = 0;
  for (const c of chunks) {
    merged.set(c, offset);
    offset += c.length;
  }
  writePTYChunk(id, t.term, merged, finish);
}

window.flushPendingTerminalChunks = flushPending;

export function isTerminalAtBottom(t) {
  if (!t || !t.term || !t.term.buffer) return true;
  const buf = t.term.buffer.active;
  return buf.viewportY + t.term.rows >= buf.length;
}

// xterm の内部スクロール状態（ydisp=ybase=最下部）と、ネイティブの
// .xterm-viewport.scrollTop（青いスクロールバーのつまみ位置）は別管理で、
// fit（リサイズ）直後に PTY 出力が来るとつまみ位置の同期が取りこぼされ、
// 「表示は最下部なのにつまみだけ先頭に残る」状態になることがある。
// scrollToBottom() の後にこれを呼び、つまみも実際の表示位置（最下部）へ合わせる。
function syncViewportScrollbarToBottom(t) {
  const viewport = t?.term?.element?.querySelector('.xterm-viewport') as HTMLElement | null;
  if (!viewport) return;
  const target = viewport.scrollHeight - viewport.clientHeight;
  if (target > 0 && Math.abs(viewport.scrollTop - target) > 1) {
    viewport.scrollTop = target;
  }
}

export function fitTerminalPreservingBottom(t, id, forceVisualFit = false) {
  if (!canFitTerminal(t)) return;
  // PTY リサイズ抑制中（action-bar 表示中など）は通常スキップするが、
  // forceVisualFit=true の場合は「見た目のフィット（xterm の行数調整）」だけは行う。
  // SIGWINCH の送信（sendResize）は呼び元が isPtyResizeSuppressed() で別途ガードする。
  // これにより action-bar 出現でターミナルが縮んでも、xterm が縮小後の表示領域に
  // 追従して再フィットされ、CLI 最新行がポップアップ裏に切れて隠れるのを防ぐ。
  if (!forceVisualFit && isPtyResizeSuppressed()) return;
  const wasAtBottom = isTerminalAtBottom(t) || t.autoScroll;
  t.fitAddon.fit();
  if (wasAtBottom) {
    t.autoScroll = true;
    t.term.scrollToBottom();
    syncViewportScrollbarToBottom(t);
    if (id === activeSessionId) updateScrollLockBtn(false);
  }
}

// action-bar（承認ポップアップ）の手動リサイズに追従して、アクティブセッションの
// xterm を現在の表示領域へ再フィットし、最下部へスクロールする。
// 承認表示中は isPtyResizeSuppressed() が true で通常の fit はスキップされるため、
// forceVisualFit=true で「見た目のフィット」だけ強制し、ポップアップを縮めた瞬間に
// CLI 最新行が空いた領域へ降りてくるようにする（ユーザーが手動スクロールせずに済む）。
export function followActionBarResize(): void {
  if (activeSessionId === null) return;
  const t = terminals.get(activeSessionId);
  if (!canFitTerminal(t)) return;
  t.autoScroll = true;
  fitTerminalPreservingBottom(t, activeSessionId, true);
}

// xterm が alternate screen buffer（TUI モード, Codex 等）に居るかを判定。
// alt buffer は scrollback を持たないため term.scrollLines は no-op となり、
export function isAlternateBuffer(t) {
  if (!t || !t.term || !t.term.buffer) return false;
  const active = t.term.buffer.active;
  return !!(active && active.type === 'alternate');
}

// alt buffer 中のスクロール操作は PTY 側アプリ（Codex / Grok 等の TUI）に
// PgUp/PgDn として転送する。xterm は disableStdin:true のため、mouse tracking が
// ON でも wheel を PTY へ送らない。
// 戻り値: 転送した場合 true（呼び元は xterm scrollback 操作等をスキップ）。
export function scrollAltBufferPage(sessionId, t, direction) {
  if (!isAlternateBuffer(t)) return false;
  const key = direction < 0 ? '\x1b[5~' : '\x1b[6~';
  try { sendText(sessionId, key); } catch (_) {}
  return true;
}

export function forwardWheelToAltBuffer(sessionId, t, deltaY) {
  return scrollAltBufferPage(sessionId, t, deltaY < 0 ? -1 : 1);
}

export function isWheelTargetExcluded(target) {
  if (!(target instanceof Element)) return true;
  if (document.body && !document.body.contains(target)) return true;
  const input = document.getElementById('input');
  if (input && input.contains(target) && input.scrollHeight > input.clientHeight + 1) {
    return true;
  }
  if (target.closest('.card-actions')) return true;
  if (target.closest('[data-wheel-native]')) return true;
  if (target.closest('#settings-panel')) return true;
  // 承認バー（action-bar）は max-height:40vh / overflow-y:auto で自前のスクロール領域を持つ。
  // 除外しないと document レベルの wheel ハンドラが背後ターミナルへ横取りし、
  // 長い承認内容を action-bar 内でスクロールできなくなる（= 承認表示中にホイールが阻害される）。
  if (target.closest('#action-bar')) return true;
  // 左サイドバー（セッション一覧 / 新規セッションパネル / cwd 履歴ドロップダウン）は
  // ネイティブの wheel スクロールを使う。除外しないと document レベルのリスナーが
  // ターミナルへ転送して preventDefault し、サイドバー上でホイールが効かなくなる。
  if (target.closest('#session-list')) return true;
  // ファイルタブ（ツリー / プレビュー）はネイティブの wheel スクロールを使う。
  // これを除外しないと document レベルのリスナーがターミナルへ転送して preventDefault してしまい、
  // プレビューのスクロールが効かなくなる。
  if (target.closest('#files-tab-contents')) return true;
  return false;
}

export function routeWheelToOpenSettingsPanel(e) {
  const panel = document.getElementById('settings-panel');
  if (!panel || panel.hidden) return false;
  const body = panel.querySelector('.settings-body');
  if (!body) return false;

  if (e.target instanceof Element && body.contains(e.target)) {
    return false;
  }

  const unit = e.deltaMode === WheelEvent.DOM_DELTA_LINE
    ? 24
    : e.deltaMode === WheelEvent.DOM_DELTA_PAGE
      ? Math.max(1, body.clientHeight)
      : 1;
  body.scrollTop += e.deltaY * unit;
  e.preventDefault();
  e.stopPropagation();
  return true;
}

export function getWheelTargetSessionId(target) {
  if (!(target instanceof Element)) return activeSessionId;

  const multiView = document.getElementById('multi-view');
  const mgr = window.multiPaneManager;
  if (multiView && !multiView.hidden && mgr) {
    const slotEl = target.closest('.pane-slot');
    if (slotEl && multiView.contains(slotEl)) {
      const idx = parseInt(slotEl.dataset.slotIdx || '', 10);
      const session = Number.isInteger(idx) && mgr.slots ? mgr.slots[idx]?.session : null;
      if (session && session.id !== undefined && terminals.has(session.id)) {
        return session.id;
      }
    }
  }

  return activeSessionId;
}

// xterm のネイティブ wheel は viewport.scrollTop を直接書き換える → scroll イベント →
// scrollLines という非同期チェーンを経て初めて BufferService.isUserScrolling=true になる。
// この間に PTY 出力が来ると、内部 scroll() が `isUserScrolling || (ydisp = ybase)` で
// 強制的に最下部に戻してしまう（= AI 実行中に wheel up しても戻されるバグの正体）。
// → capture phase で wheel を奪い、scrollLines() を同期で呼んで isUserScrolling を
//    確実に立ててから xterm に伝播させない。
document.addEventListener('wheel', (e) => {
  if (routeWheelToOpenSettingsPanel(e)) return;

  // モーダル/オーバーレイ（about-panel / model-picker / new-session-panel 等）表示中は、
  // この document レベルハンドラが背後ターミナルを scrollLines しないよう早期 return する。
  // settings-panel は上の routeWheelToOpenSettingsPanel が先に処理するのでここに来ても問題ない
  //（body 内なら return false → ここで早期 return しネイティブスクロールに任せる）。
  // ここを抜くと、モーダルの非スクロール領域でのホイールが背後 CLI に転送されてしまう。
  if (isModalOverlayOpen()) return;

  // マウスがチャットペイン上にある場合: 最近傍のスクロール可能要素へ明示スクロール
  if (e.target instanceof Element && e.target.closest('#chat-pane')) {
    let scrollEl = null;
    let el = e.target;
    while (el && el.id !== 'chat-pane') {
      const ov = window.getComputedStyle(el).overflowY;
      if ((ov === 'auto' || ov === 'scroll') && el.scrollHeight > el.clientHeight) {
        scrollEl = el;
        break;
      }
      el = el.parentElement;
    }
    if (!scrollEl) scrollEl = document.querySelector('#chat-pane .chat-timeline');
    if (scrollEl) {
      const unit = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? 24
        : e.deltaMode === WheelEvent.DOM_DELTA_PAGE ? Math.max(1, scrollEl.clientHeight) : 1;
      scrollEl.scrollTop += e.deltaY * unit;
      e.preventDefault();
      e.stopPropagation();
    }
    return;
  }

  const targetSessionId = getWheelTargetSessionId(e.target);
  if (targetSessionId === null || targetSessionId === undefined) return;
  const t = terminals.get(targetSessionId);
  if (!t || !t.term) return;
  if (isWheelTargetExcluded(e.target)) return;

  if (forwardWheelToAltBuffer(targetSessionId, t, e.deltaY)) {
    markTerminalManualScrollIntent();
    e.preventDefault();
    e.stopPropagation();
    return;
  }

  markTerminalManualScrollIntent();
  const lineHeight = 24;
  const lines = Math.sign(e.deltaY) * Math.max(1, Math.round(Math.abs(e.deltaY) / lineHeight));
  try { t.term.scrollLines(lines); } catch (_) {}
  t.autoScroll = isTerminalAtBottom(t);
  if (targetSessionId === activeSessionId) updateScrollLockBtn(!t.autoScroll);
  e.preventDefault();
  e.stopPropagation();
}, { passive: false, capture: true });

export let terminalBottomFollow: any = {
  id: null,
  startedAt: 0,
};

export let lastTerminalManualScrollAt = 0;

export function markTerminalManualScrollIntent() {
  lastTerminalManualScrollAt = Date.now();
  terminalBottomFollow.id = null;
}

export function scrollTerminalToBottomSoon(id, opts: any = {}) {
  const t = terminals.get(id);
  if (!t || !t.term) return;
  const force = !!opts.force;
  const passes = Math.max(1, opts.passes || 1);
  const startedAt = opts.startedAt || Date.now();

  const snap = () => {
    if (!terminals.has(id)) return;
    const tNext = terminals.get(id);
    if (!tNext || !tNext.term) return;
    if (force && lastTerminalManualScrollAt > startedAt) return;
    if (!force && !tNext.autoScroll) return;
    tNext.autoScroll = true;
    tNext.term.scrollToBottom();
    syncViewportScrollbarToBottom(tNext);
    if (id === activeSessionId) updateScrollLockBtn(false);
  };

  snap();
  let remaining = passes;
  const scheduleNext = () => {
    if (remaining <= 0) return;
    remaining--;
    requestAnimationFrame(() => {
      snap();
      scheduleNext();
    });
  };
  scheduleNext();
}

export function refitAndStickTerminalToBottomSoon(id, opts: any = {}) {
  if (id !== activeSessionId) return;
  const passes = Math.max(1, opts.passes || 4);
  const force = !!opts.force;
  const startedAt = opts.startedAt || Date.now();

  const run = () => {
    if (id !== activeSessionId) return;
    if (force && lastTerminalManualScrollAt > startedAt) return;
    const t = terminals.get(id);
    if (!canFitTerminal(t)) return;
    const prevCols = t.term.cols;
    const prevRows = t.term.rows;
    t.autoScroll = true;
    // 抑制中でも視覚的なフィットは行い（行数を縮小後の表示領域に合わせ）、
    // ポップアップ裏に CLI が切れて隠れるのを防ぐ。
    fitTerminalPreservingBottom(t, id, true);
    // SIGWINCH の送信は抑制中はスキップ（action-bar が閉じた時に正しいサイズで1回だけ送る）。
    if ((t.term.cols !== prevCols || t.term.rows !== prevRows) && !isPtyResizeSuppressed()) {
      sendResize(id, t.term.cols, t.term.rows);
    }
    scrollTerminalToBottomSoon(id, { force, passes: 1, startedAt });
  };

  requestAnimationFrame(() => {
    run();
    let remaining = passes - 1;
    const next = () => {
      if (remaining <= 0) return;
      remaining--;
      requestAnimationFrame(() => {
        run();
        next();
      });
    };
    next();
  });
}

export function refitAndStickTerminalToBottomAfterLayoutSettles(id, opts: any = {}) {
  const startedAt = opts.startedAt || Date.now();
  const force = !!opts.force;
  const passes = opts.passes || 4;
  const delays = opts.delays || [0, 80, 220];

  for (const delay of delays) {
    setTimeout(() => {
      if (activeSessionId !== id) return;
      if (force && lastTerminalManualScrollAt > startedAt) return;
      refitAndStickTerminalToBottomSoon(id, { force, passes, startedAt });
    }, delay);
  }
}

export function resumeTerminalBottomFollow(id, opts: any = {}) {
  if (id === null || id === undefined) return;
  const startedAt = opts.startedAt || Date.now();
  terminalBottomFollow.id = id;
  terminalBottomFollow.startedAt = startedAt;
  scrollTerminalToBottomSoon(id, { force: true, passes: 2, startedAt });
}

export function revealApprovalPromptForSession(id) {
  if (id === null || id === undefined) return;
  if (!approvalVisibleCache.get(id) && !(approvalRawOptionsCache.get(id)?.length > 0)) return;
  const startedAt = Date.now();
  scrollTerminalToBottomSoon(id, { force: true, passes: 4, startedAt });
  refitAndStickTerminalToBottomSoon(id, { force: true, passes: 4, startedAt });
  refitAndStickTerminalToBottomAfterLayoutSettles(id, {
    force: true,
    passes: 4,
    startedAt,
  });
}

export function refitActiveTerminalAfterLayout(stickToBottom) {
  if (activeSessionId === null) return;
  const id = activeSessionId;
  const t = terminals.get(id);
  if (!canFitTerminal(t)) return;
  if (stickToBottom) {
    t.autoScroll = true;
  }
  requestAnimationFrame(() => {
    if (activeSessionId !== id || !canFitTerminal(t)) return;
    const prevCols = t.term.cols;
    const prevRows = t.term.rows;
    fitTerminalPreservingBottom(t, id);
    const dimsChanged = t.term.cols !== prevCols || t.term.rows !== prevRows;
    if (dimsChanged) {
      if (!isPtyResizeSuppressed()) {
        sendResize(id, t.term.cols, t.term.rows);
      }
    }
    if (stickToBottom) {
      scrollTerminalToBottomSoon(id);
      // 入力欄の改行（Shift+Enter）等で寸法が変わると、Codex の Ink TUI が
      // SIGWINCH を受けて全画面を再描画し、スクリーンクリア系シーケンス
      // （\x1b[2J / \x1b[3J / \x1b[H 等）で viewportY を最上部へ落とす。
      // この再描画は上の単発スナップより遅れて届き、その際 onScroll が
      // autoScroll=false に倒すため snapToBottomAfterScreenClear も効かず、
      // ターミナルが最上部に張り付いてしまう。寸法が変わったときだけ、
      // レイアウト確定後に force 付きで数フレーム追従し直して取りこぼしを
      // 防ぐ（手動スクロール中は force 側が lastTerminalManualScrollAt を
      // 見て中断するため、上にスクロール中の表示位置は維持される）。
      if (dimsChanged) {
        refitAndStickTerminalToBottomAfterLayoutSettles(id, { force: true, startedAt: Date.now() });
      }
    }
  });
}

export function updateScrollLockBtn(_locked?: boolean) {
  // ボタンはアクティブセッションがある間は常時表示する。
  // 以前は viewportY で「最上部/最下部判定して hidden」していたが、
  // xterm の onScroll は viewportY が動いた時しか発火せず、
  // PTY 出力でバッファが伸びた場合などにボタン表示が更新されない事象があった。
  // 常時表示なら更新タイミングに依存しないし、すでに端に居る時に再度押しても無害。
  const topBtn = document.getElementById('scroll-to-top-btn');
  const bottomBtn = document.getElementById('scroll-to-bottom-btn');
  const hasSession = activeSessionId !== null && terminals.has(activeSessionId);
  if (topBtn) topBtn.hidden = !hasSession;
  if (bottomBtn) bottomBtn.hidden = !hasSession;
  // 承認ポップアップ 再表示/消す ボタンもセッション表示中は常時表示する。
  const recallBar = document.getElementById('approval-recall-bar');
  if (recallBar) recallBar.hidden = !hasSession;
}

document.getElementById('scroll-to-top-btn')?.addEventListener('click', () => {
  if (activeSessionId === null) return;
  const t = terminals.get(activeSessionId);
  if (!t) return;
  markTerminalManualScrollIntent();
  if (scrollAltBufferPage(activeSessionId, t, -1)) {
    t.autoScroll = false;
    updateScrollLockBtn(true);
    return;
  }
  t.autoScroll = false;
  t.term.scrollToTop();
  updateScrollLockBtn(true);
});

document.getElementById('scroll-to-bottom-btn')?.addEventListener('click', () => {
  if (activeSessionId === null) return;
  const t = terminals.get(activeSessionId);
  if (!t) return;
  if (scrollAltBufferPage(activeSessionId, t, 1)) {
    t.autoScroll = true;
    updateScrollLockBtn(false);
    return;
  }
  t.autoScroll = true;
  t.term.scrollToBottom();
  syncViewportScrollbarToBottom(t);
});

export const hubMarkerBytePatterns = [
  new TextEncoder().encode('[MANY-AI-CLI]'),
  new TextEncoder().encode('[/MANY-AI-CLI]'),
];
export const hubMarkerEndBytes = hubMarkerBytePatterns[1];
export const hubDoneMarkerOpen = new TextEncoder().encode('[MANY-AI-CLI-DONE]');
export const hubDoneMarkerClose = new TextEncoder().encode('[/MANY-AI-CLI-DONE]');
export const eraseDisplayBelowBytes = new TextEncoder().encode('\x1b[J');
export const screenClearSeqBytePatterns = [
  asciiBytes('\x1b[2J'),
  asciiBytes('\x1b[3J'),
  asciiBytes('\x1b[H'),
  asciiBytes('\x1b[0;0H'),
  asciiBytes('\x1b[1;1H'),
  asciiBytes('\x1b[?1049h'),
  asciiBytes('\x1b[?1049l'),
];
export const screenClearSeqCarryLength = Math.max(...screenClearSeqBytePatterns.map(pattern => pattern.length)) - 1;
export const synchronizedUpdateSeqBytePatterns = [
  asciiBytes('\x1b[?2026h'),
  asciiBytes('\x1b[?2026l'),
];
export const synchronizedUpdateSeqCarryLength = Math.max(...synchronizedUpdateSeqBytePatterns.map(pattern => pattern.length)) - 1;
export const hideCursorSeq = asciiBytes('\x1b[?25l');
export const showCursorSeq = asciiBytes('\x1b[?25h');

export function bytesStartWith(bytes, offset, pattern) {
  if (offset + pattern.length > bytes.length) return false;
  for (let i = 0; i < pattern.length; i++) {
    if (bytes[offset + i] !== pattern[i]) return false;
  }
  return true;
}

function isPossiblePrefix(bytes, offset, patterns) {
  const remaining = bytes.length - offset;
  return patterns.some((pattern) => {
    if (remaining >= pattern.length) return false;
    for (let i = 0; i < remaining; i++) {
      if (bytes[offset + i] !== pattern[i]) return false;
    }
    return true;
  });
}

export function isPossibleMarkerPrefix(bytes, offset) {
  return isPossiblePrefix(bytes, offset, hubMarkerBytePatterns) ||
    isPossiblePrefix(bytes, offset, [hubDoneMarkerOpen]);
}

export function filterHubMarkersForDisplay(id, bytes) {
  const t = terminals.get(id);
  if (!t) return bytes;
  const carry = t.markerFilterCarry || new Uint8Array(0);
  const combined = new Uint8Array(carry.length + bytes.length);
  combined.set(carry, 0);
  combined.set(bytes, carry.length);

  const out = [];
  let i = 0;
  let inDone = t.inDoneBlock || false;

  while (i < combined.length) {
    if (inDone) {
      if (bytesStartWith(combined, i, hubDoneMarkerClose)) {
        i += hubDoneMarkerClose.length;
        inDone = false;
        for (const b of eraseDisplayBelowBytes) out.push(b);
        continue;
      }
      if (isPossiblePrefix(combined, i, [hubDoneMarkerClose])) break;
      i++;
      continue;
    }

    if (bytesStartWith(combined, i, hubDoneMarkerOpen)) {
      i += hubDoneMarkerOpen.length;
      inDone = true;
      continue;
    }

    const marker = hubMarkerBytePatterns.find(pattern => bytesStartWith(combined, i, pattern));
    if (marker) {
      i += marker.length;
      if (marker === hubMarkerEndBytes) {
        for (const b of eraseDisplayBelowBytes) out.push(b);
      }
      continue;
    }
    if (isPossibleMarkerPrefix(combined, i)) break;
    out.push(combined[i]);
    i++;
  }

  t.markerFilterCarry = combined.slice(i);
  t.inDoneBlock = inDone;
  return new Uint8Array(out);
}

export function asciiBytes(str) {
  const bytes = new Uint8Array(str.length);
  for (let i = 0; i < str.length; i++) bytes[i] = str.charCodeAt(i) & 0xff;
  return bytes;
}

export function filterReverseVideoForDisplay(id, bytes) {
  const t = terminals.get(id);
  if (!t) return bytes;
  const carry = t.reverseVideoFilterCarry || new Uint8Array(0);
  const combined = new Uint8Array(carry.length + bytes.length);
  combined.set(carry, 0);
  combined.set(bytes, carry.length);

  const out = [];
  let i = 0;
  while (i < combined.length) {
    if (combined[i] !== 0x1b) {
      out.push(combined[i]);
      i++;
      continue;
    }
    if (i + 1 >= combined.length) break;
    if (combined[i + 1] !== 0x5b) {
      out.push(combined[i]);
      i++;
      continue;
    }

    let j = i + 2;
    while (j < combined.length && !(combined[j] >= 0x40 && combined[j] <= 0x7e)) j++;
    if (j >= combined.length) break;
    if (combined[j] !== 0x6d) {
      for (let k = i; k <= j; k++) out.push(combined[k]);
      i = j + 1;
      continue;
    }

    const params = Array.from(combined.slice(i + 2, j), b => String.fromCharCode(b)).join('');
    const parts = params.split(';');
    const hasReverse = parts.includes('7');
    const hasReverseOff = parts.includes('27');
    const filtered = parts.filter(p => p !== '7' && p !== '27');
    if (hasReverse) filtered.push('48', '5', '238');
    if (hasReverseOff) filtered.push('49');
    if (filtered.length > 0) {
      for (const b of asciiBytes(`\x1b[${filtered.join(';')}m`)) out.push(b);
    }
    i = j + 1;
  }

  t.reverseVideoFilterCarry = combined.slice(i);
  return new Uint8Array(out);
}

export function filterSynchronizedUpdateForDisplay(id, bytes) {
  const t = terminals.get(id);
  if (!t) return bytes;
  const carry = t.synchronizedUpdateFilterCarry || new Uint8Array(0);
  const combined = new Uint8Array(carry.length + bytes.length);
  combined.set(carry, 0);
  combined.set(bytes, carry.length);

  const out = [];
  let i = 0;
  const carryStartLimit = Math.max(0, combined.length - synchronizedUpdateSeqCarryLength);
  while (i < combined.length) {
    const seq = synchronizedUpdateSeqBytePatterns.find(pattern => bytesStartWith(combined, i, pattern));
    if (seq) {
      i += seq.length;
      continue;
    }
    if (i >= carryStartLimit) {
      const maybePrefix = synchronizedUpdateSeqBytePatterns.some((pattern) => {
        const remaining = combined.length - i;
        if (remaining >= pattern.length) return false;
        for (let j = 0; j < remaining; j++) {
          if (combined[i + j] !== pattern[j]) return false;
        }
        return true;
      });
      if (maybePrefix) break;
    }
    out.push(combined[i]);
    i++;
  }

  t.synchronizedUpdateFilterCarry = combined.slice(i);
  return new Uint8Array(out);
}

export function detectScreenClearSeqForAutoScroll(id, bytes) {
  const t = terminals.get(id);
  if (!t || !bytes || bytes.length === 0) return false;
  const carry = t.screenClearSeqCarry || new Uint8Array(0);
  const combined = new Uint8Array(carry.length + bytes.length);
  combined.set(carry, 0);
  combined.set(bytes, carry.length);

  let found = false;
  for (let i = 0; i < combined.length && !found; i++) {
    found = screenClearSeqBytePatterns.some(pattern => bytesStartWith(combined, i, pattern));
  }

  const carryStart = Math.max(0, combined.length - screenClearSeqCarryLength);
  t.screenClearSeqCarry = combined.slice(carryStart);
  return found;
}

export const eraseScrollbackSeqBytePatterns = [
  asciiBytes('\x1b[3J'),
];
export const eraseScrollbackSeqCarryLength = Math.max(...eraseScrollbackSeqBytePatterns.map(pattern => pattern.length)) - 1;

export function filterEraseScrollbackForDisplay(id, bytes) {
  const t = terminals.get(id);
  if (!t) return bytes;
  if (sessions.get(id)?.provider !== 'codex') return bytes;
  const carry = t.eraseScrollbackFilterCarry || new Uint8Array(0);
  const combined = new Uint8Array(carry.length + bytes.length);
  combined.set(carry, 0);
  combined.set(bytes, carry.length);

  const out: number[] = [];
  let i = 0;
  while (i < combined.length) {
    const seq = eraseScrollbackSeqBytePatterns.find(pattern => bytesStartWith(combined, i, pattern));
    if (seq) {
      i += seq.length;
      continue;
    }
    if (i >= Math.max(0, combined.length - eraseScrollbackSeqCarryLength)) {
      const maybePrefix = eraseScrollbackSeqBytePatterns.some((pattern) => {
        const remaining = combined.length - i;
        if (remaining >= pattern.length) return false;
        for (let j = 0; j < remaining; j++) {
          if (combined[i + j] !== pattern[j]) return false;
        }
        return true;
      });
      if (maybePrefix) break;
    }
    out.push(combined[i]);
    i++;
  }

  t.eraseScrollbackFilterCarry = combined.slice(i);
  return new Uint8Array(out);
}

export function snapToBottomAfterScreenClear(id) {
  const t = terminals.get(id);
  if (!t || !t.autoScroll) return;
  t.term.scrollToBottom();
  syncViewportScrollbarToBottom(t);
  if (id === activeSessionId) updateScrollLockBtn(false);
}

// \x1b[?25l（カーソル非表示）〜 \x1b[?25h（表示）ブロックの中に row;col 形式の
// 絶対カーソル移動（\x1b[row;colH）が含まれる場合はブロック全体をフィルタする。
// Claude Code がステータスバーをこのパターンで書き込んでおり、xterm.js の
// スクロールバックに混入してツール呼び出し行の文字が壊れる原因になる。
// 絶対移動を含まない（初期化の cursor home 等）は通過させる。
// 改行（LF）を含むブロックも通過させる。ステータスバー更新は 1 行内の
// 書き換えで改行を含まない一方、Claude Code の起動バナー等の本文描画は
// 同じ ?25l〜?25h + 絶対移動パターンかつ複数行で、破棄すると spawn 直後の
// 画面が真っ黒になる（jsonl 実測: バナー約1.3〜1.5KB は改行入り、
// 破棄すべきステータス更新 30B は改行なし）。
// 改行を見つけた時点で「破棄対象でない」と確定するため、?25h を待たず即 flush する。
// Claude Code の /model 等のセレクタダイアログは描画後カーソルを非表示のままにして
// ?25h を送らないため、閉じを待つ実装だと描画全体（<2KB）が blockBuf に滞留し、
// 次の PTY 出力が来るまでダイアログが画面に一切表示されない
// （承認バーには出るのにターミナルには出ない、の原因）。
const MAX_CURSOR_HIDE_BUF = 2048;
export function filterCursorHideShowBlocksForDisplay(id, bytes) {
  const t = terminals.get(id);
  if (!t) return bytes;
  const carry = t.cursorHideFilterCarry || new Uint8Array(0);
  const combined = new Uint8Array(carry.length + bytes.length);
  combined.set(carry, 0);
  combined.set(bytes, carry.length);

  const out: number[] = [];
  let i = 0;
  let inBlock: boolean = t.inCursorHideBlock || false;
  let blockBuf: number[] = [...(t.cursorHideBlockBuf || [])];
  let hasAbsPos: boolean = t.cursorHideHasAbsPos || false;
  let hasNewline: boolean = t.cursorHideHasNewline || false;

  while (i < combined.length) {
    if (!inBlock) {
      if (bytesStartWith(combined, i, hideCursorSeq)) {
        inBlock = true;
        blockBuf = [];
        hasAbsPos = false;
        hasNewline = false;
        i += hideCursorSeq.length;
        continue;
      }
      if (isPossiblePrefix(combined, i, [hideCursorSeq])) {
        t.cursorHideFilterCarry = combined.slice(i);
        t.inCursorHideBlock = false;
        t.cursorHideBlockBuf = [];
        t.cursorHideHasAbsPos = false;
        t.cursorHideHasNewline = false;
        return new Uint8Array(out);
      }
      out.push(combined[i]);
      i++;
    } else {
      // バッファ上限超過時は非ステータス扱いで通過
      if (blockBuf.length >= MAX_CURSOR_HIDE_BUF) {
        for (const b of hideCursorSeq) out.push(b);
        for (const b of blockBuf) out.push(b);
        inBlock = false;
        blockBuf = [];
        hasAbsPos = false;
        hasNewline = false;
        continue;
      }
      if (bytesStartWith(combined, i, showCursorSeq)) {
        if (!hasAbsPos || hasNewline) {
          // ステータスバー更新でない（絶対移動なし or 複数行の本文描画） → 通過
          for (const b of hideCursorSeq) out.push(b);
          for (const b of blockBuf) out.push(b);
          for (const b of showCursorSeq) out.push(b);
        } else {
          // ステータスバー更新（スピナー進捗等）は scrollback へ描かず破棄するが、
          // 可読テキストを抽出して専用ライブ行に出し、進捗を可視化する。
          extractAndSetLiveStatus(id, blockBuf);
        }
        inBlock = false;
        blockBuf = [];
        hasAbsPos = false;
        hasNewline = false;
        i += showCursorSeq.length;
        continue;
      }
      if (isPossiblePrefix(combined, i, [showCursorSeq])) {
        t.cursorHideFilterCarry = combined.slice(i);
        t.inCursorHideBlock = true;
        t.cursorHideBlockBuf = blockBuf;
        t.cursorHideHasAbsPos = hasAbsPos;
        t.cursorHideHasNewline = hasNewline;
        return new Uint8Array(out);
      }
      // \x1b[row;colH（row・col ともに数字あり）を検出したらステータス更新とみなす
      if (!hasAbsPos && combined[i] === 0x1b && i + 4 < combined.length && combined[i + 1] === 0x5b) {
        let j = i + 2;
        let rowDigits = 0;
        while (j < combined.length && combined[j] >= 0x30 && combined[j] <= 0x39) { j++; rowDigits++; }
        if (rowDigits > 0 && j < combined.length && combined[j] === 0x3b) {
          j++;
          let colDigits = 0;
          while (j < combined.length && combined[j] >= 0x30 && combined[j] <= 0x39) { j++; colDigits++; }
          if (colDigits > 0 && j < combined.length && combined[j] === 0x48) {
            hasAbsPos = true;
            for (let k = i; k <= j; k++) blockBuf.push(combined[k]);
            i = j + 1;
            continue;
          }
        }
      }
      if (combined[i] === 0x0A) {
        // 改行入り = 本文描画と確定。?25h を待たずに通過させてブロックを抜ける。
        // 以降のバイトは生のまま通過し、後続の ?25h も（来れば）そのまま流れる。
        blockBuf.push(combined[i]);
        i++;
        for (const b of hideCursorSeq) out.push(b);
        for (const b of blockBuf) out.push(b);
        inBlock = false;
        blockBuf = [];
        hasAbsPos = false;
        hasNewline = false;
        continue;
      }
      blockBuf.push(combined[i]);
      i++;
    }
  }

  t.cursorHideFilterCarry = new Uint8Array(0);
  t.inCursorHideBlock = inBlock;
  t.cursorHideBlockBuf = blockBuf;
  t.cursorHideHasAbsPos = hasAbsPos;
  t.cursorHideHasNewline = hasNewline;
  return new Uint8Array(out);
}

// ── スピナー等のライブ進捗行 ───────────────────────────────────────────────
// filterCursorHideShowBlocksForDisplay が破棄するステータスバーブロックから
// 可読テキストを取り出し、#terminal-live-status へ出す。スピナーは毎秒数回
// 更新されるため、更新が LIVE_STATUS_HIDE_MS 途切れたら稼働中→待機表示(idle)へ移す。
// 待機表示でも枠は消さず常時残し、「何も送られていない＝終わっている」を一目で分かるようにする。
const LIVE_STATUS_HIDE_MS = 1500;
// ライブステータス表示（#terminal-live-status）の有効/無効。
// 以前はステータス更新ブロックを「断片で丸ごと上書き」していたため `·ii` / `+49`
// のような断片しか出ず無効化していた（bugfix_live-status-spinner-fragments_2026-06-12.md）。
// 現在は列アドレス（CUP/CUF）でセッションごとの 1 行へ部分更新を適用する再構成方式に
// 置き換え、断片バグが構造的に再発しないため有効化。くるくる自体は Web 側の CSS
// アニメーション（.live-spinner）で描くので再構成精度に関係なく必ず回る。
// 再無効化したい場合は false に戻す（退路として残す）。
const LIVE_STATUS_ENABLED = true;
const liveStatusDecoder = new TextDecoder('utf-8');

// ステータスバーブロック（絶対カーソル移動 + 部分書き換え）を、セッションごとの
// 仮想 1 行バッファ（列 → 文字のスパース配列）へ適用して全文テキストを組み立てる。
// 部分更新（スピナー記号だけ col1 / `↓` だけ col18 / 末尾だけ col29〜 等）が複数フレームに
// 分かれて来ても、列アドレスで同じ行へ重ね書きするため断片にならず全文が復元される。
function reconstructLiveLine(id, blockBuf): string {
  const t = terminals.get(id);
  if (!t) return '';
  let s = '';
  try { s = liveStatusDecoder.decode(new Uint8Array(blockBuf)); } catch (_) { return ''; }
  if (t.liveLineRow === undefined) t.liveLineRow = null;
  if (!t.liveLineCells) t.liveLineCells = [];
  const cells: string[] = t.liveLineCells;

  let row = t.liveLineRow ?? 1;        // 仮想カーソル行（液晶上の行番号）
  let col = 1;                         // 仮想カーソル列（1-based）
  const chars: string[] = Array.from(s); // コードポイント単位（全角は 1 要素 = 1 セル近似）

  for (let i = 0; i < chars.length; i++) {
    const ch = chars[i];
    if (ch === '\x1b') {
      const next = chars[i + 1];
      if (next === '[') {
        // CSI: パラメータ（0x20-0x3F）を読み、終端バイト（0x40-0x7E = @〜~）で確定
        let j = i + 2;
        let params = '';
        while (j < chars.length && !/[@-~]/.test(chars[j])) { params += chars[j]; j++; }
        const final = chars[j];
        if (final === 'H' || final === 'f') {            // CUP: 絶対位置
          const m = params.match(/^(\d*);?(\d*)$/);
          row = m && m[1] ? parseInt(m[1], 10) : 1;
          col = m && m[2] ? parseInt(m[2], 10) : 1;
        } else if (final === 'C') {                      // CUF: 右移動（実ログに [30C 等）
          col += params ? (parseInt(params, 10) || 1) : 1;
        } else if (final === 'D') {                      // CUB: 左移動
          col = Math.max(1, col - (params ? (parseInt(params, 10) || 1) : 1));
        } else if (final === 'G') {                      // CHA: 列絶対
          col = params ? (parseInt(params, 10) || 1) : 1;
        } else if (final === 'K') {                      // EL: 行消去（対象行のみ）
          if (row === t.liveLineRow) {
            const mode = params ? (parseInt(params, 10) || 0) : 0;
            if (mode === 0) cells.length = Math.min(cells.length, col);          // col 以降を消去
            else if (mode === 2) cells.length = 0;                               // 全消去
            else if (mode === 1) { for (let k = 1; k <= col && k < cells.length; k++) cells[k] = ' '; }
          }
        }
        // m（SGR 色）等その他の終端は読み飛ばすだけ
        i = (final === undefined) ? chars.length : j;
        continue;
      } else if (next === ']') {
        // OSC: BEL か ST（ESC \）まで読み飛ばす
        let j = i + 2;
        while (j < chars.length && chars[j] !== '\x07' && !(chars[j] === '\x1b' && chars[j + 1] === '\\')) j++;
        if (chars[j] === '\x1b') j++;
        i = j;
        continue;
      } else {
        i = i + 1; // 2 バイト ESC: 次を読み飛ばす
        continue;
      }
    }
    if (ch === '\r') { col = 1; continue; }
    if (ch === '\n') { row += 1; continue; }
    if (ch <= '\x1f' || ch === '\x7f') continue; // その他制御文字は無視
    // 印字可能文字
    if (row !== t.liveLineRow) {
      if (ch === ' ') { col += 1; continue; } // 別行への空白書き込み（[17;3H のカーソル退避先）は行を切り替えない
      // 文字・記号の実書き込みでこの行をステータス行として確定（端末スクロール追従）
      t.liveLineRow = row;
      cells.length = 0;
    }
    cells[col] = ch;
    col += 1;
  }

  let out = '';
  for (let k = 1; k < cells.length; k++) out += (cells[k] === undefined ? ' ' : cells[k]);
  out = out.replace(/\s+/g, ' ').trim();
  // 文字・数字を含まない（記号・空白だけの）段階では前回値を維持（''を返す）
  if (!/[\p{L}\p{N}]/u.test(out)) return '';
  return out;
}

export function extractAndSetLiveStatus(id, blockBuf) {
  if (!LIVE_STATUS_ENABLED) return; // 無効時はステータスバーブロックの破棄のみ行い、ライブ行へは出さない
  if (!blockBuf || blockBuf.length === 0) return;
  const text = reconstructLiveLine(id, blockBuf);
  // text が '' でも「ステータス更新フレームが来た」事実＝稼働中なので窓は出し続ける（くるくる継続）。
  setSessionLiveStatus(id, text);
}

// ── provider 別ライブステータス抽出（Codex/Copilot/Cursor 用） ─────────────────
// Claude は filterCursorHideShowBlocksForDisplay がステータスバーブロックを本文から
// 抜き取り、extractAndSetLiveStatus でピル内テキストを埋める。一方 Codex 等は別方式
// （Synchronized Update + 絶対カーソル移動でメインバッファへインライン全画面描画）の
// ため、このブロック抽出に乗らず liveStatusText が空になり「中のテキスト」が出ない。
// そこで本文（xterm バッファ）末尾を走査して進捗行を拾い、同じピルへ流し表示を統一する。
// スピナー回転自体は state=running 由来で別に回るため、ここでは中のテキストだけ補う。

// Codex: 「• Working (12s • esc to interrupt)」等のステータス行を最優先で拾い、
// 無ければ直近のアクション行（• Running/Ran/Reading …）を返す。どちらも無ければ
// '' を返し、setSessionLiveStatus 側で前回値を維持する。
function extractCodexLiveStatus(id) {
  const lines = scanBuffer(id, 48); // 末尾 48 行（おおむね 1 画面ぶん）を後方優先で見る
  let action = '';
  for (let i = lines.length - 1; i >= 0; i--) {
    const raw = lines[i].replace(/\s+/g, ' ').trim();
    if (!raw) continue;
    const stripped = raw.replace(/^[•·]\s*/, '').trim();
    if (/^Working\b/i.test(stripped)) return stripped;           // 稼働ステータス行（経過秒つき）
    if (!action && /^(Running|Ran|Reading|Read|Editing|Edited|Searching|Thinking)\b/i.test(stripped)) {
      action = stripped;                                          // 直近アクション行（最初の 1 件を控える）
    }
  }
  return action;
}

// provider（小文字）→ 抽出関数。Claude は既存のブロック抽出経路を使うため載せない。
// Copilot/Cursor は実機ログ採取後、同形式の抽出関数をここへ足すだけで載る拡張ポイント。
const liveStatusExtractors: Record<string, (id: number) => string> = {
  codex: extractCodexLiveStatus,
};

// provider 別抽出を 250ms に間引いて実行し、ピル内テキストへ流す。leading 抑制
// （予約中は新規予約しない）で Codex の毎秒大量フレームでもバッファ走査を間引く。
const liveStatusExtractTimers = new Map();
export function scheduleLiveStatusExtract(id) {
  if (!LIVE_STATUS_ENABLED) return;
  if (liveStatusExtractTimers.has(id)) return;
  const provider = String(sessions.get(id)?.provider || '').toLowerCase();
  const extractor = liveStatusExtractors[provider];
  if (!extractor) return; // Claude 等は既存経路に任せる
  liveStatusExtractTimers.set(id, setTimeout(() => {
    liveStatusExtractTimers.delete(id);
    if (!terminals.get(id)) return;
    setSessionLiveStatus(id, extractor(id));
  }, 250));
}

export function setSessionLiveStatus(id, text) {
  const t = terminals.get(id);
  if (!t) return;
  // 再構成済みの綺麗なステータス行からも compact 開始／完了を拾う（生チャンクが分割
  // されて「Compacting conversation」「Conversation compacted」が途切れても、ここで確実
  // に検出できる）。完了マーカーが先に来ることはないので順序判定は不要。
  if (text) {
    if (COMPACT_DETECT_RE.test(text)) noteCompactStart(id);
    else if (COMPACT_DONE_RE.test(text)) noteCompactDone(id);
  }
  if (text) t.liveStatusText = text; // 意味のある全文だけ差し替え（記号だけの中間フレームでは前回値維持）
  // フレームが来た＝稼働中。判定タイマーをリセットして「稼働中」表示へ。
  if (t.liveStatusHideTimer) clearTimeout(t.liveStatusHideTimer);
  // 更新が LIVE_STATUS_HIDE_MS 途切れたらセッション状態ベースの表示へ。枠は消さず常時残す
  // （何も送られていない＝終わっている／承認待ち等を一目で判別できるようにする）。
  t.liveStatusHideTimer = setTimeout(() => {
    const cur = terminals.get(id);
    if (!cur) return;
    cur.liveStatusHideTimer = null;
    if (id === activeSessionId) { const v = liveStatusViewFor(id); renderLiveStatusDom(v.mode, v.text); }
  }, LIVE_STATUS_HIDE_MS);
  if (id === activeSessionId) renderLiveStatusDom('active', t.liveStatusText || '');
}

// ── compact（Claude /compact）専用のライブ表示 ────────────────────────────────
// Claude Code は compact の中間進捗（10%,20%…）を PTY へ出さず 0%→100% に飛ぶため、
// ターミナル内のバーは固まって見える。そこで many-ai-cli 側で経過秒を発番し、ライブ
// 進捗窓に「圧縮中…(Ns)」＋不定形バーを出して「動いている」ことを示す。正確な % の
// 再現はしない（PTY に無いものは作らない）。
const COMPACT_DETECT_RE = /Compacting conversation/i;
// compact 完了時に Claude Code が PTY へ出す確定マーカー。これを拾えば IDLE タイムアウトを
// 待たずに即時解除できる（compact 直後に通常応答が始まるとフレームが途切れず IDLE では
// 解除できないため、確定マーカーが無いと「圧縮中…(Ns)」のまま秒数だけ伸び続ける）。
const COMPACT_DONE_RE = /Conversation compacted/i;
// compact のステータス更新が途切れた（＝完了）とみなすまでの猶予。spinner 再描画間隔より長く。
const COMPACT_IDLE_MS = 1500;
let compactTickTimer: ReturnType<typeof setInterval> | null = null;

// compact 開始を記録し、ライブ表示（不定形バー）を起動する。生チャンク・再構成済み
// ステータス行のどちらから検出しても通れるよう共通化する（チャンク分割での取りこぼし防止）。
function noteCompactStart(id) {
  const t = terminals.get(id);
  if (!t) return;
  const justStarted = t.compactingSince == null;
  if (justStarted) t.compactingSince = Date.now();
  t.compactSeenAt = Date.now();
  if (id === activeSessionId && justStarted) {
    ensureCompactTick();
    renderLiveStatusDom('active', ''); // text は renderLiveStatusDom 側で compact 文言へ差し替え
  }
}

// compact 完了マーカー（"Conversation compacted"）を観測した瞬間に呼ぶ。フレーム途切れ
// による IDLE 解除に頼らず即時で通常表示へ戻す。compact 中でなければ何もしない。
function noteCompactDone(id) {
  const t = terminals.get(id);
  if (!t || t.compactingSince == null) return;
  t.compactingSince = null;
  t.compactDetectTail = '';
  if (id === activeSessionId) {
    stopCompactTick();
    syncLiveStatusDomForActive();
  }
}

// PTY 出力チャンクから compact を追従する。
// 注意: 「Compacting conversation」は compact の最初の数フレームにしか出ず、その後は
// 通常ターンと同じランダムな進行語（Combobulating/Churning/Actioning 等。compact 無しでも
// 出る汎用スピナー語）へ切り替わる。よって「この語が出続ける限り表示」だと 1.5s で消え、
// 長い compact 本体の間ずっと何も出ない。開始を一度捉えたら、以降は内容を問わず当該
// セッションのフレーム到来で生存を更新し（＝処理が続く限り表示）、フレームが
// COMPACT_IDLE_MS 途切れたら完了とみなす（tick 側で判定）。
export function markCompactActivity(id, textChunk) {
  const t = terminals.get(id);
  if (!t) return;
  if (t.compactingSince != null) {
    t.compactSeenAt = Date.now(); // compact 中は任意フレームで延命
    // ただし完了マーカー "Conversation compacted" が同チャンクに来ていたら即解除する。
    // compact 直後に通常応答（Read 等）が始まると PTY フレームが途切れず IDLE 解除が効か
    // ないので、確定マーカーを直接拾う必要がある。チャンク境界分断対策は開始検出と同じ
    // 末尾繰り越し方式。
    if (textChunk) {
      const probe = (t.compactDetectTail || '') + textChunk;
      if (COMPACT_DONE_RE.test(probe)) { noteCompactDone(id); return; }
      t.compactDetectTail = probe.slice(-24);
    }
    return;
  }
  if (!textChunk) return;
  // 検出語「Compacting conversation」は compact 全体で 1 回しか出ない（その後は通常ターンと
  // 同じ汎用スピナー語へ替わる）。この 1 回をチャンク境界で分断されても確実に拾うため、
  // 直前チャンク末尾を前置して判定し、未検出なら末尾だけ次回へ繰り越す。
  const probe = (t.compactDetectTail || '') + textChunk;
  if (COMPACT_DETECT_RE.test(probe)) { t.compactDetectTail = ''; noteCompactStart(id); return; }
  t.compactDetectTail = probe.slice(-24); // 検出語長(23)+余裕ぶんだけ繰り越す
}

// activeSession が compact 中なら経過秒（整数）、そうでなければ null。
function activeCompactElapsedSec(): number | null {
  const id = activeSessionId;
  const t = id != null ? terminals.get(id) : null;
  if (!t || t.compactingSince == null) return null;
  return Math.max(0, Math.floor((Date.now() - t.compactingSince) / 1000));
}

// 経過秒を毎秒更新する単一タイマー。activeSession の compact が解除されたら停止する。
function ensureCompactTick() {
  if (compactTickTimer != null) return;
  compactTickTimer = setInterval(() => {
    const id = activeSessionId;
    const t = id != null ? terminals.get(id) : null;
    if (!t || t.compactingSince == null) { stopCompactTick(); return; }
    if (Date.now() - t.compactSeenAt > COMPACT_IDLE_MS) {
      // compact のステータス更新が途切れた＝完了とみなし、通常表示へ戻す。
      t.compactingSince = null;
      stopCompactTick();
      syncLiveStatusDomForActive();
      return;
    }
    renderLiveStatusDom('active', ''); // 経過秒を再計算して再描画
  }, 1000);
}

function stopCompactTick() {
  if (compactTickTimer != null) { clearInterval(compactTickTimer); compactTickTimer = null; }
}

// 現在のピル表示（mode + ラベル）を決める。フレーム流入中はライブテキスト、
// 途切れていればセッション状態（running/waiting/standby/error/disconnected）からラベルを引く。
// これにより「実行中／承認待ち／待機中（＝終わってる）／切断」がピルだけで分かる。
function liveStatusViewFor(sid) {
  const term = terminals.get(sid);
  const streaming = !!(term && term.liveStatusHideTimer);
  if (streaming) return { mode: 'active', text: (term && term.liveStatusText) || '' };
  const state = (sessions.get(sid)?.state as string) || 'standby';
  switch (state) {
    case 'running':      return { mode: 'active',  text: ti18n('live_status_running') };
    case 'waiting':      return { mode: 'waiting', text: ti18n('live_status_waiting') };
    case 'error':        return { mode: 'idle',    text: ti18n('live_status_error') };
    case 'disconnected': return { mode: 'idle',    text: ti18n('live_status_disconnected') };
    default:             return { mode: 'idle',    text: ti18n('live_status_idle') }; // standby ＝ 待機中（送信待ち）
  }
}

// ライブ進捗窓の描画。mode: 'active'（青・スピナー回転）/ 'waiting'（アンバー・承認待ち）/
// 'idle'（グレー・スピナー停止・状態ラベル／枠は残す）/ 'hidden'（アクティブセッション無し時のみ）。
function renderLiveStatusDom(mode, text) {
  const el = document.getElementById('terminal-live-status');
  if (!el) return;
  const textEl = el.querySelector('.live-status-text') as HTMLElement | null;
  const barEl = el.querySelector('.live-compact-bar') as HTMLElement | null;
  if (!LIVE_STATUS_ENABLED || mode === 'hidden') {
    el.hidden = true;
    el.classList.remove('idle', 'waiting');
    if (textEl) textEl.textContent = '';
    if (barEl) barEl.hidden = true;
    syncLiveStatusLongproc();
    return;
  }
  // compact 中は経過秒ラベル＋不定形バーへ差し替える。active の特殊形として扱い、
  // idle/waiting には落とさない（処理中であることを優先表示）。
  const compactSec = activeCompactElapsedSec();
  if (compactSec != null) {
    mode = 'active';
    text = ti18n('live_status_compacting', { sec: compactSec });
  }
  el.hidden = false;
  el.classList.toggle('idle', mode === 'idle');
  el.classList.toggle('waiting', mode === 'waiting');
  if (barEl) barEl.hidden = (compactSec == null);
  if (textEl && textEl.textContent !== text) textEl.textContent = text || '';
  syncLiveStatusLongproc();
}

// 長時間処理中インジケータ（ライブ帯の右側・パレットボタンの左隣）。
// アクティブセッションが running に入ってから LIVE_LONGPROC_SEC を超えて応答が続くと
// 「⚠ 長時間処理中」を帯の右側へ出す。サイドバーのカード長時間バッジ（session-list.ts の
// CARD_LONGPROC_SEC）と同じしきい値・同じ意味で、入力欄の真上でも気付けるようにする。
// ライブ帯はアクティブセッションぶんしか表示しないため、追跡もアクティブ分だけ持つ。
const LIVE_LONGPROC_SEC = 300;
const liveStatusRunningSince = new Map<number, number>();

export function syncLiveStatusLongproc(): void {
  const el = document.getElementById('terminal-live-status');
  if (!el) return;
  const id = activeSessionId;
  const state = id != null ? (sessions.get(id)?.state as string) : null;
  const lp = el.querySelector('.live-status-longproc') as HTMLElement | null;
  // running 以外（standby/waiting/error/切断）は追跡を捨てて非表示にする。
  if (id == null || state !== 'running') {
    if (id != null) liveStatusRunningSince.delete(id);
    if (lp) lp.hidden = true;
    return;
  }
  let since = liveStatusRunningSince.get(id);
  if (!since) { since = Date.now(); liveStatusRunningSince.set(id, since); }
  const sec = Math.max(0, Math.floor((Date.now() - since) / 1000));
  let badge = lp;
  if (!badge) {
    badge = document.createElement('span');
    badge.className = 'live-status-longproc';
    badge.textContent = `⚠ ${ti18n('card_longproc_label')}`;
    badge.title = ti18n('card_longproc_title');
    // パレットボタンの左隣へ置く（パレットは buildUI で末尾に append される）。
    const paletteBtn = el.querySelector('.live-status-palette-btn');
    if (paletteBtn) el.insertBefore(badge, paletteBtn); else el.appendChild(badge);
  }
  badge.hidden = sec < LIVE_LONGPROC_SEC;
}

// セッション切替・状態変化時に、アクティブセッションの現在状態を DOM へ反映する。
// 枠は常時表示。アクティブセッションが無いときだけ枠ごと消す(hidden)。
export function syncLiveStatusDomForActive() {
  if (activeSessionId === null || !terminals.get(activeSessionId)) { stopCompactTick(); renderLiveStatusDom('hidden', ''); return; }
  // 切替先が compact 中なら経過秒タイマーを再開（非アクティブ中は tick を回していないため）。
  if (activeCompactElapsedSec() != null) ensureCompactTick();
  const v = liveStatusViewFor(activeSessionId);
  renderLiveStatusDom(v.mode, v.text);
}

// \r（CR）で行頭へ戻ったあと行末を消去しないと、短い上書きテキストの後ろに
// 旧テキストの末尾が残ってスクロールバック上で混在して見える。
// \r の直後に \x1b[K（EL: Erase Line from cursor to right）を挿入して残留を防ぐ。
// \r\n の \r（正常な改行ペア）には挿入しない（不要かつ \n 前の空白消去になる）。
export function filterBareCarriageReturnForDisplay(id, bytes) {
  const t = terminals.get(id);
  if (!t) return bytes;
  const carry = t.crFilterCarry || new Uint8Array(0);
  const combined = new Uint8Array(carry.length + bytes.length);
  combined.set(carry, 0);
  combined.set(bytes, carry.length);

  const EL = asciiBytes('\x1b[K');
  const out: number[] = [];
  for (let i = 0; i < combined.length; i++) {
    out.push(combined[i]);
    if (combined[i] === 0x0D) {  // \r
      if (i + 1 < combined.length) {
        if (combined[i + 1] !== 0x0A) {  // 直後が \n でなければ EL を挿入
          for (const b of EL) out.push(b);
        }
      } else {
        // チャンク末尾の \r は次チャンクの先頭が \n かどうか未確定のため carry に残す
        t.crFilterCarry = combined.slice(i);
        return new Uint8Array(out.slice(0, out.length - 1));
      }
    }
  }
  t.crFilterCarry = new Uint8Array(0);
  return new Uint8Array(out);
}

export function writePTYChunk(id, term, bytes, onFlush) {
  const hasScreenClearSeq = detectScreenClearSeqForAutoScroll(id, bytes);
  const displayBytes = filterSynchronizedUpdateForDisplay(id, filterBareCarriageReturnForDisplay(id, filterCursorHideShowBlocksForDisplay(id, filterEraseScrollbackForDisplay(id, filterReverseVideoForDisplay(id, filterHubMarkersForDisplay(id, bytes))))));
  const wrappedFlush = () => {
    if (hasScreenClearSeq) snapToBottomAfterScreenClear(id);
    if (onFlush) onFlush();
  };
  if (displayBytes.length === 0) {
    wrappedFlush();
    return;
  }
  if (typeof term.writeUtf8 === 'function') {
    term.writeUtf8(displayBytes, wrappedFlush);
    return;
  }
  term.write(utf8Decoder.decode(displayBytes, { stream: true }), wrappedFlush);
}

// ---- バッファスキャン共通 ----

export function scanBuffer(id, limit?: number) {
  const t = terminals.get(id);
  if (!t || !t.term.buffer) return [];
  const buf = t.term.buffer.active;
  const start = (limit != null) ? Math.max(0, buf.length - limit) : 0;
  const lines = [];
  for (let i = start; i < buf.length; i++) {
    lines.push(buf.getLine(i)?.translateToString(true) || '');
  }
  return lines;
}

// ---- resize ----

export function sendResize(sessionId, cols, rows) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: 'pty_resize', session_id: sessionId, cols, rows }));
  }
}

export function applyRemotePtyResize(sessionId, cols, rows) {
  const id = Number(sessionId || 0);
  const nextCols = Number(cols || 0);
  const nextRows = Number(rows || 0);
  if (!id || nextCols <= 0 || nextRows <= 0) return;
  const t = terminals.get(id);
  if (!t || !t.term) return;
  if (t.term.cols === nextCols && t.term.rows === nextRows) return;
  const wasAtBottom = isTerminalAtBottom(t) || t.autoScroll;
  try {
    t.term.resize(nextCols, nextRows);
    if (wasAtBottom) {
      t.autoScroll = true;
      t.term.scrollToBottom();
      syncViewportScrollbarToBottom(t);
      if (id === activeSessionId) updateScrollLockBtn(false);
    }
  } catch (_) {}
}

export function canFitTerminal(t) {
  if (!t || !t.container || !t.container.isConnected) return false;
  if (!t.term || !t.term.element || !t.term.element.isConnected) return false;
  // display:none などで非表示状態だと offsetParent が null になり、幅も 0 になる。
  // この状態で fitAddon.fit() を呼ぶと cols が 1 桁台に潰れ、表示復帰後も narrow なまま残るので除外。
  if (t.container.offsetWidth <= 0 || t.container.offsetHeight <= 0) return false;
  return true;
}

export let lastDevicePixelRatio = window.devicePixelRatio || 1;
export let suppressPtyResizeUntil = 0;

export function suppressPtyResizeForInputLayout(durationMs = 300) {
  suppressPtyResizeUntil = Math.max(suppressPtyResizeUntil, Date.now() + durationMs);
}

export function clearSuppressPtyResize() {
  suppressPtyResizeUntil = 0;
}

export function isPtyResizeSuppressed() {
  return Date.now() < suppressPtyResizeUntil;
}

export function refitAllTerminals(refreshRows = false) {
  terminals.forEach((t, id) => {
    if (!canFitTerminal(t)) return;
    const prevCols = t.term.cols;
    const prevRows = t.term.rows;
    fitTerminalPreservingBottom(t, id);
    if (refreshRows && t.term.rows > 0) {
      t.term.refresh(0, t.term.rows - 1);
    }
    if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
      sendResize(id, t.term.cols, t.term.rows);
    }
  });
}

export let _resizeRafPending = false;
export const resizeObserver = new ResizeObserver(() => {
  if (_resizeRafPending) return;
  _resizeRafPending = true;
  requestAnimationFrame(() => {
    _resizeRafPending = false;
    if (activeSessionId === null) return;
    const t = terminals.get(activeSessionId);
    if (!canFitTerminal(t)) return;
    const prevCols = t.term.cols;
    const prevRows = t.term.rows;
    const shouldFollowBottom = terminalBottomFollow.id === activeSessionId
      && lastTerminalManualScrollAt <= terminalBottomFollow.startedAt;
    if (shouldFollowBottom) {
      t.autoScroll = true;
    }
    fitTerminalPreservingBottom(t, activeSessionId);
    if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
      if (!isPtyResizeSuppressed()) {
        sendResize(activeSessionId, t.term.cols, t.term.rows);
      }
    }
    if (shouldFollowBottom) {
      try { t.term.scrollToBottom(); syncViewportScrollbarToBottom(t); } catch (_) {}
      updateScrollLockBtn(false);
    }
  });
});

export const termArea = document.getElementById('terminal-area');
if (termArea) resizeObserver.observe(termArea);

window.addEventListener('resize', () => {
  const dpr = window.devicePixelRatio || 1;
  if (Math.abs(dpr - lastDevicePixelRatio) < 0.001) return;
  lastDevicePixelRatio = dpr;
  refitAllTerminals(true);
});

// 別窓 Session Grid（detached-grid）と通常 Hub は同一セッションの PTY を共有する。
// 別窓側は自スロットの小さいサイズに PTY をフィットさせるため、別窓を操作した後
// 通常 Hub に戻ると PTY が縮んだまま（このウィンドウの xterm の cols/rows は変わって
// いないので resizeObserver は発火せず、PTY が再アサートされない）。
// ウィンドウがフォーカス/可視に戻ったタイミングで、アクティブセッションの正しい
// サイズへ PTY を取り戻す（local の cols/rows 変化に依らず無条件で sendResize する）。
export function reassertActivePtySize() {
  // detached-grid ウィンドウ自身では実行しない（PTY 主導権を奪い合わないため）。
  if (window.detachedGridManager) return;
  if (activeSessionId === null) return;
  if (isPtyResizeSuppressed()) return;
  const t = terminals.get(activeSessionId);
  if (!canFitTerminal(t)) return;
  fitTerminalPreservingBottom(t, activeSessionId);
  // PTY は別窓に縮められている可能性があるため、local 変化に関わらず再送する。
  sendResize(activeSessionId, t.term.cols, t.term.rows);
}

window.addEventListener('focus', reassertActivePtySize);
document.addEventListener('visibilitychange', () => {
  if (!document.hidden) reassertActivePtySize();
});
