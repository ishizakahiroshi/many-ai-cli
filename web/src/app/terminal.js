// Extracted from app.js. Keep classic-script global scope; no module wrapper.

// ---- xterm.js 管理 ----

const TERMINAL_SCROLLBACK_LINES = 2000;
const TERMINAL_PENDING_MAX_BYTES = 100_000;
const TERMINAL_PENDING_FLUSH_MAX_CHUNKS = 8;
const TERMINAL_PENDING_FLUSH_MAX_BYTES = 24_000;

function ensureTerminal(id) {
  if (terminals.has(id)) return;
  const provider = sessions.get(id)?.provider;
  const term = new Terminal({
    cursorBlink: false,
    scrollback: TERMINAL_SCROLLBACK_LINES,
    // xterm はセル幅ベースで描画するため、絵文字フォント混在でグリフが巨大化/崩れする環境がある。
    // 端末領域は等幅フォントのみを使い、見た目の安定性を優先する。
    // 'TerminalNarrowNum'（styles.css 定義）を先頭に置き、丸数字・ローマ数字だけを
    // 半角字形の等幅フォントへ振り分ける。範囲外の文字はスキップされ "MS Gothic" 以降に落ちる。
    fontFamily: '"TerminalNarrowNum", "MS Gothic", "BIZ UDGothic", "BIZ UDゴシック", "Segoe UI Symbol", "Cascadia Mono", "Cascadia Code", Consolas, "Courier New", monospace',
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
  const fitAddon = new FitAddon.FitAddon();
  term.loadAddon(fitAddon);
  if (typeof WebLinksAddon !== 'undefined') {
    const webLinks = new WebLinksAddon.WebLinksAddon((event, uri) => {
      event.preventDefault();
      window.open(uri, '_blank', 'noopener');
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
      callback(links);
    }
  });
  if (typeof Unicode11Addon !== 'undefined') {
    const u11 = new Unicode11Addon.Unicode11Addon();
    term.loadAddon(u11);
    term.unicode.activeVersion = '11';
  }
  terminals.set(id, {
    term,
    fitAddon,
    container: null,
    pendingChunks: [],
    pendingTotalBytes: 0,
    pendingFlushActive: false,
    pendingFlushSeq: 0,
    pendingTextTail: '',
    textDecoder: new TextDecoder('utf-8'),
    markerFilterCarry: new Uint8Array(0),
    screenClearSeqCarry: new Uint8Array(0),
    autoScroll: true,
    everAttached: false,
  });
}

function attachTerminal(id) {
  const area = document.getElementById('terminal-area');
  if (!area) return;
  const t = terminals.get(id);
  if (!t) return;
  if (t.container) {
    area.innerHTML = '';
    area.appendChild(t.container);
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
    });
    return;
  }
  const container = document.createElement('div');
  container.style.width = '100%';
  container.style.height = '100%';
  t.container = container;
  area.innerHTML = '';
  area.appendChild(container);
  whenLayoutReady(id, container);
}

function whenLayoutReady(id, container) {
  const t = terminals.get(id);
  if (!t) return;
  if (container.clientWidth > 0 && container.clientHeight > 0) {
    t.term.open(container);
    fitTerminalPreservingBottom(t, id);
    if (!t.scrollHandlerInstalled) {
      t.scrollHandlerInstalled = true;
      t.scrollDisposable = t.term.onScroll(() => {
        const atBottom = isTerminalAtBottom(t);
        t.autoScroll = atBottom;
        if (id === activeSessionId) updateScrollLockBtn(!atBottom);
      });
    }
    flushPending(id);
    t.everAttached = true;
    sendResize(id, t.term.cols, t.term.rows);
    container.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      const sel = t.term.getSelection();
      if (sel) copyCleanText(sel, e).catch(() => {});
    });
  } else {
    requestAnimationFrame(() => whenLayoutReady(id, container));
  }
}

function queuePendingTerminalChunk(id, bytes) {
  const t = terminals.get(id);
  if (!t || !bytes) return;
  t.pendingChunks.push(bytes);
  t.pendingTotalBytes = (t.pendingTotalBytes || 0) + bytes.length;
  while (t.pendingTotalBytes > TERMINAL_PENDING_MAX_BYTES && t.pendingChunks.length > 1) {
    t.pendingTotalBytes -= t.pendingChunks[0].length;
    t.pendingChunks.shift();
  }
}

function flushPending(id) {
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
  let i = 0;

  const finish = () => {
    const latest = terminals.get(id);
    if (!latest || latest.pendingFlushSeq !== seq) return;
    latest.pendingFlushActive = false;
    if (latest.pendingChunks.length > 0) {
      requestAnimationFrame(() => flushPending(id));
      return;
    }
    if (latest.autoScroll) latest.term.scrollToBottom();
    scheduleApprovalCheck(id);
  };

  const writeBatch = () => {
    const latest = terminals.get(id);
    if (!latest || latest.pendingFlushSeq !== seq) return;
    let writtenChunks = 0;
    let writtenBytes = 0;
    while (i < chunks.length &&
           writtenChunks < TERMINAL_PENDING_FLUSH_MAX_CHUNKS &&
           writtenBytes < TERMINAL_PENDING_FLUSH_MAX_BYTES) {
      const chunk = chunks[i];
      const isLast = i === chunks.length - 1;
      i++;
      writtenChunks++;
      writtenBytes += chunk.length;
      writePTYChunk(id, latest.term, chunk, isLast ? finish : undefined);
    }
    if (i < chunks.length) {
      requestAnimationFrame(writeBatch);
    }
  };

  requestAnimationFrame(writeBatch);
}

window.flushPendingTerminalChunks = flushPending;

function isTerminalAtBottom(t) {
  if (!t || !t.term || !t.term.buffer) return true;
  const buf = t.term.buffer.active;
  return buf.viewportY + t.term.rows >= buf.length;
}

function fitTerminalPreservingBottom(t, id) {
  if (!canFitTerminal(t)) return;
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
function isAlternateBuffer(t) {
  if (!t || !t.term || !t.term.buffer) return false;
  const active = t.term.buffer.active;
  return !!(active && active.type === 'alternate');
}

// alt buffer 中の wheel は PTY 側アプリ（Codex の TUI 等）に PgUp/PgDn として転送する。
// 戻り値: 転送した場合 true（呼び元はそれ以降の autoScroll 操作等をスキップ）。
// mouse tracking が ON の場合は xterm が wheel を mouse escape として送るため二重送信を避ける。
function forwardWheelToAltBuffer(sessionId, t, deltaY) {
  if (!isAlternateBuffer(t)) return false;
  try {
    const mode = t.term.modes && t.term.modes.mouseTrackingMode;
    if (mode && mode !== 'none') return true; // xterm 自身が送るのでこちらは何もしない（が転送扱いで他処理を抑止）
  } catch (_) {}
  const key = deltaY < 0 ? '\x1b[5~' : '\x1b[6~';
  try { sendText(sessionId, key); } catch (_) {}
  return true;
}

function isWheelTargetExcluded(target) {
  if (!(target instanceof Element)) return true;
  if (document.body && !document.body.contains(target)) return true;
  const input = document.getElementById('input');
  if (input && input.contains(target) && input.scrollHeight > input.clientHeight + 1) {
    return true;
  }
  if (target.closest('.card-actions')) return true;
  if (target.closest('[data-wheel-native]')) return true;
  if (target.closest('#settings-panel')) return true;
  // ファイルタブ（ツリー / プレビュー）はネイティブの wheel スクロールを使う。
  // これを除外しないと document レベルのリスナーがターミナルへ転送して preventDefault してしまい、
  // プレビューのスクロールが効かなくなる。
  if (target.closest('#files-tab-contents')) return true;
  return false;
}

function routeWheelToOpenSettingsPanel(e) {
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

function getWheelTargetSessionId(target) {
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

let lastTerminalManualScrollAt = 0;

function markTerminalManualScrollIntent() {
  lastTerminalManualScrollAt = Date.now();
}

function scrollTerminalToBottomSoon(id, opts = {}) {
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

function refitAndStickTerminalToBottomSoon(id, opts = {}) {
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

function refitAndStickTerminalToBottomAfterLayoutSettles(id, opts = {}) {
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

function revealApprovalPromptForSession(id) {
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

function refitActiveTerminalAfterLayout(stickToBottom) {
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
    if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
      sendResize(id, t.term.cols, t.term.rows);
    }
    if (stickToBottom) scrollTerminalToBottomSoon(id);
  });
}

function updateScrollLockBtn(_locked) {
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

const hubMarkerBytePatterns = [
  new TextEncoder().encode('[ANY-AI-CLI]'),
  new TextEncoder().encode('[/ANY-AI-CLI]'),
];
const hubMarkerEndBytes = hubMarkerBytePatterns[1];
const eraseDisplayBelowBytes = new TextEncoder().encode('\x1b[J');
const screenClearSeqBytePatterns = [
  asciiBytes('\x1b[2J'),
  asciiBytes('\x1b[3J'),
  asciiBytes('\x1b[H'),
  asciiBytes('\x1b[0;0H'),
  asciiBytes('\x1b[1;1H'),
  asciiBytes('\x1b[?1049h'),
  asciiBytes('\x1b[?1049l'),
];
const screenClearSeqCarryLength = Math.max(...screenClearSeqBytePatterns.map(pattern => pattern.length)) - 1;

function bytesStartWith(bytes, offset, pattern) {
  if (offset + pattern.length > bytes.length) return false;
  for (let i = 0; i < pattern.length; i++) {
    if (bytes[offset + i] !== pattern[i]) return false;
  }
  return true;
}

function isPossibleMarkerPrefix(bytes, offset) {
  const remaining = bytes.length - offset;
  return hubMarkerBytePatterns.some((pattern) => {
    if (remaining >= pattern.length) return false;
    for (let i = 0; i < remaining; i++) {
      if (bytes[offset + i] !== pattern[i]) return false;
    }
    return true;
  });
}

function filterHubMarkersForDisplay(id, bytes) {
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

function asciiBytes(str) {
  const bytes = new Uint8Array(str.length);
  for (let i = 0; i < str.length; i++) bytes[i] = str.charCodeAt(i) & 0xff;
  return bytes;
}

function filterReverseVideoForDisplay(id, bytes) {
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

function detectScreenClearSeqForAutoScroll(id, bytes) {
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

function snapToBottomAfterScreenClear(id) {
  const t = terminals.get(id);
  if (!t || !t.autoScroll) return;
  t.term.scrollToBottom();
  if (id === activeSessionId) updateScrollLockBtn(false);
}

function writePTYChunk(id, term, bytes, onFlush) {
  const hasScreenClearSeq = detectScreenClearSeqForAutoScroll(id, bytes);
  const displayBytes = filterReverseVideoForDisplay(id, filterHubMarkersForDisplay(id, bytes));
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

function scanBuffer(id, limit) {
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

function sendResize(sessionId, cols, rows) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: 'pty_resize', session_id: sessionId, cols, rows }));
  }
}

function canFitTerminal(t) {
  if (!t || !t.container || !t.container.isConnected) return false;
  if (!t.term || !t.term.element || !t.term.element.isConnected) return false;
  // display:none などで非表示状態だと offsetParent が null になり、幅も 0 になる。
  // この状態で fitAddon.fit() を呼ぶと cols が 1 桁台に潰れ、表示復帰後も narrow なまま残るので除外。
  if (t.container.offsetWidth <= 0 || t.container.offsetHeight <= 0) return false;
  return true;
}

let lastDevicePixelRatio = window.devicePixelRatio || 1;

function refitAllTerminals(refreshRows = false) {
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

let _resizeRafPending = false;
const resizeObserver = new ResizeObserver(() => {
  if (_resizeRafPending) return;
  _resizeRafPending = true;
  requestAnimationFrame(() => {
    _resizeRafPending = false;
    if (activeSessionId === null) return;
    const t = terminals.get(activeSessionId);
    if (!canFitTerminal(t)) return;
    const prevCols = t.term.cols;
    const prevRows = t.term.rows;
    fitTerminalPreservingBottom(t, activeSessionId);
    if (t.term.cols !== prevCols || t.term.rows !== prevRows) {
      sendResize(activeSessionId, t.term.cols, t.term.rows);
    }
  });
});

const termArea = document.getElementById('terminal-area');
if (termArea) resizeObserver.observe(termArea);

window.addEventListener('resize', () => {
  const dpr = window.devicePixelRatio || 1;
  if (Math.abs(dpr - lastDevicePixelRatio) < 0.001) return;
  lastDevicePixelRatio = dpr;
  refitAllTerminals(true);
});
