// terminal-geometry.ts
// 一時観測: 承認ポップアップ表示中にスクロールすると画面が重なって描かれ、直前の内容が
// 読めなくなる症状の切り分け用（docs/local/bugfix_terminal-grid-divergence-on-scroll_2026-08-24.md）。
//
// 2026-08-24 の実測で、PTY へ通知した寸法（128 桁）で生バイト列を再生すると画面はきれいに
// なることが分かっている。つまり壊れているのは CLI の出力ではなく表示側で、xterm の格子が
// PTY へ伝えた寸法から外れている疑いがある。外れる瞬間を掴むのがこの観測の目的。
//
// 記録するのは 4 つの記録点と 1 本の定期サンプラー。
//   geo.fit     fitTerminalPreservingBottom の fit 直後。xterm の寸法が動いた瞬間と、
//               そのとき PTY 送信が抑制されていたか
//   geo.send    sendResize の結果。sent / dedup / ws-closed を分けて残す
//   geo.apply   Hub からの pty_resize 反映。PTY 実寸と xterm 実寸の突き合わせ
//   geo.scroll  PgUp / PgDn を送る瞬間の xterm 実寸と PTY 通知済み寸法
//   sample      2 秒ごとに両者を比べ、変化したときとずれているときだけ 1 行出す
//
// ゲートは 2 層。build 側（MAI_DEBUG=1 でなければ成果物に入らない）と runtime 側
// （URL クエリ ?geodebug=1 のときだけ sink を登録しサンプラーを回す）。既定では 1 バイトも送らない。
// 送るのは寸法・件数・状態フラグだけで、ターミナル本文と入力テキストは含まない。
// 原因が確定したら撤去する（instrumentation.json の terminal-grid-divergence）。

import { activeSessionId, terminals } from '../app/state.js';
import { debugLastSentPtySize } from '../app/terminal.js';
import { apiFetch } from '../app/util.js';
import { registerProbeSink, type ProbeFields } from './probe.js';

const SAMPLE_INTERVAL_MS = 2000;
// 開いたまま放置しても増え続けないよう、送信総数に蓋をする。
const MAX_POSTS = 600;

function isEnabled(): boolean {
  try {
    return new URLSearchParams(location.search).get('geodebug') === '1';
  } catch (_) {
    return false;
  }
}

let posted = 0;

function post(kind: string, fields: ProbeFields): void {
  if (posted >= MAX_POSTS) return;
  posted++;
  void apiFetch('/api/debug/terminal-geometry', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ kind, seq: posted, ...fields }),
  }).catch(() => {
    // 観測が本体の動作を止めないよう握り潰す。
  });
}

// 承認バーが出ているかどうか。ターミナルの表示領域を変える側なので一緒に残す。
function approvalBarVisible(): boolean {
  const bar = document.getElementById('action-bar');
  return !!bar && bar.classList.contains('visible');
}

interface SampleShape {
  cols: number;
  rows: number;
  lastSent: string;
  bar: boolean;
  elemW: number;
  elemH: number;
}

let lastShape: SampleShape | null = null;

function sample(): void {
  const id = activeSessionId;
  if (id === null || id === undefined) return;
  // TODO(ts): TerminalEntry.term は state.ts 側で any 定義のため any のまま扱う。
  const entry: any = terminals.get(id);
  if (!entry?.term) return;
  const shape: SampleShape = {
    cols: entry.term.cols,
    rows: entry.term.rows,
    lastSent: debugLastSentPtySize(id),
    bar: approvalBarVisible(),
    elemW: entry.container?.clientWidth ?? -1,
    elemH: entry.container?.clientHeight ?? -1,
  };
  const diverged = shape.lastSent !== '' && shape.lastSent !== `${shape.cols}x${shape.rows}`;
  const changed = !lastShape
    || lastShape.cols !== shape.cols
    || lastShape.rows !== shape.rows
    || lastShape.lastSent !== shape.lastSent
    || lastShape.bar !== shape.bar
    || lastShape.elemW !== shape.elemW
    || lastShape.elemH !== shape.elemH;
  lastShape = shape;
  // 変化が無く、ずれてもいないときは 1 行も出さない（hub.log を埋めないため）。
  if (!changed && !diverged) return;
  post('sample', {
    sessionId: id,
    ...shape,
    diverged,
    altBuffer: entry.term.buffer?.active?.type === 'alternate',
    dpr: window.devicePixelRatio,
  });
}

if (isEnabled()) {
  registerProbeSink('geo.fit', (_channel, fields) => post('fit', { ...fields, bar: approvalBarVisible() }));
  registerProbeSink('geo.send', (_channel, fields) => post('send', { ...fields, bar: approvalBarVisible() }));
  registerProbeSink('geo.apply', (_channel, fields) => post('apply', { ...fields, bar: approvalBarVisible() }));
  registerProbeSink('geo.scroll', (_channel, fields) => post('scroll', { ...fields, bar: approvalBarVisible() }));
  window.setTimeout(sample, 1000);
  window.setInterval(sample, SAMPLE_INTERVAL_MS);
}
