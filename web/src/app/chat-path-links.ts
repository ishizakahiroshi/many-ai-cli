// チャット表示の本文（URL 以外の区間）から、ファイルパスとして扱う部分を切り出す。
// DOM には触らない。描画は chat-history.ts が行う。

export type ChatTextPiece = { text: string; path?: string };

// path 候補:
//   - 絶対パス Unix: /usr/... (ただしコードブロック外)
//   - 絶対パス Windows: C:\... or C:/...
//   - 相対パス: ./xxx ../xxx
//   - 拡張子付き相対: foo/bar.ext または bar.ext (拡張子に絞る)
// 拡張子付き相対の本体は 255 文字まで（{0,255}）。上限が無いと、空白も拡張子も
// 無い長い 1 行（16 進の列など）で開始位置ごとに後戻りが続き、長さの二乗の時間が
// かかって画面全体が止まる（U30）。それより長い相対パスは ./ か / で始まる形だけ
// リンクになる。
function chatPathPattern(): RegExp {
  return /((?:[a-zA-Z]:[\\/]|[.]{1,2}[\\/]|\/)[^\s<>"'`(\]]+)|([\w][\w\-/\\.]{0,255}\.[a-zA-Z]{1,8}\b)/g;
}

// 安全側: 末尾の句読点 ,.;:!?) などはパスに含めない。
export function splitChatPaths(text: string): ChatTextPiece[] {
  const pieces: ChatTextPiece[] = [];
  if (!text) return pieces;
  const re = chatPathPattern();
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) pieces.push({ text: text.slice(last, m.index) });
    // 上限で途中から始まった相対パス（直前も語の文字）は、別の場所を指す途中の
    // 切れ端なのでリンクにしない。
    if (m[2] !== undefined && m.index > 0 && /\w/.test(text[m.index - 1])) {
      pieces.push({ text: m[0] });
      last = m.index + m[0].length;
      continue;
    }
    let token = m[0];
    let trail = '';
    while (token.length > 0 && /[.,;:!?)\]}>]/.test(token[token.length - 1])) {
      trail = token[token.length - 1] + trail;
      token = token.slice(0, -1);
    }
    if (!token) {
      pieces.push({ text: m[0] });
    } else {
      pieces.push({ text: token, path: token });
      if (trail) pieces.push({ text: trail });
    }
    last = m.index + m[0].length;
  }
  if (last < text.length) pieces.push({ text: text.slice(last) });
  return pieces;
}
