// Pure approval parsers. Keep classic-script compatibility; no module wrapper.
(function (root) {
  'use strict';

  const reviewAnswersRe = /Review your answers/i;
  const readyToSubmitAnswersRe = /Ready to submit your answers/i;
  const tabBoxMarkerRe = /[◻□☐✓☑]/; // ◻ □ ☐ ✓ ☑
  const sequentialQuestionHeaderRe = /^\s*([A-Z]{1,3}\d{1,3}|Q\d{1,3}|問\d{1,3})\s*[:：]\s*(.+?)\s*$/i;
  const userSpecifiesRe = /user specifies|その他指定/i;
  const hubChoiceQuestionRe = /どれで進めますか|どれで進める|どちらで進め|どの選択肢|選択してください|how would you like to proceed|which option/i;
  const recommendedChoiceRe = /\(recommended\)|（recommended）|推奨/i;
  const approvalLabelRe = /\b(yes|no|allow|deny|proceed|abort|don[''']t ask|cancel|once|always|permission|confirm|details)\b/i;

  function isMultiQuestionPrompt(lines) {
    for (const line of lines || []) {
      if (!line) continue;
      if (reviewAnswersRe.test(line)) return true;
      if (readyToSubmitAnswersRe.test(line)) return true;
      if (line.indexOf('←') !== -1 && line.indexOf('→') !== -1 &&
          (tabBoxMarkerRe.test(line) || /\bSubmit\b/.test(line))) {
        return true;
      }
    }
    return false;
  }

  function extractSequentialChoicePrompts(lines) {
    const prompts = [];
    let current = null;
    const recent = (lines || []).slice(-80).map(line => String(line || '').trimEnd());
    for (const rawLine of recent) {
      const line = rawLine.trim();
      if (!line) continue;
      if (/\[ANY-AI-CLI\]|\[\/ANY-AI-CLI\]/.test(line)) return null;

      const hm = line.match(sequentialQuestionHeaderRe);
      if (hm) {
        if (current && current.options.length >= 2) prompts.push(current);
        current = {
          key: hm[1].trim(),
          question: hm[2].trim(),
          options: [],
        };
        continue;
      }

      if (!current) continue;
      const om = line.match(/^\s*(\d{1,2})\.\s*(.+?)\s*$/);
      if (om) {
        current.options.push({
          num: parseInt(om[1], 10),
          label: om[2].trim(),
          isCurrent: current.options.length === 0,
        });
        continue;
      }
      if (/^\s*N\.\s*(User specifies|その他指定)/i.test(line)) continue;

      if (current.options.length > 0 && !/^\s{2,}/.test(rawLine)) {
        if (current.options.length >= 2) prompts.push(current);
        current = null;
      }
    }
    if (current && current.options.length >= 2) prompts.push(current);

    const unique = [];
    const seen = new Set();
    for (const prompt of prompts) {
      const key = `${prompt.key}:${prompt.question}`;
      if (seen.has(key)) continue;
      seen.add(key);
      prompt.options.sort((a, b) => a.num - b.num);
      unique.push(prompt);
    }
    return unique.length >= 2 ? unique : null;
  }

  function isBatchOptions(value) {
    return Array.isArray(value) && value.length > 0 &&
      value[0] && Array.isArray(value[0].options);
  }

  function approvalSig(options) {
    if (isBatchOptions(options)) {
      return JSON.stringify(options.map(s => ({
        n: s.num,
        t: String(s.title || '').replace(/\s+/g, ' ').slice(0, 80),
        o: (s.options || []).map(o => `${o.num}:${String(o.label || '').trim().replace(/\s+/g, ' ').slice(0, 80)}`),
      })));
    }
    return JSON.stringify((options || []).map(o => {
      const lbl = String(o.label || '').trim().replace(/\s+/g, ' ').slice(0, 80);
      const ctx = o && o._ctx ? `|${o._ctx}` : '';
      return `${o.num}:${lbl}${ctx}`;
    }));
  }

  function approvalCtxHash(s) {
    const text = String(s || '').replace(/\s+/g, ' ').trim();
    let h = 5381;
    for (let i = 0; i < text.length; i++) {
      h = (((h << 5) + h) + text.charCodeAt(i)) | 0;
    }
    return (h >>> 0).toString(36);
  }

  function sequentialChoiceSig(prompts) {
    return approvalCtxHash((prompts || []).map(p => `${p.key}:${p.question}:${p.options.map(o => `${o.num}.${o.label}`).join('|')}`).join('\n'));
  }

  function hasYesNoApprovalMarker(text) {
    return /\(Y:1\/N:0\)/.test(String(text || ''));
  }

  function looksLikeYesNoQuestion(text) {
    const s = String(text || '');
    if (!hasYesNoApprovalMarker(s)) return false;
    const before = s.slice(0, s.lastIndexOf('(Y:1/N:0)'));
    return /[?？]\s*$/.test(before.trim()) || /[?？]/.test(before.slice(-120));
  }

  function yesNoCtxFromText(text) {
    const s = String(text || '');
    const idx = s.lastIndexOf('(Y:1/N:0)');
    const before = idx >= 0 ? s.slice(0, idx) : s;
    return before.slice(-200);
  }

  function yesNoApprovalOptions(ctxText) {
    const ctx = ctxText ? approvalCtxHash(ctxText) : '';
    return [
      { num: 1, label: 'Yes (1)', isCurrent: true, preserveOrder: true, _ctx: ctx },
      { num: 0, label: 'No (0)', isCurrent: false, preserveOrder: true, _ctx: ctx },
    ];
  }

  function extractHubMarkerApproval(lines) {
    const source = Array.isArray(lines) ? lines : [];
    const searchStart = Math.max(0, source.length - 40);
    const recentText = source.slice(searchStart).join('\n');
    const blockRe = /\[ANY-AI-CLI\]([\s\S]*?)\[\/ANY-AI-CLI\]/g;
    let match;
    let lastBlock = null;
    while ((match = blockRe.exec(recentText)) !== null) {
      lastBlock = match[1];
    }
    if (lastBlock !== null) {
      const inner = lastBlock.split('\n').map(l => l.trim()).filter(Boolean);
      return parseHubBlock(inner);
    }

    let closeIdx = -1;
    let openIdx = -1;
    for (let i = source.length - 1; i >= searchStart; i--) {
      const line = source[i];
      if (/\[ANY-AI-CLI\]/.test(line) && /\[\/ANY-AI-CLI\]/.test(line)) {
        const inner = line.replace(/^[\s\S]*?\[ANY-AI-CLI\]/, '').replace(/\[\/ANY-AI-CLI\][\s\S]*$/, '').trim();
        return parseHubBlock([inner]);
      }
      if (/\[\/ANY-AI-CLI\]/.test(line) && closeIdx === -1) { closeIdx = i; continue; }
      if (/\[ANY-AI-CLI\]/.test(line) && closeIdx !== -1) { openIdx = i; break; }
    }

    if (openIdx === -1 || closeIdx === -1) return null;
    const inner = source.slice(openIdx + 1, closeIdx).map(l => l.trim()).filter(Boolean);
    return parseHubBlock(inner);
  }

  function extractPlainYesNoApproval(lines) {
    const source = Array.isArray(lines) ? lines : [];
    const searchStart = Math.max(0, source.length - 20);
    const recentLines = source.slice(searchStart).map(line => String(line || '').trim()).filter(Boolean);
    for (let i = source.length - 1; i >= searchStart; i--) {
      const line = String(source[i] || '').trim();
      if (!line) continue;
      if (/\[ANY-AI-CLI\]|\[\/ANY-AI-CLI\]/.test(line)) continue;
      if (looksLikeYesNoQuestion(line)) return yesNoApprovalOptions(yesNoCtxFromText(line));
    }
    const recentText = recentLines.join('\n');
    if (!/\[ANY-AI-CLI\]|\[\/ANY-AI-CLI\]/.test(recentText) && looksLikeYesNoQuestion(recentText)) {
      return yesNoApprovalOptions(yesNoCtxFromText(recentText));
    }
    return null;
  }

  // Ink 等の TUI 再描画では、画面幅を超える長い選択肢が折り返される際に
  // 実際の改行コードが入らず、次の質問見出し行が直前の行へ連結されることがある。
  // 連結されると見出しが行頭でなくなり、parseHubBlock が新しい質問と認識できず
  // 2つの質問が1つに合体する（一括承認パネルの件数・ボタン文字列が壊れる）。
  // 各質問末尾の「N. User specifies」を区切りアンカーとして、行内に埋もれた
  // 「N行」および後続の見出しを元の行構造へ再分割する。
  function ungluedMarkerLines(lines) {
    // 「N. User specifies / その他指定」を区切りアンカーにする（行頭・行中問わず）。
    // \b 直前判定で "PLAN." 等の語中 N は誤マッチしない。
    const splitRe = /\s*\b(N\.[ \t]*(?:User specifies|その他指定))\b\s*/i;
    const out = [];
    const expand = (line) => {
      const m = line.match(splitRe);
      if (!m) { out.push(line); return; }
      const before = line.slice(0, m.index).trim();         // 連結されていた選択肢/本文
      const after = line.slice(m.index + m[0].length).trim(); // 後続の見出し等
      if (before) out.push(before);
      out.push(m[1]);                                        // N. User specifies
      if (after) expand(after);                             // 後続をさらに分割
    };
    for (const raw of (lines || [])) expand(String(raw || ''));
    return out;
  }

  function parseHubBlock(rawLines) {
    const lines = ungluedMarkerLines(rawLines);
    const text = lines.join('\n');
    if (hasYesNoApprovalMarker(text)) {
      return yesNoApprovalOptions(yesNoCtxFromText(text));
    }
    const sections = [];
    const looseOpts = [];
    let cur = null;
    const optionRe = /^(\d+)\.\s*(.+?)\s*$/;
    const headingRe = /^(\d+)\s+(.+?)\s*$/;
    for (const raw of lines) {
      const line = String(raw || '').trim();
      if (!line) continue;
      const om = line.match(optionRe);
      if (om) {
        const opt = { num: parseInt(om[1], 10), label: om[2].trim(), isCurrent: false };
        if (cur) cur.options.push(opt);
        else looseOpts.push(opt);
        continue;
      }
      const hm = line.match(headingRe);
      if (hm) {
        cur = { num: parseInt(hm[1], 10), title: hm[2].trim(), options: [] };
        sections.push(cur);
        continue;
      }
    }
    const filledSections = sections.filter(s => s.options.length > 0);
    if (filledSections.length >= 2) return filledSections;
    if (filledSections.length === 1) return filledSections[0].options;
    return looseOpts.length > 0 ? looseOpts : null;
  }

  function shortcutSendText(label) {
    const m = String(label || '').match(/\((y|p|n|!|#|\?|esc|escape)\)\s*$/i);
    if (!m) return null;
    const key = m[1].toLowerCase();
    if (key === 'esc' || key === 'escape') return '\x1b';
    return key;
  }

  function buildApprovalOption(numText, labelText, isCurrent) {
    const label = String(labelText || '').trim()
      .replace(/\s{2,}.*$/, '')
      .replace(/\s*\d+\.\s*[A-Za-z].*$/, '')
      .trim();
    const opt = { num: parseInt(numText, 10), label, isCurrent: !!isCurrent };
    const sendText = shortcutSendText(label);
    if (sendText) opt._sendText = sendText;
    return opt;
  }

  function approvalContextLines(lines, cluster, margin = 10) {
    if (!cluster) return lines;
    return lines.slice(Math.max(0, cluster.start - margin), Math.min(lines.length, cluster.end + margin + 1));
  }

  function matchNativeApprovalTrigger(line) {
    if (!line) return false;
    const lower = String(line).toLowerCase();
    return lower.includes('requires approval') ||
      lower.includes('would you like to run the following command') ||
      lower.includes('would you like to run') ||
      lower.includes('do you want to proceed?') ||
      lower.includes('this command requires approval') ||
      lower.includes('permission required') ||
      lower.includes('permissions required') ||
      lower.includes('requires permission') ||
      lower.includes('requires confirmation') ||
      lower.includes('prompts for user confirmation') ||
      lower.includes('allow all similar') ||
      lower.includes('deny all similar') ||
      (lower.includes('press enter to confirm') && !lower.includes('esc to go back')) ||
      lower.includes('enter to select') ||
      lower.includes('↑/↓ to navigate') ||
      lower.includes('esc to cancel');
  }

  function extractApprovalOptions(tail) {
    const options = [];
    let clusterStart = -1;
    let clusterEnd = -1;
    let seenOption = false;
    let blankGap = 0;
    let pendingContinuation = [];
    const maxBlankGap = 4;
    const consumeContinuations = (label) => {
      if (pendingContinuation.length === 0) return label;
      const suffix = pendingContinuation.slice().reverse().join(' ');
      pendingContinuation = [];
      return `${label} ${suffix}`.replace(/\s+/g, ' ').trim();
    };
    for (let i = (tail || []).length - 1; i >= 0; i--) {
      const line = tail[i];
      const cm = String(line || '').match(/^\s*[>❯›❱]\s*(\d{1,2})\.\s*(.+?)\s*$/);
      if (cm) {
        options.unshift(buildApprovalOption(cm[1], consumeContinuations(cm[2]), true));
        if (clusterEnd === -1) clusterEnd = i;
        clusterStart = i;
        seenOption = true;
        blankGap = 0;
        continue;
      }
      const om = String(line || '').match(/^\s*(\d{1,2})\.\s*(.+?)\s*$/);
      if (om) {
        options.unshift(buildApprovalOption(om[1], consumeContinuations(om[2]), false));
        if (clusterEnd === -1) clusterEnd = i;
        clusterStart = i;
        seenOption = true;
        blankGap = 0;
        continue;
      }
      if (!seenOption) continue;
      if (!String(line || '').trim()) {
        blankGap++;
        if (blankGap > maxBlankGap) break;
        pendingContinuation = [];
        continue;
      }
      if (blankGap === 0 && /^\s+\S/.test(line)) {
        pendingContinuation.push(line.trim());
        continue;
      }
      break;
    }
    if (options.length === 0) return { options: [], cluster: null };
    const nums = options.map(opt => opt.num);
    const numMin = Math.min(...nums);
    const numMax = Math.max(...nums);
    if (numMax > 20 || numMax - numMin > 15 || options.length > 12) {
      return { options: [], cluster: null };
    }
    const seen = new Set();
    const uniqueOptions = options.filter(opt => {
      const key = `${opt.num}:${opt.label}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
    if (uniqueOptions.length < 2) return { options: [], cluster: null };
    return {
      options: uniqueOptions,
      cluster: { start: clusterStart, end: clusterEnd },
    };
  }

  function approvalLineHasHint(provider, line, providerTriggerMatcher) {
    const matchProvider = typeof providerTriggerMatcher === 'function'
      ? providerTriggerMatcher
      : (typeof root.matchProviderApprovalTrigger === 'function' ? root.matchProviderApprovalTrigger : null);
    return userSpecifiesRe.test(line) ||
      recommendedChoiceRe.test(line) ||
      (matchProvider ? matchProvider(provider, line) : false) ||
      matchNativeApprovalTrigger(line);
  }

  function approvalLinesHaveHint(provider, lines, providerTriggerMatcher) {
    return (lines || []).some(line => approvalLineHasHint(provider, line, providerTriggerMatcher));
  }

  function isHubChoicePrompt(contextLines, options) {
    if (!options.length) return false;
    const hasPrompt = contextLines.some(line => hubChoiceQuestionRe.test(line));
    const hasChoiceMarker = contextLines.some(line => userSpecifiesRe.test(line)) ||
      options.some(opt => userSpecifiesRe.test(opt.label) || recommendedChoiceRe.test(opt.label));
    return hasPrompt && hasChoiceMarker;
  }

  function markHubChoiceDefault(options, contextLines) {
    if (options.some(o => o.isCurrent) || !isHubChoicePrompt(contextLines, options)) return;
    const recommended = options.find(o => recommendedChoiceRe.test(o.label)) || options.find(o => o.num === 1) || options[0];
    if (recommended) recommended.isCurrent = true;
  }

  function hasApprovalLikeLabel(options) {
    return (options || []).some((opt) => approvalLabelRe.test(opt.label));
  }

  const api = {
    lineHasHint: approvalLineHasHint,
    linesHaveHint: approvalLinesHaveHint,
    approvalLineHasHint,
    approvalLinesHaveHint,
    extractHubMarkerApproval,
    extractPlainYesNoApproval,
    extractSequentialChoicePrompts,
    extractApprovalOptions,
    approvalContextLines,
    approvalSig,
    sequentialChoiceSig,
    isBatchOptions,
    isMultiQuestionPrompt,
    isHubChoicePrompt,
    markHubChoiceDefault,
    matchNativeApprovalTrigger,
    hasApprovalLikeLabel,
    userSpecifiesRe,
  };

  root.approvalParser = api;
  Object.assign(root, api);
  root._approvalCtxHash = approvalCtxHash;

  if (typeof module !== 'undefined' && module.exports) {
    module.exports = api;
  }
})(typeof window !== 'undefined' ? window : globalThis);

// --- ESM re-exports from the IIFE-published approval parser API (generated) ---
const __esmRoot = (typeof window !== 'undefined') ? window : globalThis;
export const approvalParser = __esmRoot.approvalParser;
export const {
  lineHasHint, linesHaveHint, approvalLineHasHint, approvalLinesHaveHint, extractHubMarkerApproval, extractPlainYesNoApproval, extractSequentialChoicePrompts, extractApprovalOptions, approvalContextLines, isBatchOptions, isMultiQuestionPrompt, isHubChoicePrompt, markHubChoiceDefault, matchNativeApprovalTrigger, hasApprovalLikeLabel, userSpecifiesRe,
} = __esmRoot.approvalParser;
