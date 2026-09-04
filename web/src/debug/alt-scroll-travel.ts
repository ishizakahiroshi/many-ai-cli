// alt-scroll-travel.ts
// 一時観測: 代替画面のセッションで「上へは一部行くが、昔の内容まで遡れない」症状の切り分け用
// （docs/local/bugfix_alt-screen-scroll-up-stalls-after-turn-end_2026-09-04.md）。
//
// 測りたいのは 3 つ。ログ（~/.many-ai-cli/logs/sessions/*.jsonl）にはホイールもボタンも
// 同じバイト列で出るため、外形からは区別も計数もできない。
//   1. `↑ up` の 1 クリック（12 ノッチ要求）が実際に何ノッチ送られ、何ノッチ確定したか
//   2. 途中で打ち切られた場合、何ノッチ目で止まったか（altscroll.timeout の sent/confirmed）
//   3. 1 ノッチで CLI の画面が実際に何行動いたか（altscroll.confirm の shift）
//      xterm の実バッファから測るので、日本語画面でも桁ずれしない
//
// あわせて、確定できなかったのに画面は動いていた回（altscroll.reject の shift != 0）を残す。
// ここが多ければ、直す対象は「移動距離」ではなく「確定条件」になる。
//
// ゲートは 2 層。build 側（MAI_DEBUG=1 でなければ成果物に入らない）と runtime 側
// （URL クエリ ?scrolldebug=1 のときだけ sink を登録する）。既定では 1 バイトも送らない。
// 送るのは件数・行数・方向・セッション ID だけで、ターミナル本文と入力テキストは含まない。
// 原因が確定したら撤去する（instrumentation.json の alt-scroll-travel）。

import { apiFetch } from '../app/util.js';
import { registerProbeSink, type ProbeFields } from './probe.js';

// 開いたまま放置しても増え続けないよう、送信総数に蓋をする。
const MAX_POSTS = 600;

function isEnabled(): boolean {
  try {
    return new URLSearchParams(location.search).get('scrolldebug') === '1';
  } catch (_) {
    return false;
  }
}

let posted = 0;

function post(kind: string, fields: ProbeFields): void {
  if (posted >= MAX_POSTS) return;
  posted++;
  void apiFetch('/api/debug/alt-scroll', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ kind, seq: posted, ...fields }),
  }).catch(() => {
    // 観測が本体の動作を止めないよう握り潰す。
  });
}

// 捨てたホイールは 1 件ずつ出すと的が絞れないので、500ms ぶんまとめて件数だけ出す。
let dropUp = 0;
let dropDown = 0;
let dropTimer: number | null = null;

function flushDrops(): void {
  dropTimer = null;
  if (dropUp === 0 && dropDown === 0) return;
  post('drop', { up: dropUp, down: dropDown });
  dropUp = 0;
  dropDown = 0;
}

if (isEnabled()) {
  registerProbeSink('altscroll.request', (_channel, fields) => post('request', fields));
  registerProbeSink('altscroll.send', (_channel, fields) => post('send', fields));
  registerProbeSink('altscroll.confirm', (_channel, fields) => post('confirm', fields));
  registerProbeSink('altscroll.timeout', (_channel, fields) => post('timeout', fields));
  // 却下は PTY flush ごとに来る。「画面は動いたのに確定できなかった」回だけ残す。
  // 動いていない再描画（shift 0 かつ寸法も同じ）は捨てる。
  registerProbeSink('altscroll.reject', (_channel, fields) => {
    const shift = Number(fields.shift) || 0;
    const sameShape = fields.beforeRows === fields.afterRows && fields.beforeType === fields.afterType;
    if (shift === 0 && sameShape) return;
    post('reject', fields);
  });
  registerProbeSink('altscroll.drop', (_channel, fields) => {
    if (Number(fields.dir) < 0) dropUp++;
    else dropDown++;
    if (dropTimer === null) dropTimer = window.setTimeout(flushDrops, 500);
  });
}
