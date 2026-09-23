// Hub の保留中の記録のブロックを選択肢にする純粋なパーサ。Keep classic-script compatibility; no module wrapper.
//
// 入力は Hub が送ってくる記録のブロック（承認マーカーの原文・順次質問の Block）と、承認タブの
// 履歴の行だけ。端末の文字から承認を作る処理（推測での拾い上げ・マーカー無しの Yes/No・
// 旧形式の選択・複数質問の判定）は Hub（internal/hub/approval_text_question.go /
// approval_detector.go）にだけあり、ここには置かない（scripts/check-approval-display-source.mjs）。
(function (root) {
  'use strict';

  const sequentialQuestionHeaderRe = /^\s*([A-Z]{1,3}\d{1,3}|Q\d{1,3}|問\d{1,3})\s*[:：]\s*(.+?)\s*$/i;
  const yesNoApprovalMarkerRe = /[（(]\s*[YＹ]\s*[:：]\s*1\s*[\/／]\s*[NＮ]\s*[:：]\s*0\s*[）)]/ig;

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

  function approvalCtxHash(s) {
    const text = String(s || '').replace(/\s+/g, ' ').trim();
    let h = 5381;
    for (let i = 0; i < text.length; i++) {
      h = (((h << 5) + h) + text.charCodeAt(i)) | 0;
    }
    return (h >>> 0).toString(36);
  }

  // 差分再描画の残骸でマーカー文字列がラベルに混入したパース結果は出さない。
  function optionNumSeq(opts) {
    return (opts || [])
      .map(o => (o && typeof o.num === 'number') ? o.num : NaN)
      .filter(n => Number.isFinite(n));
  }

  // 最初の 1 以降だけを見て重複を判定する。前置き（経緯）の地の文に行頭数字が混じると
  // parseHubBlock がそれを擬似 option として拾うため（実例: 前置きの IP アドレス
  // 192.168.68.100 が num=192 になる）、先頭から見ると正常なブロックを弾いてしまう。
  function hasDuplicateNumFromFirstOne(nums) {
    const i1 = nums.indexOf(1);
    if (i1 < 0) return false;
    const seen = new Set();
    for (const n of nums.slice(i1)) {
      if (seen.has(n)) return true;
      seen.add(n);
    }
    return false;
  }

  // 罫線（box drawing）の連続。approval-rules.md はマーカーブロック内の罫線・表組みを
  // 禁じているため、パース結果に罫線が現れたら AI の出力ではなく、TUI コンポーザの枠線が
  // 再描画で本文へ重なった証拠になる（Hub 側 classifyApprovalMarkerBlock と同じ判定を
  // クライアントにも置く。旧 Hub と新 UI の組み合わせでも症状を出さないため）。
  // しきい値 3 は「10─20」のような範囲表記を誤爆させないための余裕。
  const boxRule = /[\u2500-\u257F]{3,}/;

  function hasBoxRuleText(options) {
    const arr = options as any;
    if (boxRule.test(String(arr._question || ''))) return true;
    for (const el of arr) {
      if (!el || typeof el !== 'object') continue;
      if (boxRule.test(String(el.title || ''))) return true;
      if (boxRule.test(String(el.label || ''))) return true;
      if (boxRule.test(String(el._question || ''))) return true;
      for (const o of (el.options || [])) {
        if (o && boxRule.test(String(o.label || ''))) return true;
      }
    }
    return false;
  }

  function isCorruptHubMarkerOptions(options) {
    if (!options || !Array.isArray(options) || options.length === 0) return true;
    if (hasBoxRuleText(options)) return true;
    const arr = options as any;
    const markerLeak = /\[MANY-AI-CLI\]|\[\/MANY-AI-CLI\]/i;
    const q = arr._question != null ? String(arr._question) : '';
    if (markerLeak.test(q)) return true;
    if (isBatchOptions(options)) {
      if (options.some(sec =>
        markerLeak.test(String((sec && sec.title) || '')) ||
        (sec.options || []).some(o => markerLeak.test(String((o && o.label) || ''))))) return true;
      // バッチは「ブロック通し番号」「質問ごとに 1 から振り直し」の両方が許容されるため
      // （approval-rules.md）、先頭セクションが 1 を含むかだけを検査する。
      const firstSec = optionNumSeq(options[0] && options[0].options);
      if (firstSec.length > 0 && firstSec.indexOf(1) < 0) return true;
      // 質問をまたいだ番号の再利用（Q1 が 1,2,3 / Q2 が 3,4,5）は approval-rules.md が
      // 許容しているため、重複検査はセクション内に閉じる。
      return options.some(sec => hasDuplicateNumFromFirstOne(optionNumSeq(sec && sec.options)));
    }
    if (options.some(o => markerLeak.test(String((o && o.label) || '')) ||
      markerLeak.test(String((o && o._question) || '')))) return true;
    // 選択肢番号の構造検査。
    // 背景: docs/local/archive/v0.5.x/bugfix_codex-approval-marker-vt-wrap-corruption_2026-07-31.md
    // Hub の VT ミラーが実端末と乖離すると、開始/終了マーカーは揃ったまま選択肢行だけが
    // 失われ、「選択肢が 3 から始まるパネル」「ボタン 1 個だけのパネル」が描画される。
    // approval-rules.md はブロック通し番号も質問ごとの振り直しも認めるが、どちらの書式でも
    // 選択肢番号 1 は必ずどこかに現れる。1 が 1 つも無いのは欠落の証拠。
    // 「先頭が 1」で判定しないのが要点 — 前置きの地の文に行頭数字（IP アドレス・版数など）が
    // あると parseHubBlock がそれを擬似 option として拾い、正常なブロックを弾いてしまう。
    // Y/N 形式は yesNoApprovalOptions が [1, 0] を合成するので誤爆しない。
    const nums = optionNumSeq(options);
    if (nums.length > 0 && nums.indexOf(1) < 0) return true;
    return hasDuplicateNumFromFirstOne(nums);
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

  function extractHubMarkerApproval(lines) {
    const source = Array.isArray(lines) ? lines : [];
    // [MANY-AI-CLI]…[/MANY-AI-CLI] を「末尾優先の完全ブロック」として取り出す。
    // ブロックの行数に依存させない（固定窓で切らない）のが要点 — 複数質問一括や #multi で
    // 選択肢が増え、長い日本語ラベルが端末幅で折り返されてブロックが何十行になっても、
    // 開きマーカーが窓から外れて末尾の質問だけ拾う事故を構造的に防ぐ。マーカーは明示
    // デリミタ済みで scrollback 誤検出の懸念がないため source 全体を走査してよい。
    // 入力は Hub の記録のブロック（または台帳の行）で、ブロック全体が渡ってくる。
    const recentText = source.join('\n');
    const blockRe = /\[MANY-AI-CLI\]([\s\S]*?)\[\/MANY-AI-CLI\]/g;
    let match;
    let lastBlock = null;
    while ((match = blockRe.exec(recentText)) !== null) {
      lastBlock = match[1];
    }
    if (lastBlock !== null) {
      const inner = lastBlock.split('\n').map(l => l.trim()).filter(Boolean);
      const parsed = parseHubBlock(inner);
      return isCorruptHubMarkerOptions(parsed) ? null : parsed;
    }

    // 開き/閉じが別チャンクに割れて全文一致しなかった場合の末尾アンカー・フォールバック。
    // 同じく固定窓は使わず source 全体を末尾から遡って対の開きマーカーを探す。
    let closeIdx = -1;
    let openIdx = -1;
    for (let i = source.length - 1; i >= 0; i--) {
      const line = source[i];
      if (/\[MANY-AI-CLI\]/.test(line) && /\[\/MANY-AI-CLI\]/.test(line)) {
        const inner = line.replace(/^[\s\S]*?\[MANY-AI-CLI\]/, '').replace(/\[\/MANY-AI-CLI\][\s\S]*$/, '').trim();
        const parsed = parseHubBlock([inner]);
        return isCorruptHubMarkerOptions(parsed) ? null : parsed;
      }
      if (/\[\/MANY-AI-CLI\]/.test(line) && closeIdx === -1) { closeIdx = i; continue; }
      if (/\[MANY-AI-CLI\]/.test(line) && closeIdx !== -1) { openIdx = i; break; }
    }

    if (openIdx === -1 || closeIdx === -1) return null;
    const inner = source.slice(openIdx + 1, closeIdx).map(l => l.trim()).filter(Boolean);
    const parsed = parseHubBlock(inner);
    return isCorruptHubMarkerOptions(parsed) ? null : parsed;
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
    // `User` と `specifies` の間の空白は AI 出力で落ちることがある（観測: `N.Userspecifies`）。
    // そのため `User\s*specifies` で空白省略形も受理する。マッチ結果は正規化して
    // 「N. User specifies」固定文字列で出力し、後段の userSpecifiesLineRe と一致させる。
    const splitRe = /\s*\bN\.[ \t]*(User\s*specifies|その他指定)\b\s*/i;
    const out = [];
    const expand = (line) => {
      const m = line.match(splitRe);
      if (!m) { out.push(line); return; }
      const before = line.slice(0, m.index).trim();         // 連結されていた選択肢/本文
      const after = line.slice(m.index + m[0].length).trim(); // 後続の見出し等
      if (before) out.push(before);
      // 正規化（`N.Userspecifies` → `N. User specifies` / `N.その他指定` → `N. その他指定`）。
      const kind = /その他指定/.test(m[1]) ? 'その他指定' : 'User specifies';
      out.push(`N. ${kind}`);
      if (after) expand(after);                             // 後続をさらに分割
    };
    for (const raw of (lines || [])) expand(String(raw || ''));
    return out;
  }

  // 行内に紛れた `Q\d+` 見出しを直前で改行する。AI が前置き文末尾と Q1 を改行なしで
  // 続けて出力すると（観測例: `...確認したい情報）:Q1「セッションが切れる」とは...`）、
  // 後段の splitGluedNumberedLine が Q1 配下の選択肢を見つけられても、見出しが見つからず
  // 「単一質問・複数選択肢」として誤解釈され、Q1 と Q2 の選択肢が 1 つの質問に混じる。
  // 行内の `Q\d+` を見つけたら、その直前で行を切って独立した見出し行として後段へ渡す。
  // 注意: 行頭が既に `Q\d+` の正規見出しは触らない（slice(1) で先頭を除外して検索する）。
  // 全角 `Ｑ` も拾うが、直後が英数字（`Q1A` 等の識別子）の場合は見出し扱いしない。
  function splitInlineQuestionHeading(lines) {
    const out = [];
    // 番号付き選択肢行（`1. ラベル本文`）はラベル本文が自然文であり、
    // AI が「Q1 のフォームを外す」の意で `Q1フォームを外し…` と書いた `Q1` を
    // 見出しとして分離すると、選択肢が疑似 Q セクション化されて質問ポップアップが
    // 重複する（bugfix_hub-approval-q-split-inside-option_2026-07-24.md）。
    // 選択肢行内の `Q\d+` はすべて本文扱いとし、この行では一切分割しない。
    // マーカー規約は全 AI エージェント共通なので、この防御は provider を問わない。
    const numberedOptionRe = /^\s*\d{1,2}\.\s+\S/;
    for (const raw of (lines || [])) {
      let rest = String(raw || '');
      if (numberedOptionRe.test(rest)) { if (rest.trim()) out.push(rest); continue; }
      while (true) {
        if (rest.length <= 1) break;
        const m = /[QＱ]\d{1,2}(?![A-Za-z\d])/.exec(rest.slice(1));
        if (!m) break;
        const at = m.index + 1;
        // 直前文字が開き括弧（`[「『【（(`）のときはラベル/装飾内の `Q1` 言及とみなして
        // 見出し分割を諦め、ループを抜ける（残りの行に他の候補があってもこの行では切らない）。
        const prev = rest.charAt(at - 1);
        if (/[\[「『【（(]/.test(prev)) break;
        const head = rest.slice(0, at).trim();
        if (head) out.push(head);
        rest = rest.slice(at);
      }
      if (rest.trim()) out.push(rest);
    }
    return out;
  }

  // Ink のカーソル位置制御描画では選択肢間の改行も失われ、
  // 「1. … 2. … 3. …」が1行へ連結されることがある（行の split で1要素化）。
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
    // 強パターン: `N.[短ラベル]` / `N. [短ラベル]` は marker block の選択肢規約そのもの。
    // 直前文字が漢字/かな（境界文字集合に入っていない）でも、`.[…]` が続けばほぼ確実に選択肢開始。
    // 観測例: `...失敗する3.[両方混在]...両方が混在4.[その他]...` — 弱パターン側は境界不足で
    // 3, 4 を拾えず marks=[1,2,5] となり連番チェックで原文返却していた。
    // 強パターン側は前文字を問わないので 1,2,3,4,5 を全て検出できる。
    // approval-rules.md の規約は `1. [短ラベル] 本文` と `.` と `[` の間に空白を挟む書式なので、
    // `\s*` で空白 0〜N 個を許容する（`1.[X]` glued も従来どおり検出できる）。
    // 監督条件: 単調非減少（重複は許容しない・降順は除外）。連番強制までは課さない
    //（弱パターンは課すが、強パターンは bracket suffix 自体が誤検出の盾になるため緩める）。
    const strongRe = /(\d{1,2})\.\s*\[([^\][\n]{1,16})\]/g;
    const strongMarks = [];
    let smOk = true;
    let sm;
    while ((sm = strongRe.exec(line)) !== null) {
      const num = parseInt(sm[1], 10);
      if (strongMarks.length > 0 && num <= strongMarks[strongMarks.length - 1].num) { smOk = false; break; }
      strongMarks.push({ at: sm.index, num });
      if (sm.index === strongRe.lastIndex) strongRe.lastIndex++;
    }
    if (smOk && strongMarks.length >= 2) {
      const out = [];
      const head = line.slice(0, strongMarks[0].at).trim();
      if (head) out.push(head);
      for (let i = 0; i < strongMarks.length; i++) {
        const end = i + 1 < strongMarks.length ? strongMarks[i + 1].at : line.length;
        const seg = line.slice(strongMarks[i].at, end).trim();
        if (seg) out.push(seg);
      }
      return out;
    }
    // 強パターン 1 つだけ＋見出し連結のケース（`Q2 ...? 5.[短時間ランダム]...` を想定）。
    // 見出し（headingRe）と option（optionRe）の境界で切り、parseHubBlock 側が独立した
    // 要素として認識できるようにする。head が空 or 数字始まりだけの場合は触らない
    //（=「先頭が `5.[…]` の正規行」では head 空のため不変）。
    if (strongMarks.length === 1) {
      const head = line.slice(0, strongMarks[0].at).trim();
      const seg = line.slice(strongMarks[0].at).trim();
      if (head && seg && !/^\d/.test(head)) {
        return [head, seg];
      }
    }
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

  // 差分再描画型 TUI（観測: Grok CLI / mer セッション 2026-07-12 実ログ）は、行を改行ではなく
  // 絶対カーソル移動（CUP `ESC[行;列H`）で描画し、さらに前フレームから変わらないセル
  //（選択肢番号直後の「. 」等）を書き直さずカーソル前進（CUF `ESC[nC`）でスキップする。
  // これを stripAnsi で単純削除すると行境界と桁の隙間が同時に消え、マーカーブロック全体が
  // 「…(Recommended)2原因…3Excel…」の 1 行に連結されて番号のピリオドまで失われる
  //（弱パターンの連番分割が効かず、選択肢 1 のボタンへ全部飲み込まれる症状）。
  // 削除ではなくテキスト構造へ変換してから行分割する:
  // ・行移動（CUP H / CNL E / CPL F / VPA d）→ 改行（別の行へ描く = 行境界）
  // ・カーソル前進（CUF C）→ スキップ桁数ぶんの空白（画面上は前フレームの文字が残る隙間）
  // ・文字消去（ECH X）→ 空白 1 個（消去領域は空白 = 区切りとしてのみ意味を持つ）
  // SGR・カーソル表示切替など残りのシーケンスは従来どおり呼び出し側の stripAnsi が落とす。
  function normalizeVtCursorOps(text) {
    return String(text == null ? '' : text)
      .replace(/\x1b\[[0-9;]*[HEFd]/g, '\n')
      .replace(/\x1b\[([0-9]*)C/g, (_, n) => ' '.repeat(Math.min(parseInt(n || '1', 10) || 1, 200)))
      .replace(/\x1b\[[0-9]*X/g, ' ');
  }

  // 連結された承認行を元の行構造へ復元する（parseHubBlock の前処理）。
  // 先に「N. User specifies」アンカーで切り、その後で行内連番を分割する
  //（N. を先に切らないと最後の選択肢ラベルへ「N. User specifies」が混入するため）。
  function ungluedApprovalLines(lines) {
    // 行内に紛れた `Q\d+` 見出しを先に切り出し、各セグメント内で
    // 「N. User specifies」アンカー → 連結された連番選択肢、の順に分解する。
    // Q-split を最先にするのは、Q1〜Q4 の選択肢番号が広域で（1〜16 等）連続するため
    // 弱パターン側の連番チェックが大ジャンプで失敗するのを防ぐため。
    const afterQ = splitInlineQuestionHeading(lines);
    const afterN = splitUserSpecifiesAnchor(afterQ);
    return afterN.flatMap(splitGluedNumberedLine);
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
    const optionRe = /^(\d+)\.\s*(.+?)\s*$/;
    // 差分再描画のセルスキップで番号直後の「. 」が届かなかった選択肢行の救済形。
    // normalizeVtCursorOps がスキップ桁を空白へ変換するため「2  原因…」の形で行に残る。
    // 空白 2 個以上（=「. 」の 2 桁ぶん）を必須にして、後方互換の No-Q 見出し
    // 「1 質問文?」（空白 1 個）とは衝突させない。グループ構成は optionRe と同じ
    //（1=番号 / 2=ラベル）で、マッチ結果を差し替え可能にしている。
    const dotlessOptionRe = /^(\d{1,2})[ \t]{2,}(\S.*?)\s*$/;
    // 見出しは `Q1 質問文?`（Q + 連番）を正とする。選択肢 `1.`（数字+ピリオド）と区別するため。
    // 後方互換: 旧 `1 質問文?`（プレフィックスなし数字+スペース）も引き続き受理する。
    // 区切りゆれ吸収: 数字直後の `:` `：` `.` は任意（`Q1: 質問` / `Q1. 質問` も可）。
    // Q プレフィックス有りの場合は空白省略形も受理する（`Q2切れるまでの…?` のように
    // AI が空白なしで質問本文を続け書きするケースの救済。No-Q 側で同じ緩和をすると
    // `1abc` のような誤マッチを生むため、Q 有り側だけ空白省略を許す）。
    // optionRe を先に評価するので `1. 選択肢` は見出しに誤マッチしない（順序維持が前提）。
    const headingRe = /^(?:[QＱ][ \t]*(\d+)[ \t]*[.:：]?[ \t]*(\S.*?)|(\d+)[ \t]*[.:：]?[ \t]+(.+?))\s*$/i;
    const preambleLines = [];
    let preambleEnd = 0;
    for (let i = 0; i < lines.length; i++) {
      const raw = lines[i];
      const line = String(raw || '').trim();
      if (!line) { preambleEnd = i + 1; continue; }
      if (optionRe.test(line) || dotlessOptionRe.test(line) || headingRe.test(line) || multiSelectDirectiveRe.test(line) || hasYesNoApprovalMarker(line)) {
        preambleEnd = i;
        break;
      }
      preambleLines.push(line);
      preambleEnd = i + 1;
    }
    const attachPreamble = (parsed) => {
      const preamble = preambleLines.join('\n').trim();
      if (preamble && parsed && Array.isArray(parsed)) (parsed as any)._preamble = preamble;
      return parsed;
    };
    const questionLines = preambleLines.length ? lines.slice(preambleEnd) : lines;
    const text = questionLines.join('\n');
    if (hasYesNoApprovalMarker(text)) {
      if (isPlaceholderYesNoQuestion(text)) return null;
      const yn = yesNoApprovalOptions(yesNoCtxFromText(text));
      const q = yesNoQuestionText(text);
      if (q) (yn as any)._question = q; // 質問本文を承認ポップアップへ表示するため付与
      return attachPreamble(yn);
    }
    // 複数選択（#multi）: ディレクティブ行があれば番号付き選択肢を multiSelect として返す。
    // 単一選択・バッチのセクション解析より前に確定させる（#multi 行自体は heading/option に
    // マッチしないので通常ループには載らないが、明示的に専用経路で処理する）。
    {
      let question = '';
      let isMulti = false;
      const opts = [];
      let lastMultiOpt = null;
      for (const raw of lines) {
        const line = String(raw || '').trim();
        if (!line) continue;
        const dm = line.match(multiSelectDirectiveRe);
        if (dm) { isMulti = true; question = dm[1].trim(); lastMultiOpt = null; continue; }
        const om = line.match(optionRe) || line.match(dotlessOptionRe);
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
        return attachPreamble(opts.map(o => ({
          num: o.num,
          label: o.label,
          isCurrent: false,
          _multiSelect: true,
          _question: question,
        })));
      }
    }
    const sections = [];
    const looseOpts = [];
    let cur = null;
    let lastOpt = null; // 続き行結合の対象（直近の選択肢）
    let looseFreeInput = false; // 単一選択（looseOpts）に「N. User specifies」があったか
    const looseQuestionLines = []; // 単一ブロックで最初の選択肢より前に置かれた質問文（見出し`Q1`形式でない地の文）
    for (const raw of lines) {
      const line = String(raw || '').trim();
      if (!line) continue;
      const om = line.match(optionRe) || line.match(dotlessOptionRe);
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
        // headingRe は Q 有り（group 1,2）と Q 無し（group 3,4）の二択 alternation。
        const num = parseInt(hm[1] != null ? hm[1] : hm[3], 10);
        const title = String(hm[2] != null ? hm[2] : hm[4]).trim();
        cur = { num, title, options: [] };
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
      // 単一ブロックの先頭（選択肢も見出しも未出現）に置かれた地の文は質問文として捕捉する。
      // 従来はここで捨てられ、単一質問の承認 UI に質問が出ず CLI 画面を見ないと内容が分からなかった。
      // バッチ（`Q1 質問?` 見出し）と対称に、単一でも質問文をポップアップへ出すための捕捉。
      else looseQuestionLines.push(line);
    }
    const filledSections = sections.filter(s => s.options.length > 0);
    if (filledSections.length >= 2) return attachPreamble(filledSections);
    if (filledSections.length === 1) {
      // 単一セクション（`Q1 見出し` 形式）を flat options に落とすとき、
      // セクション上の _freeInput / title を配列プロパティへ伝播する。
      // これを忘れると「N. User specifies」があっても action-bar に自由入力肢が出ない
      // （pending_action-bar-invisible 副次 / 2026-07-14 観測）。
      const sec = filledSections[0];
      const singleOpts = sec.options;
      if ((sec as any)._freeInput) (singleOpts as any)._freeInput = true;
      if (sec.title) (singleOpts as any)._question = String(sec.title || '').trim();
      return attachPreamble(singleOpts);
    }
    if (looseOpts.length > 0) {
      // 自由入力フラグは配列プロパティで持たせる（option 構造は変えず isBatchOptions=false を維持）。
      if (looseFreeInput) (looseOpts as any)._freeInput = true;
      // 質問文も配列プロパティで持たせる（同上・isBatchOptions/approvalSig に影響させない）。
      if (looseQuestionLines.length) {
        (looseOpts as any)._question = looseQuestionLines.join(' ').replace(/\s+/g, ' ').trim();
      }
      return attachPreamble(looseOpts);
    }
    return null;
  }

  const api = {
    extractHubMarkerApproval,
    extractSequentialChoicePrompts,
    sequentialChoiceSig,
    isBatchOptions,
    isMultiSelectOptions,
    ungluedApprovalLines,
    normalizeVtCursorOps,
  };

  root.approvalParser = api;
  Object.assign(root, api);
  root._approvalCtxHash = approvalCtxHash;

})(typeof window !== 'undefined' ? window : globalThis);

// --- ESM re-exports from the IIFE-published approval parser API (generated) ---
const __esmRoot = (typeof window !== 'undefined') ? window : globalThis;
export const approvalParser = __esmRoot.approvalParser;
export const {
  extractHubMarkerApproval, extractSequentialChoicePrompts, sequentialChoiceSig, isBatchOptions, isMultiSelectOptions, ungluedApprovalLines, normalizeVtCursorOps,
} = __esmRoot.approvalParser;
