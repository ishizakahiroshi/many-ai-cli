// --- ESM imports (generated) ---
import { copyCleanText, copyOneLineText } from './util.js';
import { t as ti18n } from '../i18n.js';
import { FONTSIZE_MAP, STORAGE_FONTSIZE_KEY } from './user-prefs.js';
import { activeSessionId, approvalRawOptionsCache, approvalVisibleCache, sessions, terminals, utf8Decoder } from './state.js';
import { sendText } from '../app.js';
import { ABS_UNIX_PATH_RE, ABS_WIN_PATH_RE, REL_PATH_RE, isLikelyRelPath, isTerminalPathStartBoundary, resolveTerminalPathCandidate, scheduleHidePathPopup, showPathPopup, trimTerminalPathCandidate } from './path-links.js';
import { ws } from './ws-client.js';
import { scheduleApprovalCheck } from './approval.js';
import { handleCrunchLinkClick } from './expand-popup.js';
import { resetHistoryViewerForSessionChange, updateHistoryHint } from './history-viewer.js';

// Claude Code の折りたたみマーカー: "… +23 lines (ctrl+o to expand)"
const CRUNCH_LINK_RE = /[…\.]{1,3}\s*\+\d+\s*lines?\s*\(ctrl\+o to expand\)/gi;

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
    lineHeight: 1.25,
    windowsPty: { backend: 'conpty' },
    // cursor を背景色と同色にしてブロックカーソルを不可視化する。
    // 'transparent' はブラウザ実装によって輪郭線だけ □ として描画されることがある。
    theme: { background: '#0d1117', cursor: '#0d1117', cursorAccent: '#e6edf3' },
    disableStdin: true,
    allowProposedApi: true,
  });
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
        t.autoScroll = atBottom;
        if (id === activeSessionId) updateScrollLockBtn(!atBottom);
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
  mkItem('term_ctx_copy', 'コピー', '⎘', (sel, anchor) => copyCleanText(sel, anchor).catch(() => {}));
  mkItem('term_ctx_copy_oneline', '1行コピー', '⇥', (sel, anchor) => copyOneLineText(sel, anchor).catch(() => {}));
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
    if (t.autoScroll) t.term.scrollToBottom();
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
    if (latest.autoScroll) latest.term.scrollToBottom();
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

export function fitTerminalPreservingBottom(t, id) {
  if (!canFitTerminal(t)) return;
  if (isPtyResizeSuppressed()) return;
  const wasAtBottom = isTerminalAtBottom(t) || t.autoScroll;
  t.fitAddon.fit();
  if (wasAtBottom) {
    t.autoScroll = true;
    t.term.scrollToBottom();
    if (id === activeSessionId) updateScrollLockBtn(false);
  }
}

// xterm が alternate screen buffer（TUI モード, Codex 等）に居るかを判定。
// alt buffer は scrollback を持たないため term.scrollLines は no-op となり、
export function isAlternateBuffer(t) {
  if (!t || !t.term || !t.term.buffer) return false;
  const active = t.term.buffer.active;
  return !!(active && active.type === 'alternate');
}

// alt buffer 中の wheel は PTY 側アプリ（Codex の TUI 等）に PgUp/PgDn として転送する。
// 戻り値: 転送した場合 true（呼び元はそれ以降の autoScroll 操作等をスキップ）。
// mouse tracking が ON の場合は xterm が wheel を mouse escape として送るため二重送信を避ける。
export function forwardWheelToAltBuffer(sessionId, t, deltaY) {
  if (!isAlternateBuffer(t)) return false;
  try {
    const mode = t.term.modes && t.term.modes.mouseTrackingMode;
    if (mode && mode !== 'none') return true; // xterm 自身が送るのでこちらは何もしない（が転送扱いで他処理を抑止）
  } catch (_) {}
  const key = deltaY < 0 ? '\x1b[5~' : '\x1b[6~';
  try { sendText(sessionId, key); } catch (_) {}
  return true;
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

  // マウスがチャット履歴ペイン上にある場合: 最近傍のスクロール可能要素へ明示スクロール
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
    fitTerminalPreservingBottom(t, id);
    if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
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
}

document.getElementById('scroll-to-top-btn')?.addEventListener('click', () => {
  if (activeSessionId === null) return;
  const t = terminals.get(activeSessionId);
  if (!t) return;
  markTerminalManualScrollIntent();
  t.autoScroll = false;
  t.term.scrollToTop();
  updateScrollLockBtn(true);
});

document.getElementById('scroll-to-bottom-btn')?.addEventListener('click', () => {
  if (activeSessionId === null) return;
  const t = terminals.get(activeSessionId);
  if (!t) return;
  t.autoScroll = true;
  t.term.scrollToBottom();
});

export const hubMarkerBytePatterns = [
  new TextEncoder().encode('[ANY-AI-CLI]'),
  new TextEncoder().encode('[/ANY-AI-CLI]'),
];
export const hubMarkerEndBytes = hubMarkerBytePatterns[1];
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

export function bytesStartWith(bytes, offset, pattern) {
  if (offset + pattern.length > bytes.length) return false;
  for (let i = 0; i < pattern.length; i++) {
    if (bytes[offset + i] !== pattern[i]) return false;
  }
  return true;
}

export function isPossibleMarkerPrefix(bytes, offset) {
  const remaining = bytes.length - offset;
  return hubMarkerBytePatterns.some((pattern) => {
    if (remaining >= pattern.length) return false;
    for (let i = 0; i < remaining; i++) {
      if (bytes[offset + i] !== pattern[i]) return false;
    }
    return true;
  });
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
  while (i < combined.length) {
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

export function snapToBottomAfterScreenClear(id) {
  const t = terminals.get(id);
  if (!t || !t.autoScroll) return;
  t.term.scrollToBottom();
  if (id === activeSessionId) updateScrollLockBtn(false);
}

export function writePTYChunk(id, term, bytes, onFlush) {
  const hasScreenClearSeq = detectScreenClearSeqForAutoScroll(id, bytes);
  const displayBytes = filterSynchronizedUpdateForDisplay(id, filterReverseVideoForDisplay(id, filterHubMarkersForDisplay(id, bytes)));
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
      try { t.term.scrollToBottom(); } catch (_) {}
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
