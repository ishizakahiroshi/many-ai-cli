// [MANY-AI-CLI]…[/MANY-AI-CLI] / [MANY-AI-CLI-DONE]…[/MANY-AI-CLI-DONE] ブロック本文を、
// 実端末に近い軽量 VT グリッドへ一旦描画してから読みやすいプレーンテキストへ変換する。
//
// 背景（2026-08-24 実測・セッション #23 のログを実際の filterHubMarkersPure に通して確認）:
// Claude Code の Ink UI はマーカーブロック本文の行区切りを改行文字ではなく、行ごとの
// 絶対カーソル位置指定（CSI H/f）だけで表現する。従来の flushBufToOut は ANSI を
// 単純に正規表現で削除するだけだったため、行区切りだったカーソル移動が何にも
// 変換されずに消え、複数行の本文が 1 行へ繋がって表示されていた（改行が無い・
// 行クリア用の空白が地の文に残る、の 2 症状として観測）。
//
// ここでは internal/hub/vt_buffer.go と同じ考え方（グリッドへ実際に描画してから
// 読み出す）を、マーカー本文という「境界が明示された短いバイト列」向けに簡略化して
// 移植する。実測では、Ink がストリーミング中に同じブロックを複数回まるごと再描画し、
// 古い世代の残骸（別の絶対行に書かれたまま上書きされない断片）がバッファに残ることも
// 確認できたため、「最後に文字を書き込んだ行」を描画範囲の上限にすることで、
// それより後ろの古い世代の残骸を自然に除外する。

const MAX_VT_COLS = 200;
// 32KB のブロック上限（MAX_MARKER_BUFFER_BYTES）に対して十分すぎる安全上限。
// 実端末の行数を模す必要はなく、単に無制限成長を避けるための保険。
const MAX_VT_ROWS = 2000;

function clampInt(v: number, lo: number, hi: number): number {
  if (hi < lo) return lo;
  if (v < lo) return lo;
  if (v > hi) return hi;
  return v;
}

// internal/hub/vt_buffer.go の runeCellWidth と同じ判定基準（xterm.js ではなく
// PTY の向こうで描画バイト列を組み立てる CLI 本体 = Node の string-width 系に合わせる）。
function runeCellWidth(cp: number): number {
  if (cp < 0x0300) return 1;
  if (
    (cp >= 0x0300 && cp <= 0x036f) ||
    (cp >= 0x1ab0 && cp <= 0x1aff) ||
    (cp >= 0x1dc0 && cp <= 0x1dff) ||
    (cp >= 0x200b && cp <= 0x200f) ||
    (cp >= 0x2060 && cp <= 0x2064) ||
    (cp >= 0x20d0 && cp <= 0x20ff) ||
    (cp >= 0xfe00 && cp <= 0xfe0f) ||
    (cp >= 0xfe20 && cp <= 0xfe2f) ||
    (cp >= 0xe0100 && cp <= 0xe01ef)
  ) return 0;
  if (
    (cp >= 0x1100 && cp <= 0x115f) ||
    cp === 0x231a || cp === 0x231b ||
    (cp >= 0x23e9 && cp <= 0x23ec) ||
    cp === 0x23f0 || cp === 0x23f3 ||
    (cp >= 0x25fd && cp <= 0x25fe) ||
    (cp >= 0x2614 && cp <= 0x2615) ||
    (cp >= 0x2648 && cp <= 0x2653) ||
    cp === 0x267f || cp === 0x2693 || cp === 0x26a1 ||
    (cp >= 0x26aa && cp <= 0x26ab) ||
    (cp >= 0x26bd && cp <= 0x26be) ||
    (cp >= 0x26c4 && cp <= 0x26c5) ||
    cp === 0x26ce || cp === 0x26d4 || cp === 0x26ea ||
    (cp >= 0x26f2 && cp <= 0x26f3) ||
    cp === 0x26f5 || cp === 0x26fa || cp === 0x26fd ||
    cp === 0x2705 ||
    (cp >= 0x270a && cp <= 0x270b) ||
    cp === 0x2728 || cp === 0x274c || cp === 0x274e ||
    (cp >= 0x2753 && cp <= 0x2755) ||
    cp === 0x2757 ||
    (cp >= 0x2795 && cp <= 0x2797) ||
    cp === 0x27b0 || cp === 0x27bf ||
    (cp >= 0x2b1b && cp <= 0x2b1c) ||
    cp === 0x2b50 || cp === 0x2b55 ||
    (cp >= 0x2e80 && cp <= 0x303e) ||
    (cp >= 0x3041 && cp <= 0x33ff) ||
    (cp >= 0x3400 && cp <= 0x4dbf) ||
    (cp >= 0x4e00 && cp <= 0x9fff) ||
    (cp >= 0xa000 && cp <= 0xa4cf) ||
    (cp >= 0xac00 && cp <= 0xd7a3) ||
    (cp >= 0xf900 && cp <= 0xfaff) ||
    (cp >= 0xfe10 && cp <= 0xfe19) ||
    (cp >= 0xfe30 && cp <= 0xfe6f) ||
    (cp >= 0xff00 && cp <= 0xff60) ||
    (cp >= 0xffe0 && cp <= 0xffe6) ||
    (cp >= 0x1f004 && cp <= 0x1f0cf) ||
    (cp >= 0x1f18e && cp <= 0x1f19a) ||
    (cp >= 0x1f1e6 && cp <= 0x1f1ff) ||
    (cp >= 0x1f200 && cp <= 0x1f320) ||
    (cp >= 0x1f32d && cp <= 0x1f335) ||
    (cp >= 0x1f337 && cp <= 0x1f37c) ||
    (cp >= 0x1f37e && cp <= 0x1f393) ||
    (cp >= 0x1f3a0 && cp <= 0x1f3ca) ||
    (cp >= 0x1f3cf && cp <= 0x1f3d3) ||
    (cp >= 0x1f3e0 && cp <= 0x1f3f0) ||
    (cp >= 0x1f3f4 && cp <= 0x1f43e) ||
    (cp >= 0x1f440 && cp <= 0x1f4fc) ||
    (cp >= 0x1f4ff && cp <= 0x1f53d) ||
    (cp >= 0x1f54b && cp <= 0x1f567) ||
    (cp >= 0x1f5fb && cp <= 0x1f64f) ||
    (cp >= 0x1f680 && cp <= 0x1f6c5) ||
    (cp >= 0x1f6cc && cp <= 0x1f6d2) ||
    (cp >= 0x1f7e0 && cp <= 0x1f7eb) ||
    (cp >= 0x1f90c && cp <= 0x1f9ff) ||
    (cp >= 0x1fa70 && cp <= 0x1faff) ||
    (cp >= 0x20000 && cp <= 0x3fffd)
  ) return 2;
  return 1;
}

// 2 桁文字の右半分を表す番兵。render() で読み飛ばす。
const WIDE_CONTINUATION = '';

class MarkerGrid {
  private cellRows: string[][] = [];
  private row = 0;
  private col = 0;
  private savedRow = 0;
  private savedCol = 0;
  // 「最後に文字を書き込んだ行」。カーソル移動だけでは更新しない。
  // Ink はストリーミング中に同じブロックをまるごと再描画することがあり、古い世代の
  // 残骸（別の絶対行に書かれたまま上書きされない断片）が先に来ることがある。
  // 描画範囲をこの行までに絞ることで、後から来た最終世代より後ろに残らない古い残骸を
  // 自然に除外する（先に来て後で使われなくなった行は、最終世代の最後の書き込みより
  // 手前か奥かのどちらかにしかなり得ない前提。奥に残った場合のみ拾ってしまうが、
  // 実測ではこの前提が成立していた）。
  private lastWriteRow = -1;

  private ensureRow(r: number): number {
    r = clampInt(r, 0, MAX_VT_ROWS - 1);
    while (this.cellRows.length <= r) this.cellRows.push([]);
    return r;
  }

  private ensureCol(line: string[], c: number): void {
    while (line.length <= c) line.push(' ');
  }

  cr(): void { this.col = 0; }

  // 実端末の \n（LF のみ）は桁を戻さないが、マーカー本文は Ink が絶対位置指定で
  // 描くため地の文の \n はほぼ現れない。現れる場合（他 provider の素の \r\n 出力等）は
  // 論理行区切りとして扱うのが実用上正しいため、\n を桁 0 復帰つきの改行として扱う。
  lf(): void {
    this.row = clampInt(this.row + 1, 0, MAX_VT_ROWS - 1);
    this.col = 0;
    this.ensureRow(this.row);
  }

  tab(): void {
    const next = (Math.floor(this.col / 8) + 1) * 8;
    this.col = clampInt(next, 0, MAX_VT_COLS - 1);
  }

  moveTo(row1: number, col1: number): void {
    this.row = clampInt((row1 || 1) - 1, 0, MAX_VT_ROWS - 1);
    this.col = clampInt((col1 || 1) - 1, 0, MAX_VT_COLS - 1);
    this.ensureRow(this.row);
  }

  moveRel(dRow: number, dCol: number): void {
    this.row = clampInt(this.row + dRow, 0, MAX_VT_ROWS - 1);
    this.col = clampInt(this.col + dCol, 0, MAX_VT_COLS - 1);
    this.ensureRow(this.row);
  }

  colAbs(col1: number): void {
    this.col = clampInt((col1 || 1) - 1, 0, MAX_VT_COLS - 1);
  }

  save(): void { this.savedRow = this.row; this.savedCol = this.col; }
  restore(): void { this.row = this.savedRow; this.col = this.savedCol; this.ensureRow(this.row); }

  eraseLine(mode: number): void {
    const r = this.ensureRow(this.row);
    const line = this.cellRows[r];
    if (mode === 1) {
      this.ensureCol(line, this.col);
      for (let c = 0; c <= this.col; c++) line[c] = ' ';
    } else if (mode === 2) {
      this.ensureCol(line, MAX_VT_COLS - 1);
      for (let c = 0; c < line.length; c++) line[c] = ' ';
    } else {
      this.ensureCol(line, MAX_VT_COLS - 1);
      for (let c = this.col; c < line.length; c++) line[c] = ' ';
    }
  }

  eraseDisplay(mode: number): void {
    if (mode === 1) {
      for (let r = 0; r < this.row; r++) this.clearRow(r);
      const r = this.ensureRow(this.row);
      const line = this.cellRows[r];
      this.ensureCol(line, this.col);
      for (let c = 0; c <= this.col; c++) line[c] = ' ';
    } else if (mode === 2 || mode === 3) {
      // 画面クリアは「これより前は全部無関係」という明示シグナル。
      // 残すと空行の山になるので、蓄積そのものを破棄して仕切り直す。
      this.cellRows = [];
      this.lastWriteRow = -1;
    } else {
      const r = this.ensureRow(this.row);
      const line = this.cellRows[r];
      this.ensureCol(line, MAX_VT_COLS - 1);
      for (let c = this.col; c < line.length; c++) line[c] = ' ';
      for (let rr = this.row + 1; rr < this.cellRows.length; rr++) this.clearRow(rr);
    }
  }

  private clearRow(r: number): void {
    if (r < 0 || r >= this.cellRows.length) return;
    const line = this.cellRows[r];
    for (let c = 0; c < line.length; c++) line[c] = ' ';
  }

  writeChar(ch: string, width: number): void {
    if (width <= 0) return;
    if (this.col + width > MAX_VT_COLS) this.lf();
    const r = this.ensureRow(this.row);
    const line = this.cellRows[r];
    this.ensureCol(line, this.col + width - 1);
    line[this.col] = ch;
    for (let i = 1; i < width; i++) line[this.col + i] = WIDE_CONTINUATION;
    this.col = clampInt(this.col + width, 0, MAX_VT_COLS - 1);
    this.lastWriteRow = r;
  }

  render(): string {
    if (this.lastWriteRow < 0) return '';
    const lines: string[] = [];
    for (let r = 0; r <= this.lastWriteRow; r++) {
      const cells = this.cellRows[r] || [];
      let line = '';
      for (const c of cells) { if (c !== WIDE_CONTINUATION) line += c; }
      line = line.replace(/[ \t]+$/, '');
      lines.push(line);
    }
    // 冒頭の完全な空行（本文がもっと下の絶対行から始まるだけのケース）は見た目のノイズなので削る。
    while (lines.length > 0 && lines[0] === '') lines.shift();
    return lines.join('\n');
  }
}

function csiFinalByte(ch: string): boolean {
  const cp = ch.codePointAt(0) || 0;
  return cp >= 0x40 && cp <= 0x7e;
}

function applyCSI(grid: MarkerGrid, body: string, final: string | undefined): void {
  const private_ = body.startsWith('?');
  const paramsStr = private_ ? body.slice(1) : body;
  const params = paramsStr.length > 0 ? paramsStr.split(';').map((s) => parseInt(s, 10) || 0) : [];
  const p = (idx: number, def: number) => params[idx] || def;
  switch (final) {
    case 'A': grid.moveRel(-p(0, 1), 0); break;
    case 'B': grid.moveRel(p(0, 1), 0); break;
    case 'C': grid.moveRel(0, p(0, 1)); break;
    case 'D': grid.moveRel(0, -p(0, 1)); break;
    case 'G': grid.colAbs(p(0, 1)); break;
    case 'H':
    case 'f': grid.moveTo(p(0, 1), p(1, 1)); break;
    case 'J': grid.eraseDisplay(params[0] || 0); break;
    case 'K': grid.eraseLine(params[0] || 0); break;
    case 's': grid.save(); break;
    case 'u': grid.restore(); break;
    default: break; // SGR('m')・DEC private mode 等はテキスト構造に影響しないため無視
  }
}

// マーカーブロック本文（ANSI 込みの UTF-8 デコード済み文字列）を、実端末に近い
// 軽量グリッドへ描画してから読みやすいプレーンテキストへ変換する。純関数。
export function renderMarkerBytesToText(text: string): string {
  const grid = new MarkerGrid();
  const chars = Array.from(text); // コードポイント単位（サロゲートペア対応）
  let i = 0;
  while (i < chars.length) {
    const ch = chars[i];
    if (ch === '\x1b') {
      const next = chars[i + 1];
      if (next === '[') {
        i += 2;
        let body = '';
        while (i < chars.length && !csiFinalByte(chars[i])) { body += chars[i]; i++; }
        const final = chars[i];
        i++;
        applyCSI(grid, body, final);
        continue;
      }
      if (next === ']' || next === 'P' || next === 'X' || next === '^' || next === '_') {
        // OSC / DCS / SOS / PM / APC: BEL または ST(ESC \) まで読み飛ばす
        i += 2;
        while (i < chars.length) {
          if (chars[i] === '\x07') { i++; break; }
          if (chars[i] === '\x1b' && chars[i + 1] === '\\') { i += 2; break; }
          i++;
        }
        continue;
      }
      if (next === '7') { grid.save(); i += 2; continue; }
      if (next === '8') { grid.restore(); i += 2; continue; }
      // ESC + 単一バイト（中立）。next が無い（末尾で ESC 単体）場合も 1 つだけ進める。
      i += next === undefined ? 1 : 2;
      continue;
    }
    if (ch === '\r') { grid.cr(); i++; continue; }
    if (ch === '\n') { grid.lf(); i++; continue; }
    if (ch === '\b') { grid.moveRel(0, -1); i++; continue; }
    if (ch === '\t') { grid.tab(); i++; continue; }
    const cp = ch.codePointAt(0) || 0;
    if (cp < 0x20) { i++; continue; } // その他の制御文字は無視
    const w = runeCellWidth(cp);
    if (w > 0) grid.writeChar(ch, w);
    i++;
  }
  return grid.render();
}
