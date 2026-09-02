// xterm に渡すバイト列から [MANY-AI-CLI]…[/MANY-AI-CLI] / [MANY-AI-CLI-DONE]…
// [/MANY-AI-CLI-DONE] の OPEN/CLOSE タグを剥がす純粋関数。
// 承認ブロック・完了サマリー（DONE）ブロックとも、本文は CLI が描いたまま素通しし、
// 剥がすのはタグ文字列だけ（案 J）。出力を捨てる分岐は持たない。
// terminal.ts の filterHubMarkersForDisplay から状態の橋渡しのみ受けて呼ばれる。
// 純関数として切り出すことで DOM/xterm 依存なしで node:test から検証可能にしている。
//
// 案 E（2026-06-24）: マーカーブロック内の本文を一旦 buf に貯め、CLOSE 到達時に
// ANSI escape（CSI / OSC / ESC+単一バイト / 裸 ESC）を全部除去して xterm に出す。
// 案 B（2026-06-23）の「本文ごと非表示」では CLI 側で質問本文が読めず、案 C（タグだけ
// 剥がし本文 pass-through）では Claude Code の INK popup が出す絶対カーソル位置指定
// (\x1b[<row>;<col>H 等) が xterm のスクロールと衝突して画面崩れを起こす可能性が残った。
// 案 E は「衝突原理を生む ESC シーケンスをマーカー内では一切通さない」ことで両立する。
// 質問本文の色情報は失うが、現状そこに色は付いていないので実害なし。
//
// close 後の erase-below は、OPEN マーカー到達前に部分流出した popup 残骸を掃除する保険。
//
// 案 F（2026-08-24）: 案 E は CSI を単純に削除するだけだったため、Ink が行区切りを
// 絶対カーソル位置指定だけで表現している場合（実測: セッション #23）、行区切りが
// 何にも変換されずに消えて複数行の本文が 1 行へ繋がって見えていた。軽量 VT グリッド
// （marker-vt-render.ts）へ一旦描画してから読み出す方式に変更し、カーソル移動による
// 行送り・同じ行への上書き描画（スピナー等の再描画）を実際の見た目どおりに畳み込んだ。
// ※ 案 H で書き戻し自体をやめたため、このグリッドは不要になり同ファイルは削除済み。
//
// 案 G（2026-08-26）: DONE ブロックだけは本文を端末へ書き戻さない。**承認ブロックは従来どおり。**
//
// 案 E〜F はブロック本文を「現在のカーソル位置から」書き足す。Claude Code は代替画面
// バッファで画面全体を Ink が絶対座標で管理しているため、この書き足しは Ink の管理外の
// セルを汚し、以後どれだけ再描画されても消えない。VT グリッドが 200 桁なのに実画面が
// 128 桁だと、行頭に残った 90〜111 個の空白ごと折り返して 2 行を食い、文の途中から
// 始まる断片として残る（2026-08-26 実測: セッション #3 のログを 10:23:16 で打ち切って
// 128x35 へ再生し、DONE ブロックを落とすと残骸 0 行、落とさないと 2 行 + サマリー本文 1 行）。
//
// 承認ブロックを同じ扱いにしないのは、質問本文が端末で読めなくなると案 B（2026-06-23）
// の欠点がそのまま戻るため。DONE を落としてよいのは、完了サマリーが Hub 側で
// done_summary として UI へ配信されており（internal/hub/done_summary.go の
// publishDoneSummary は通知設定と無関係に broadcast する）、端末表示が唯一の経路では
// ないから。落とすと同時に UI 側へ受け手を用意することがセットの前提（ws-client.ts の
// done_summary ハンドラ / live status 帯 / セッションカード）。
//
// なお DONE では close 後の erase-below も出さない。本文を書かない＝カーソルが動かない
// ため、そこで erase-below を送ると Ink が描いた画面下部を消すだけになる。
//
// 案 H（2026-08-26）: 承認ブロックはタグ文字列だけを剥がし、本文と CLI の位置指定はそのまま
// 通す（案 C 相当）。案 E〜F の「溜めて描き直して現在カーソル位置へ書き戻す」をやめる。
// DONE ブロックの扱い（案 G の本文破棄）は変えない。
//
// 案 G と同じ理屈が承認ブロックにもそのまま当てはまっていた。代替画面バッファでは Ink が
// 全セルを絶対座標で管理しているので、書き戻した本文は Ink の管理外セルに乗り、以後どれだけ
// 再描画されても消えない。VT グリッドが 200 桁・実画面が 128 桁だと行末パディングごと
// 折り返して 2 行を食い、行頭に長い空白を伴う断片が並ぶ。
//
// 2026-08-26 実測（セッション #9 のログを 14:10:10 で打ち切り、記録どおりの寸法変化つきで
// @xterm/headless 6.0 へ再生）:
//   フィルタ無し          崩れ 0 行（リサイズ無し / 1 秒前倒し / チャンク 1・4 個欠落でも同じ）
//   案 E〜F を通す        利用者のスクリーンショットと一行単位で一致する崩れが再現
//   タグのみ除去（案 H）  崩れ 0 行・質問本文も読める
//
// 案 C（2026-06-23）を当時見送った理由は「Ink の絶対カーソル位置指定が xterm のスクロールと
// 衝突する可能性が残る」という懸念だったが、実際に衝突していたのは書き戻す側だった。本文は
// CLI 自身が既に描いているので、こちらは何も足さないのが正しい。
//
// 案 I（2026-09-02）: マーカー文字列そのものが端末の折り返しで分断される。
// 案 E〜H はどれも「マーカーは連続したバイト列として届く」を前提に完全一致で探していたが、
// CLI は端末幅で長い行を折り返すので、マーカーの途中に CR / LF・行末パディングの空白・
// カーソル移動の CSI が割り込む。実測（2026-09-02 セッション #3・Codex 130 桁）:
//   [/MANY-AI-CLI-  CR LF  空白 130  CR LF  空白 130 …  ESC[?25l ESC[35;1H LF ESC[30;1H  DONE]
// この形だと DONE の CLOSE が永久に一致せず、ラッチしたまま以降の端末出力を全部捨てる。
// 復帰するのは 32KB 溜まってセーフガードが働くときだけで、CLI が上限などで止まっていると
// 新しい出力が来ないため永久に画面が固まる（利用者からは「送ったのに何も起きない」に見える）。
//
// 実害の実測（同日のセッションログ 17 本を filterHubMarkersPure へ再生）:
//   codex #3   1,417,693B 中 441,727B（31.2%）が xterm へ届かず・ログ末尾でも飲み込んだまま
//   claude #7  557,317B 中 68,189B（12.2%）  /  grok #15  1,656,211B 中 215,711B（13.0%）
// provider 横断で起きる。折り返し位置が CLOSE を跨ぐかどうかは完了サマリーの長さと端末幅
// 次第なので、特定 CLI の癖ではない。
//
// 対処: マーカー照合を折り返し耐性ありにする（matchMarkerAllowingWrap）。マーカー文字の
// 間に限り CR / LF / 空白 / タブ / 完結した ESC シーケンスを読み飛ばす。印字文字が 1 つでも
// 割り込んだ時点で不一致にするので、地の文が誤ってマーカー扱いされることはない。読み飛ばし
// 総量は MAX_MARKER_WRAP_GAP_BYTES で頭打ちにして、病的な走査と carry の肥大を防ぐ。
// 割り込んだバイトの扱いは既存の方針をそのまま踏襲する。承認ブロック（案 H = 何も足さない・
// 何も削らない）では素通しし、DONE ブロック（案 G = ブロックごと落とす）では捨てる。
//
// 案 J（2026-09-03）: DONE ブロックも承認ブロックと同じ「タグだけ剥がす」へ揃える。案 G の
// 「本文ごと落とす」をやめ、出力を捨てる経路そのものを無くす。
//
// 案 G〜I はどれも「OPEN が来たら CLOSE が来るまで捨てる」というストリーム前提の状態機械
// だった。ところが代替画面では CLI が 2 次元のセルを絶対座標で塗り直すので、**1 回の再描画に
// OPEN だけが入り、CLOSE は画面外にあって 1 バイトも描かれない**ことが普通に起きる。CLOSE が
// 折り返しで分断されている（案 I が直した形）のではなく、そもそも存在しない。案 I の折り返し
// 耐性では原理的に届かない。
//
// 2026-09-03 実測（セッション #7 claude・利用者が処理中に遡ろうとした 5 分間のログを
// filterHubMarkersPure へ再生）:
//   1,711,596B 中 330,205B（19.3%）が xterm へ届かない
//   最初のホイールまでの 243,471B では 792B（正規の DONE ブロック 1 個）しか落ちていない
//   01:32:47 にラッチしてから解除まで 3 分 6 秒。解除も CLOSE 一致ではなく 32KB セーフガード
//   DONE の OPEN リテラルは 95 回、CLOSE リテラルは 79 回しか描かれていない（差の 16 回が事故）
// 利用者からは「処理中にホイールで遡ろうとしても画面がまったく動かない」に見える。CLI 側は
// 正常にホイールを受けて再描画を返しており、その再描画が全部ここで捨てられていた。
//
// 案 G が本文を落としていた理由は「案 E〜F の書き戻しが Ink 管理外のセルを汚す」ことだったが、
// 案 H で書き戻し自体をやめた時点でその理由は消えている。本文は CLI 自身が既に描いており、
// こちらは何も足さない・何も削らないのが正しい（案 H が承認ブロックで出した結論と同じ）。
// 代償は完了サマリーの数行が端末にも見えること。Hub 側の done_summary 配信は従来どおり残る。
//
// これで出力を捨てる分岐が 1 つも無くなるので、CLOSE の有無に関わらず端末が固まらない。
// inDone / inMarker は「ブロックの中か」を示すだけの札になり、stray な CLOSE を位置に関係なく
// 受理してよいかの判定にしか使わない。


export const hubMarkerBytePatterns = [
  new TextEncoder().encode('[MANY-AI-CLI]'),
  new TextEncoder().encode('[/MANY-AI-CLI]'),
];
export const hubMarkerEndBytes = hubMarkerBytePatterns[1];
export const hubDoneMarkerOpen = new TextEncoder().encode('[MANY-AI-CLI-DONE]');
export const hubDoneMarkerClose = new TextEncoder().encode('[/MANY-AI-CLI-DONE]');

// ラッチ解除の閾値（2026-07-01 導入 / 案 J で役割が縮小）。実運用のマーカー本文は数百 B〜数 KB。
// close マーカー typo（例: [/MANARY-AI-CLI-DONE]）や再描画で close が描かれないと、
// inMarker / inDone がラッチしたままになる。案 J 以降はラッチしていても本文を素通しするので
// 出力は欠けないが、放置すると以後の stray close を位置に関係なく受理してしまう。閾値を
// 超えたらラッチだけ解除する。閾値は正常運用の 10 倍以上のマージン。
export const MAX_MARKER_BUFFER_BYTES = 32 * 1024;

export function bytesStartWith(bytes: Uint8Array, offset: number, pattern: Uint8Array): boolean {
  if (offset + pattern.length > bytes.length) return false;
  for (let i = 0; i < pattern.length; i++) {
    if (bytes[offset + i] !== pattern[i]) return false;
  }
  return true;
}

export function isPossiblePrefix(bytes: Uint8Array, offset: number, patterns: Uint8Array[]): boolean {
  const remaining = bytes.length - offset;
  return patterns.some((pattern) => {
    if (remaining >= pattern.length) return false;
    for (let i = 0; i < remaining; i++) {
      if (bytes[offset + i] !== pattern[i]) return false;
    }
    return true;
  });
}

// 案 I: マーカー文字の間に読み飛ばしてよい折り返し由来バイトの総量。
// 実測の割り込みは 130 桁 × 2 行 + CSI 数個で 400B 前後。4KB あればどの端末幅でも足りるうえ、
// 上限を持たせることで「'[' の後ろが延々と空白」のような入力で carry が肥大するのを防ぐ。
export const MAX_MARKER_WRAP_GAP_BYTES = 4096;

const EMPTY_BYTES = new Uint8Array(0);

export type MarkerMatch = {
  // >0 = 一致して消費したバイト数 / 0 = 不一致 / -1 = バイト不足（carry して次チャンクへ）
  consumed: number;
  // マーカー文字列の途中へ折り返しが割り込ませたバイト（CR / LF / 空白 / タブ / ESC シーケンス）
  noise: Uint8Array;
};

const MARKER_NO_MATCH: MarkerMatch = { consumed: 0, noise: EMPTY_BYTES };
const MARKER_NEED_MORE: MarkerMatch = { consumed: -1, noise: EMPTY_BYTES };

function isWrapNoiseByte(b: number): boolean {
  return b === 0x0d || b === 0x0a || b === 0x20 || b === 0x09;
}

// offset から ESC シーケンス 1 個を読み飛ばす。
// 戻り値: 消費バイト数 / 0 = ESC で始まっていない / -1 = 終端が来る前にバイトが尽きた
export function skipWrapEscape(bytes: Uint8Array, offset: number): number {
  if (bytes[offset] !== 0x1b) return 0;
  let j = offset + 1;
  if (j >= bytes.length) return -1;
  const kind = bytes[j];
  if (kind === 0x5b) { // CSI: 終端は 0x40〜0x7e
    j++;
    while (j < bytes.length) {
      const b = bytes[j];
      j++;
      if (b >= 0x40 && b <= 0x7e) return j - offset;
    }
    return -1;
  }
  if (kind === 0x5d) { // OSC: BEL または ESC \ で終わる
    j++;
    while (j < bytes.length) {
      const b = bytes[j];
      j++;
      if (b === 0x07) return j - offset;
      if (b === 0x1b) {
        if (j >= bytes.length) return -1;
        return j + 1 - offset;
      }
    }
    return -1;
  }
  return 2; // ESC + 単一バイト
}

// 案 I: 折り返しで分断されたマーカーも一致させる。マーカーの 1 文字目には割り込みを認めず、
// 2 文字目以降の隙間でだけ CR / LF / 空白 / タブ / 完結した ESC シーケンスを読み飛ばす。
// 印字文字が割り込んだ時点で不一致にするので、地の文の誤爆は増えない。
export function matchMarkerAllowingWrap(
  bytes: Uint8Array,
  offset: number,
  pattern: Uint8Array,
): MarkerMatch {
  if (offset >= bytes.length) return MARKER_NEED_MORE;
  if (bytes[offset] !== pattern[0]) return MARKER_NO_MATCH;
  let i = offset + 1;
  let p = 1;
  let gap = 0;
  let noise: number[] | null = null;
  while (p < pattern.length) {
    if (i >= bytes.length) return MARKER_NEED_MORE;
    if (bytes[i] === pattern[p]) { i++; p++; continue; }
    const esc = skipWrapEscape(bytes, i);
    if (esc < 0) return MARKER_NEED_MORE;
    if (esc > 0) {
      if (!noise) noise = [];
      for (let k = 0; k < esc; k++) noise.push(bytes[i + k]);
      i += esc;
      gap += esc;
    } else if (isWrapNoiseByte(bytes[i])) {
      if (!noise) noise = [];
      noise.push(bytes[i]);
      i++;
      gap++;
    } else {
      return MARKER_NO_MATCH;
    }
    if (gap > MAX_MARKER_WRAP_GAP_BYTES) return MARKER_NO_MATCH;
  }
  return { consumed: i - offset, noise: noise ? new Uint8Array(noise) : EMPTY_BYTES };
}

export type HubMarkerFilterState = {
  carry: Uint8Array;
  inDone: boolean;
  inMarker: boolean;
  // 案 H / 案 J: ブロック本文は貯めずに素通しするので、持つのは open からのバイト数だけ。
  // close が来ないまま肥大したときのラッチ解除にしか使わない。
  markerSeen?: number;
  doneSeen?: number;
  // 行頭ゲート（2026-07-04）: OPEN マーカーは「行頭（空白のみ先行）」でのみラッチする。
  // AI が地の文でマーカーをリテラル引用した場合（例:「…はすべて [MANY-AI-CLI] マーカー形式で…」）、
  // close が来ないまま inMarker にラッチし、後続の画面再描画 32KB を飲み込む事故の再発防止
  // （2026-07-04 orchestration 指揮者セッションで実測）。正規マーカーは Claude Code の描画上
  // 必ず「行境界（\r\n または CUP）＋インデント空白」の直後に現れることをログで確認済み。
  lineStart?: boolean;
  // lineStart 判定用の ESC シーケンス解析フェーズ（0=通常 / 1=ESC 直後 / 2=CSI 中 / 3=OSC 中）
  escPhase?: number;
};

export function filterHubMarkersPure(bytes: Uint8Array, state: HubMarkerFilterState): {
  out: Uint8Array;
  state: HubMarkerFilterState;
} {
  const carry = state.carry || new Uint8Array(0);
  const combined = new Uint8Array(carry.length + bytes.length);
  combined.set(carry, 0);
  combined.set(bytes, carry.length);

  const out: number[] = [];
  let i = 0;
  let inDone = state.inDone;
  let inMarker = state.inMarker;
  let markerSeen = state.markerSeen ?? 0;
  let doneSeen = state.doneSeen ?? 0;
  let lineStart = state.lineStart ?? true;
  let escPhase = state.escPhase ?? 0;

  // 行頭ゲート用の per-byte 状態機械。\r・\n・CUP/HVP（CSI ... H / f）で行頭に復帰し、
  // 空白・制御文字・ESC シーケンスは中立、印字文字（マルチバイト含む）で行頭を解除する。
  // Ink はインデントを空白または CUP の列指定で描くため、これで
  // 「行境界＋空白のみ先行」= 正規マーカー位置 と「文中」= prose リテラル を区別できる。
  const trackByte = (b: number): void => {
    switch (escPhase) {
      case 1: // ESC 直後
        if (b === 0x5b) escPhase = 2;        // CSI
        else if (b === 0x5d) escPhase = 3;   // OSC
        else escPhase = 0;                    // ESC+単一バイト（中立）
        return;
      case 2: // CSI 中
        if (b >= 0x40 && b <= 0x7e) {
          escPhase = 0;
          if (b === 0x48 || b === 0x66) lineStart = true; // CUP 'H' / HVP 'f'
        }
        return;
      case 3: // OSC 中
        if (b === 0x07) escPhase = 0;
        else if (b === 0x1b) escPhase = 1;
        return;
      default:
        if (b === 0x1b) { escPhase = 1; return; }
        if (b === 0x0a || b === 0x0d) { lineStart = true; return; }
        if (b === 0x20 || b === 0x09 || b < 0x20) return; // 空白・制御文字は中立
        lineStart = false;
    }
  };

  while (i < combined.length) {
    // 完了マーカー。OPEN と CLOSE は 2 バイト目（'M' / '/'）で分岐するので同時には一致しない。
    // 案 J: 承認マーカーと同型に扱い、タグ文字列だけを剥がして本文は素通しする。
    const doneOpen = matchMarkerAllowingWrap(combined, i, hubDoneMarkerOpen);
    const doneClose = doneOpen.consumed > 0
      ? MARKER_NO_MATCH
      : matchMarkerAllowingWrap(combined, i, hubDoneMarkerClose);
    const doneTag = doneOpen.consumed > 0 ? doneOpen
      : (doneClose.consumed > 0 ? doneClose : null);
    if (doneTag) {
      const isDoneClose = doneTag === doneClose;
      // 行頭ゲート: OPEN は行頭のみラッチ。close は inDone 中なら位置を問わず受理し、
      // stray close（開きなし）は行頭のみマーカー扱いにする（承認側と同じ規則）。
      // 承認ブロックの本文に DONE マーカーのリテラルが現れても DONE として扱わないよう、
      // 案 E〜G と同じ「ブロック内では相手方のマーカーを見ない」挙動を維持する。
      if (!inMarker && (inDone || lineStart)) {
        i += doneTag.consumed;
        lineStart = false;
        // 案 I: 折り返しがタグの途中へ割り込ませた CR / LF / 空白 / ESC は本文側の描画指示
        // なので落とさず素通しする（案 J の「本文へ 1 バイトも足さない・削らない」を保つ）。
        for (const b of doneTag.noise) out.push(b);
        inDone = !isDoneClose;
        doneSeen = 0;
        continue;
      }
      // 文中の prose リテラルはマーカー扱いせず素通し（'[' 1 バイトだけ進めて再走査）
      trackByte(combined[i]);
      out.push(combined[i]);
      i++;
      continue;
    }

    // 承認マーカー。OPEN と CLOSE は 2 バイト目で分岐するので同時には一致しない。
    const approvalOpen = matchMarkerAllowingWrap(combined, i, hubMarkerBytePatterns[0]);
    const approvalClose = approvalOpen.consumed > 0
      ? MARKER_NO_MATCH
      : matchMarkerAllowingWrap(combined, i, hubMarkerEndBytes);
    const marker = approvalOpen.consumed > 0 ? approvalOpen
      : (approvalClose.consumed > 0 ? approvalClose : null);
    if (marker) {
      const isClose = marker === approvalClose;
      // 行頭ゲート: OPEN は行頭のみラッチ。close は inMarker 中なら位置を問わず受理し、
      // stray close（開きなし）は行頭のみマーカー扱い（文中の prose リテラルは素通し）。
      if (!inDone && (inMarker || lineStart)) {
        i += marker.consumed;
        lineStart = false;
        // 案 H: open / close ともタグ文字列を落とすだけ。本文は下の分岐で素通しする。
        // 案 I: 折り返しがタグの途中へ割り込ませた CR / LF / 空白 / ESC は本文側の描画指示
        // なので落とさず素通しする（案 H の「本文へ 1 バイトも足さない・削らない」を保つ）。
        for (const b of marker.noise) out.push(b);
        // close 後の erase-below も出さない。こちらは何も書いていないので消す残骸が無く、
        // 送れば Ink が描いた画面下部を消すだけになる（案 G と同じ理由）。
        inMarker = !isClose;
        markerSeen = 0;
        continue;
      }
      trackByte(combined[i]);
      out.push(combined[i]);
      i++;
      continue;
    }
    // マーカー prefix の carry は「ラッチし得る文脈」のときだけ行う（文中は素通しでよい）
    const needMoreForMarker = doneOpen.consumed < 0
      || doneClose.consumed < 0
      || approvalOpen.consumed < 0
      || approvalClose.consumed < 0;
    if ((inDone || inMarker || lineStart) && needMoreForMarker) break;
    // 案 H / 案 J: 承認ブロックも DONE ブロックも本文は素通しする（落とす分岐はもう無い）。
    trackByte(combined[i]);
    out.push(combined[i]);
    i++;
    // typo/欠落/再描画で close が来ない: ラッチしたままだと以後の stray close を位置に
    // 関係なく受理してしまうため状態だけ戻す（本文は素通し済みで捨てるものは無い）。
    if (inMarker) {
      markerSeen++;
      if (markerSeen > MAX_MARKER_BUFFER_BYTES) {
        inMarker = false;
        markerSeen = 0;
      }
    }
    if (inDone) {
      doneSeen++;
      if (doneSeen > MAX_MARKER_BUFFER_BYTES) {
        inDone = false;
        doneSeen = 0;
      }
    }
  }

  return {
    out: new Uint8Array(out),
    state: {
      carry: combined.slice(i),
      inDone,
      inMarker,
      markerSeen,
      doneSeen,
      lineStart,
      escPhase,
    },
  };
}
