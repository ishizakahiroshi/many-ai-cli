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
  const yesNoApprovalMarkerRe = /[（(]\s*[YＹ]\s*[:：]\s*1\s*[\/／]\s*[NＮ]\s*[:：]\s*0\s*[）)]/ig;

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
      if (/\[MANY-AI-CLI\]|\[\/MANY-AI-CLI\]/.test(line)) return null;

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

  function isMultiSelectOptions(value) {
    return Array.isArray(value) && value.length > 0 &&
      value[0] && value[0]._multiSelect === true;
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
    yesNoApprovalMarkerRe.lastIndex = 0;
    return yesNoApprovalMarkerRe.test(String(text || ''));
  }

  function lastYesNoApprovalMarkerIndex(text) {
    const s = String(text || '');
    yesNoApprovalMarkerRe.lastIndex = 0;
    let idx = -1;
    let match;
    while ((match = yesNoApprovalMarkerRe.exec(s)) !== null) {
      idx = match.index;
      if (match[0].length === 0) yesNoApprovalMarkerRe.lastIndex++;
    }
    return idx;
  }

  function yesNoQuestionText(text) {
    const s = String(text || '');
    const idx = lastYesNoApprovalMarkerIndex(s);
    if (idx < 0) return '';
    return s.slice(0, idx).replace(/\[\/?MANY-AI-CLI\]/g, '').replace(/\s+/g, ' ').trim();
  }

  function isPlaceholderYesNoQuestion(text) {
    return /^question\s*\d*\s*[?？]$/i.test(yesNoQuestionText(text));
  }

  function looksLikeYesNoQuestion(text) {
    const s = String(text || '');
    if (!hasYesNoApprovalMarker(s)) return false;
    if (isPlaceholderYesNoQuestion(s)) return false;
    const before = yesNoQuestionText(s);
    return /[?？]\s*$/.test(before.trim()) || /[?？]/.test(before.slice(-120));
  }

  function yesNoCtxFromText(text) {
    return yesNoQuestionText(text).slice(-200);
  }

  function yesNoApprovalOptions(ctxText) {
    const ctx = ctxText ? approvalCtxHash(ctxText) : '';
    return [
      { num: 1, label: 'Yes (1)', isCurrent: true, preserveOrder: true, _ctx: ctx },
      { num: 0, label: 'No (0)', isCurrent: false, preserveOrder: true, _ctx: ctx },
    ];
  }

  // 承認ブロック直前の地の文（前置き説明）を取り出す。ポップアップ先頭に表示し、
  // ユーザーが判断の文脈を見るために CLI 本体をスクロールせずに済むようにするためのもの。
  // 境界の決め方:
  //  ① 直前の確定ブロック（前の質問の `[/MANY-AI-CLI]` / 完了 `[/MANY-AI-CLI-DONE]`）以降だけを対象にする
  //  ② 2 行以上連続する空行（大きな段落区切り）を上端の境界にする（段落間の単一空行は保持）
  //  ③ 上限 PREAMBLE_MAX_LINES 行（長文の前置きでポップアップが肥大しないよう保険。表示側はスクロール可）
  const PREAMBLE_MAX_LINES = 40;
  function preambleBeforeMarker(textBefore) {
    let s = String(textBefore || '');
    const lastClose = Math.max(s.lastIndexOf('[/MANY-AI-CLI]'), s.lastIndexOf('[/MANY-AI-CLI-DONE]'));
    if (lastClose !== -1) {
      const nl = s.indexOf('\n', lastClose);
      s = nl === -1 ? '' : s.slice(nl + 1);
    }
    // 対応の取れなかったマーカー片が残っていても表示に混ぜない。
    s = s.replace(/\[\/?MANY-AI-CLI(?:-DONE)?\]/g, '');
    let lines = s.split('\n').map(l => l.replace(/\s+$/g, ''));
    // 末尾(画面では直上)から遡り、最初に現れる「2 行連続の空行」を上端境界にする。
    let cut = 0;
    for (let i = lines.length - 1; i >= 1; i--) {
      if (!lines[i].trim() && !lines[i - 1].trim()) { cut = i + 1; break; }
    }
    lines = lines.slice(cut);
    while (lines.length && !lines[0].trim()) lines.shift();
    while (lines.length && !lines[lines.length - 1].trim()) lines.pop();
    if (lines.length > PREAMBLE_MAX_LINES) lines = lines.slice(-PREAMBLE_MAX_LINES);
    return lines.join('\n').trim();
  }

  function extractHubMarkerApproval(lines) {
    const source = Array.isArray(lines) ? lines : [];
    // [MANY-AI-CLI]…[/MANY-AI-CLI] を「末尾優先の完全ブロック」として取り出す。
    // ブロックの行数に依存させない（固定窓で切らない）のが要点 — 複数質問一括や #multi で
    // 選択肢が増え、長い日本語ラベルが端末幅で折り返されてブロックが何十行になっても、
    // 開きマーカーが窓から外れて末尾の質問だけ拾う事故を構造的に防ぐ。マーカーは明示
    // デリミタ済みで scrollback 誤検出の懸念がないため source 全体を走査してよい。
    // 取りこぼしの実質的な上限は呼び出し側が保持する pendingTextTail の文字数
    // （APPROVAL_PENDING_TEXT_TAIL_LIMIT）のみ＝制約をそこ一点に集約する。
    // ブロック全文ハッシュを各選択肢へ _blockSig として付与する。
    // 「回答済みの質問は二度と承認 UI を出さない」恒久抑制（answeredMarkerSigs）のキーに使う。
    // 質問文＋全選択肢を含むため、ラベルが同一でも別質問なら別ハッシュになり誤抑制しない。
    // approvalCtxHash は空白を正規化するので、端末幅による折り返し差は吸収される。
    // 承認ブロック直前の地の文（前置き説明）を parsed 配列へ _preamble として添付する。
    // ポップアップ先頭に表示して、ユーザーが判断の文脈を見るために CLI をスクロールせずに済むようにする。
    // 配列プロパティなので approvalSig（要素のみを見る）には影響せず、誤抑制・余計な再描画を招かない。
    const withBlockSig = (parsed, innerArr, preamble) => {
      if (parsed && Array.isArray(parsed)) {
        const sig = approvalCtxHash((innerArr || []).join('\n'));
        for (const el of parsed) { if (el && typeof el === 'object') el._blockSig = sig; }
        if (preamble) (parsed as any)._preamble = preamble;
      }
      return parsed;
    };

    const recentText = source.join('\n');
    const blockRe = /\[MANY-AI-CLI\]([\s\S]*?)\[\/MANY-AI-CLI\]/g;
    let match;
    let lastBlock = null;
    let lastOpenIdx = -1;
    while ((match = blockRe.exec(recentText)) !== null) {
      lastBlock = match[1];
      lastOpenIdx = match.index;
    }
    if (lastBlock !== null) {
      const inner = lastBlock.split('\n').map(l => l.trim()).filter(Boolean);
      const preamble = preambleBeforeMarker(recentText.slice(0, lastOpenIdx));
      return withBlockSig(parseHubBlock(inner), inner, preamble);
    }

    // 開き/閉じが別チャンクに割れて全文一致しなかった場合の末尾アンカー・フォールバック。
    // 同じく固定窓は使わず source 全体を末尾から遡って対の開きマーカーを探す。
    let closeIdx = -1;
    let openIdx = -1;
    for (let i = source.length - 1; i >= 0; i--) {
      const line = source[i];
      if (/\[MANY-AI-CLI\]/.test(line) && /\[\/MANY-AI-CLI\]/.test(line)) {
        const inner = line.replace(/^[\s\S]*?\[MANY-AI-CLI\]/, '').replace(/\[\/MANY-AI-CLI\][\s\S]*$/, '').trim();
        const preamble = preambleBeforeMarker(source.slice(0, i).join('\n'));
        return withBlockSig(parseHubBlock([inner]), [inner], preamble);
      }
      if (/\[\/MANY-AI-CLI\]/.test(line) && closeIdx === -1) { closeIdx = i; continue; }
      if (/\[MANY-AI-CLI\]/.test(line) && closeIdx !== -1) { openIdx = i; break; }
    }

    if (openIdx === -1 || closeIdx === -1) return null;
    const inner = source.slice(openIdx + 1, closeIdx).map(l => l.trim()).filter(Boolean);
    const preamble = preambleBeforeMarker(source.slice(0, openIdx).join('\n'));
    return withBlockSig(parseHubBlock(inner), inner, preamble);
  }

  function extractPlainYesNoApproval(lines) {
    const source = Array.isArray(lines) ? lines : [];
    const searchStart = Math.max(0, source.length - 20);
    const recentLines = source.slice(searchStart).map(line => String(line || '').trim()).filter(Boolean);
    for (let i = source.length - 1; i >= searchStart; i--) {
      const line = String(source[i] || '').trim();
      if (!line) continue;
      if (/\[MANY-AI-CLI\]|\[\/MANY-AI-CLI\]/.test(line)) continue;
      if (looksLikeYesNoQuestion(line)) return yesNoApprovalOptions(yesNoCtxFromText(line));
    }
    const recentText = recentLines.join('\n');
    if (!/\[MANY-AI-CLI\]|\[\/MANY-AI-CLI\]/.test(recentText) && looksLikeYesNoQuestion(recentText)) {
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
  function splitUserSpecifiesAnchor(lines) {
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

  // Ink のカーソル位置制御描画では選択肢間の改行も失われ、
  // 「1. … 2. … 3. …」が1行へ連結されることがある（pendingTextTail の split で1要素化）。
  // この状態だと行頭正規表現が先頭の「1.」しか拾えず、残り全部が1個目のラベルへ飲み込まれて
  // 承認ボタンが1つに潰れる（=「ボタンが全部一緒になる」症状）。
  // 行内に「<番号>.」が 1→2→3 と単調増加で連続する場合のみ、各番号の直前で分割する。
  // 誤分割防止: ① 直前は行頭/空白/閉じ括弧/文末記号のいずれか ② 「1.5」等の小数は除外
  //（ピリオド直後が空白か非数字のときだけ選択肢開始とみなす） ③ 連番でなければ分割しない。
  // 文末記号（。．？?！!…）も境界に含めるのは、見出しが「…ますか?」で終わり、続く選択肢が
  // 空白なしで「?1.」と連結されるケース（xterm ハードラップで改行と行頭空白が同時に落ちる）を
  // 救うため。これが無いと「?」が境界扱いされず option 1 が見出しへ飲み込まれ、タブ見出しに
  // 「質問文＋選択肢1＋選択肢2」が化けて入り、残りの番号からしかボタンが作られない
  //（=「ハブの質問が途切れる／選択肢の頭が欠ける」症状）。連番チェック③が誤分割を抑える。
  function splitGluedNumberedLine(rawLine) {
    const line = String(rawLine == null ? '' : rawLine);
    const re = /(^|[\s)）」』】。．？?！!…])(\d{1,2})\.(?:\s+|(?=\D))/g;
    const marks = [];
    let m;
    while ((m = re.exec(line)) !== null) {
      marks.push({ at: m.index + m[1].length, num: parseInt(m[2], 10) });
      if (m.index === re.lastIndex) re.lastIndex++; // ゼロ幅マッチの無限ループ防止
    }
    if (marks.length < 2) {
      // 見出し（質問文）が option 1 と同一行へ連結されたケース（「質問? 1. A」）。
      // option 2 以降は別行に残るため連番ペアが作れず、上の <2 早期 return だと
      // この行は optionRe/headingRe いずれにもマッチせず丸ごと捨てられ、option 1 が欠落する
      //（=「選択肢の 1 が無い」症状）。唯一の番号が 1 で、かつ前に見出しテキストがあるときだけ
      // 見出しと option 1 へ分割して救済する（先頭が既に「1.」の正規行は head が空なので不変）。
      // head がカーソル/箇条書き記号だけ（「❯ 1. …」等）の場合は分割しない。
      // それは見出し連結ではなく正規のカーソル付き選択肢行で、分割するとカーソルが
      // 別行へ切り離されて isCurrent 検出が壊れる（claude /model メニュー等）。
      if (marks.length === 1 && marks[0].num === 1) {
        const head = line.slice(0, marks[0].at).trim();
        if (head && !/^[>❯›❱*\-•・]+$/.test(head)) {
          const seg = line.slice(marks[0].at).trim();
          return seg ? [head, seg] : [head];
        }
      }
      return [rawLine];
    }
    for (let i = 1; i < marks.length; i++) {
      if (marks[i].num !== marks[i - 1].num + 1) return [rawLine];
    }
    const out = [];
    const head = line.slice(0, marks[0].at).trim(); // 先頭の見出し/本文（あれば）
    if (head) out.push(head);
    for (let i = 0; i < marks.length; i++) {
      const end = i + 1 < marks.length ? marks[i + 1].at : line.length;
      const seg = line.slice(marks[i].at, end).trim();
      if (seg) out.push(seg);
    }
    return out;
  }

  // 連結された承認行を元の行構造へ復元する共通処理。
  // marker 経路（parseHubBlock）/ フォールバック経路（extractApprovalOptions）双方で使う。
  // 先に「N. User specifies」アンカーで切り、その後で行内連番を分割する
  //（N. を先に切らないと最後の選択肢ラベルへ「N. User specifies」が混入するため）。
  function ungluedApprovalLines(lines) {
    return splitUserSpecifiesAnchor(lines).flatMap(splitGluedNumberedLine);
  }

  // 複数選択ディレクティブ「#multi 質問文?」。これがブロック内にあると、後続の
  // 番号付き選択肢を「任意個 ON/OFF できる複数選択」として扱う（単一選択ではない）。
  const multiSelectDirectiveRe = /^#multi\b[ \t]*(.*)$/i;
  // 各質問末尾の区切りアンカー。続き行結合の対象から除外する（ラベルへ混入させない）。
  const userSpecifiesLineRe = /^N\.\s*(?:user specifies|その他指定)/i;

  // 一括質問タブUI 用: 選択肢ラベル先頭の短ラベル表記 `[短ラベル] 本文` を分離する。
  // 角括弧内が 1〜12 文字で、かつ続く本文が空でないときのみ短ラベルとして扱う
  //（`[保留] ...` のような既存ラベル先頭表記の取り違えを避けるため本文必須）。
  // shortLabel はタブ/選択肢ボタンの圧縮表示に、label（本文）は詳細パネルに使う。
  function splitShortLabel(label) {
    const m = String(label || '').match(/^\[([^\][]{1,12})\]\s*(\S.*)$/);
    if (!m) return { shortLabel: undefined, label: String(label || '').trim() };
    return { shortLabel: m[1].trim(), label: m[2].trim() };
  }

  function parseHubBlock(rawLines) {
    const lines = ungluedApprovalLines(rawLines);
    const text = lines.join('\n');
    if (hasYesNoApprovalMarker(text)) {
      if (isPlaceholderYesNoQuestion(text)) return null;
      return yesNoApprovalOptions(yesNoCtxFromText(text));
    }
    // 複数選択（#multi）: ディレクティブ行があれば番号付き選択肢を multiSelect として返す。
    // 単一選択・バッチのセクション解析より前に確定させる（#multi 行自体は heading/option に
    // マッチしないので通常ループには載らないが、明示的に専用経路で処理する）。
    {
      const optionRe = /^(\d+)\.\s*(.+?)\s*$/;
      let question = '';
      let isMulti = false;
      const opts = [];
      let lastMultiOpt = null;
      for (const raw of lines) {
        const line = String(raw || '').trim();
        if (!line) continue;
        const dm = line.match(multiSelectDirectiveRe);
        if (dm) { isMulti = true; question = dm[1].trim(); lastMultiOpt = null; continue; }
        const om = line.match(optionRe);
        if (om) {
          lastMultiOpt = { num: parseInt(om[1], 10), label: om[2].trim() };
          opts.push(lastMultiOpt);
          continue;
        }
        if (userSpecifiesLineRe.test(line)) { lastMultiOpt = null; continue; }
        // xterm がラベル途中で折り返した続き行（数字始まりでない）は直前の選択肢へ結合する。
        if (lastMultiOpt) lastMultiOpt.label = (lastMultiOpt.label + line).replace(/\s+/g, ' ').trim();
      }
      if (isMulti && opts.length > 0) {
        return opts.map(o => ({
          num: o.num,
          label: o.label,
          isCurrent: false,
          _multiSelect: true,
          _question: question,
        }));
      }
    }
    const sections = [];
    const looseOpts = [];
    let cur = null;
    let lastOpt = null; // 続き行結合の対象（直近の選択肢）
    let looseFreeInput = false; // 単一選択（looseOpts）に「N. User specifies」があったか
    const optionRe = /^(\d+)\.\s*(.+?)\s*$/;
    // 見出しは `Q1 質問文?`（Q + 連番）を正とする。選択肢 `1.`（数字+ピリオド）と区別するため。
    // 後方互換: 旧 `1 質問文?`（プレフィックスなし数字+スペース）も引き続き受理する。
    // 区切りゆれ吸収: 数字直後の `:` `：` `.` は任意（`Q1: 質問` / `Q1. 質問` も可）。
    // optionRe を先に評価するので `1. 選択肢` は見出しに誤マッチしない（順序維持が前提）。
    const headingRe = /^(?:[QＱ][ \t]*)?(\d+)[ \t]*[.:：]?[ \t]+(.+?)\s*$/i;
    for (const raw of lines) {
      const line = String(raw || '').trim();
      if (!line) continue;
      const om = line.match(optionRe);
      if (om) {
        const opt = { num: parseInt(om[1], 10), label: om[2].trim(), isCurrent: false };
        // バッチ・単一いずれも短ラベル表記 `[短ラベル] 本文` を分離する
        //（単一質問もタブUIに統合され、短ラベル＝ボタン圧縮表示／本文＝詳細パネルに使うため）。
        const sl = splitShortLabel(opt.label);
        opt.label = sl.label;
        if (sl.shortLabel) (opt as any).shortLabel = sl.shortLabel;
        if (cur) cur.options.push(opt);
        else looseOpts.push(opt);
        lastOpt = opt;
        continue;
      }
      const hm = line.match(headingRe);
      if (hm) {
        cur = { num: parseInt(hm[1], 10), title: hm[2].trim(), options: [] };
        sections.push(cur);
        lastOpt = null;
        continue;
      }
      // 質問末尾の「N. User specifies」は区切り。続き行として混入させない。
      // バッチの質問内に出現したらその質問を、単一選択の文脈なら looseOpts を自由入力可にする
      //（タブUIで N 肢を出す）。
      if (userSpecifiesLineRe.test(line)) {
        if (cur) (cur as any)._freeInput = true;
        else looseFreeInput = true;
        lastOpt = null;
        continue;
      }
      // xterm がラベル/見出し途中で折り返した続き行（数字始まりでない）を元の要素へ結合する。
      // 直前に選択肢があればそのラベルへ、無ければ現在の見出しタイトルへ繋ぐ。
      if (lastOpt) lastOpt.label = (lastOpt.label + line).replace(/\s+/g, ' ').trim();
      else if (cur) cur.title = (cur.title + line).replace(/\s+/g, ' ').trim();
    }
    const filledSections = sections.filter(s => s.options.length > 0);
    if (filledSections.length >= 2) return filledSections;
    if (filledSections.length === 1) return filledSections[0].options;
    if (looseOpts.length > 0) {
      // 自由入力フラグは配列プロパティで持たせる（option 構造は変えず isBatchOptions=false を維持）。
      if (looseFreeInput) (looseOpts as any)._freeInput = true;
      return looseOpts;
    }
    return null;
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
    const opt: any = { num: parseInt(numText, 10), label, isCurrent: !!isCurrent };
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

  function extractApprovalOptions(rawTail) {
    // Ink 連結で1行へ潰れた選択肢を元の行構造へ復元してから走査する。
    // cluster の index は展開後の tail を基準にするため、呼び出し側へ展開後の lines も返す
    //（呼び出し側が approvalContextLines で同じ配列を使えるようにし、index ずれを防ぐ）。
    const tail = ungluedApprovalLines(rawTail);
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
      // 空行を挟まずに選択肢の直上(画面では直下)へ続く非選択肢行は、xterm の
      // ハードラップで分割された折り返し継続行とみなし、直近(上方)の選択肢ラベルへ結合する。
      // 旧実装はインデント付き行(/^\s+\S/)のみ継続扱いにしていたが、xterm の折り返しは
      // 行頭に空白を入れないため、最長になりがちな option 1(Recommended 本文)が折り返されると
      // ここで break して option 1 ごと欠落していた(「確認メッセージの 1 が途切れる」頻発症状)。
      // 空行(blankGap>0)で区切られた履歴は従来どおり break するため scrollback の誤検出は増えない。
      if (blankGap === 0) {
        pendingContinuation.push(line.trim());
        continue;
      }
      break;
    }
    if (options.length === 0) return { options: [], cluster: null, lines: tail };
    const nums = options.map(opt => opt.num);
    const numMin = Math.min(...nums);
    const numMax = Math.max(...nums);
    if (numMax > 20 || numMax - numMin > 15 || options.length > 12) {
      return { options: [], cluster: null, lines: tail };
    }
    const seen = new Set();
    const uniqueOptions = options.filter(opt => {
      const key = `${opt.num}:${opt.label}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
    if (uniqueOptions.length < 2) return { options: [], cluster: null, lines: tail };
    // Claude のネイティブ AskUserQuestion ピッカー（末尾に "Type something" /
    // "Chat about this" の自由入力肢を持つ arrow 駆動 UI）は Web ボタン化しない。
    // 再描画される VT をスクレイプすると選択肢番号が Web ボタンとズレて誤選択を招くため。
    // AI には approval-rules.md(version 10) で [MANY-AI-CLI] マーカーへ誘導済み。
    // 万一 AI が出しても Web バーは出さず、端末で直接 ↑↓/Enter 操作する。
    if (uniqueOptions.some(o => /^\s*(type something|chat about)/i.test(o.label || ''))) {
      return { options: [], cluster: null, lines: tail };
    }
    return {
      options: uniqueOptions,
      cluster: { start: clusterStart, end: clusterEnd },
      lines: tail,
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
    isMultiSelectOptions,
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

})(typeof window !== 'undefined' ? window : globalThis);

// --- ESM re-exports from the IIFE-published approval parser API (generated) ---
const __esmRoot = (typeof window !== 'undefined') ? window : globalThis;
export const approvalParser = __esmRoot.approvalParser;
export const {
  lineHasHint, linesHaveHint, approvalLineHasHint, approvalLinesHaveHint, extractHubMarkerApproval, extractPlainYesNoApproval, extractSequentialChoicePrompts, extractApprovalOptions, approvalContextLines, isBatchOptions, isMultiSelectOptions, isMultiQuestionPrompt, isHubChoicePrompt, markHubChoiceDefault, matchNativeApprovalTrigger, hasApprovalLikeLabel, userSpecifiesRe,
} = __esmRoot.approvalParser;
