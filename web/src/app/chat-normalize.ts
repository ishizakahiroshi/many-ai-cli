// チャットに入れる本文の軽い正規化（chat-history.ts から分けた。DOM に触らないので単体で試せる）。
//
// **この file の処理は、本文の長さに比例する時間で終わるように保つ。** 本文は AI の出力や
// 表示した file の中身で、長さも中身も選べない。受信のたびに（チャットを開いていなくても）走るので、
// 長さの二乗かかる処理が 1 つあると画面が止まる（監査 N34: 行末の空白を /\s+$/ で落としていて、
// 「文字 + 空白 20 万個 + 文字」の 1 行に 11 秒かかった）。先頭に固定していない正規表現で
// 「空白の並び + 行末」のような形を探さない。端を落とすのは trimEnd と下の走査で行う。

// Claude Code / Codex の「思考中」スピナー行を判定する。
// PTY を ANSI 除去しただけだと、思考中ステータス行
// （"✳ Imploring… (12s · ↑3.2k tokens · esc to interrupt)" 等）の再描画フレームが
// 大量に連結して残る。これらは会話本文ではないので 1 行まるごと落とす。
// 誤検出を避けるため、星形 dingbat（✢✳✶✷ U+2722–U+273F）と braille に絞り、
// ✓✗ など普通に使う記号は対象にしない。Go 側 sessionlog.IsThinkingNoiseLine と同義。
const SPINNER_ANIM_RE = /[✢-✿⠀-⣿]/;
const SPINNER_ANIM_RE_G = /[✢-✿⠀-⣿]/g;

export function isThinkingNoiseLine(line: unknown): boolean {
  const t = String(line == null ? '' : line).trim();
  if (!t) return false;
  const lower = t.toLowerCase();
  // 1) ステータスフッター（Claude/Codex とも "esc to interrupt" を必ず表示する）
  if (lower.includes('esc to interrupt')) return true;
  // 2) モード切替ヒント "auto mode on (shift+tab to cycle)"
  if (/shift\s*\+\s*tab to cycle/i.test(t)) return true;
  // 3) トークンバー "↑111.0k ↓764"
  if (t.includes('↑') && t.includes('↓')) return true;
  // 4) "thinking" + スピナーグリフ（思考アニメの再描画フレーム）
  if (lower.includes('thinking') && SPINNER_ANIM_RE.test(t)) return true;
  // 5) スピナーグリフが複数現れる断片（"Imp·rmpovri✶osviisng✶..." 等）
  const m = t.match(SPINNER_ANIM_RE_G);
  if (m && m.length >= 2) return true;
  return false;
}

// 先頭と末尾の改行を落とす（/^\n+|\n+$/g と同じ結果。末尾からの走査なので長さに比例する）。
function trimNewlines(s: string): string {
  let start = 0;
  let end = s.length;
  while (start < end && s.charCodeAt(start) === 10) start++;
  while (end > start && s.charCodeAt(end - 1) === 10) end--;
  return s.slice(start, end);
}

// D15: 軽い正規化（行頭末 trim / prompt prefix 除去 / 連続空行圧縮）。
// コードブロック内（```...```）の trim はあえてしない。
export function normalizeChatText(raw: string | null | undefined): string {
  if (!raw) return '';
  // [MANY-AI-CLI]...[/MANY-AI-CLI] / [MANY-AI-CLI-DONE]...[/MANY-AI-CLI-DONE] ブロックを除去
  // （承認マーカー・完了サマリーマーカーはチャットに表示しない。DONE は -DONE 付きで
  //  別リテラルのため、汎用の [MANY-AI-CLI] パターンに一致せず先に専用で除去する）
  const s = raw
    .replace(/\[MANY-AI-CLI-DONE\][\s\S]*?\[\/MANY-AI-CLI-DONE\]/g, '')
    .replace(/\[MANY-AI-CLI\][\s\S]*?\[\/MANY-AI-CLI\]/g, '');
  const lines = s.split(/\r?\n/);
  const out: string[] = [];
  let inFence = false;
  let blankRun = 0;
  for (let line of lines) {
    const isFenceMarker = /^\s*```/.test(line);
    if (isFenceMarker) inFence = !inFence;
    // 思考中スピナーの再描画フレーム行を丸ごと落とす（コードフェンス内・フェンス行は対象外）
    if (!inFence && !isFenceMarker && isThinkingNoiseLine(line)) continue;
    if (!inFence) {
      // prompt prefix 除去（行頭のみ。行頭に固定した照合なので長さに比例する）
      line = line.replace(/^[\s]*[▌>$#]\s?/, '');
      // 行末の空白。trimEnd が落とす文字は正規表現の \s と同じ。
      line = line.trimEnd();
    }
    if (!inFence && line.trim() === '') {
      blankRun++;
      if (blankRun <= 1) out.push('');
    } else {
      blankRun = 0;
      out.push(line);
    }
  }
  return trimNewlines(out.join('\n'));
}
