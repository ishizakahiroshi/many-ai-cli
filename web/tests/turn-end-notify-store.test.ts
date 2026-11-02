import { describe, expect, test } from 'bun:test';
import {
  STOPPED_MARK_MIN_WORK_MS,
  TURN_END_MIN_RUNNING_MS,
  confirmTurnEnd,
  forgetTurnEndNotify,
  isStoppedMarkTurnInProgress,
  noteStoppedMarkSessionState,
  noteTurnStarted,
  shouldNotifyTurnEnd,
} from '../src/app/turn-end-notify-store.ts';

// 子 plan: docs/local/plan_ux-notify-palette-review_c1_notify.md 内部 C1。
// 2026-10-18 修正（docs/local/bugfix_session-stop-false-positive_2026-10-18.md、親レビュー R1〜R6）:
// 開始は「Hub が確認した開始（provider の task_started = turn_start、確定ユーザー入力 =
// user_turn_started）」、終了は「Hub が確認した終了の証拠（turn_end / done_summary の
// fallback=false）」。端子出力の running では開始しないので、再描画で新しいターンが始まった
// ことにしない（R2）。前のターンの完了が遅れて届いても、開いているターンと別の provider
// の identity なら拒否して開いたままにする（R1/R6）。ターンの identity は Hub の連番では
// なく provider 自身の turn_id なので、本物の 2 ターン目を再送と取り違えない（R4）。
//
// セッション ID は describe ブロックごとに別番号を使い、テスト間で状態が混ざらないように
// する（モジュールが Map をトップレベルで持つため）。
const iso = (ms: number) => new Date(ms).toISOString();

describe('shouldNotifyTurnEnd', () => {
  test('候補・ベルON・非表示中・設定ON のときだけ通知する', () => {
    expect(shouldNotifyTurnEnd(true, false, false, true)).toBe(true);
  });
  test('候補でなければ通知しない', () => {
    expect(shouldNotifyTurnEnd(false, false, false, true)).toBe(false);
  });
  test('そのセッションのベルが OFF なら通知しない', () => {
    expect(shouldNotifyTurnEnd(true, true, false, true)).toBe(false);
  });
  test('今見ているセッションなら通知しない', () => {
    expect(shouldNotifyTurnEnd(true, false, true, true)).toBe(false);
  });
  test('全体設定が OFF なら通知しない', () => {
    expect(shouldNotifyTurnEnd(true, false, false, false)).toBe(false);
  });
});

describe('confirmTurnEnd — 印（タブの枠付き数字とカードの黄色い●）', () => {
  test('入力で開いたターンに終了の証拠が届き、作業が閾値以上なら印を付ける', () => {
    const id = 201;
    const t0 = 11_000_000;
    expect(STOPPED_MARK_MIN_WORK_MS).toBeLessThan(TURN_END_MIN_RUNNING_MS);
    noteTurnStarted(id, { startedAt: iso(t0) });
    expect(confirmTurnEnd(id, { at: iso(t0 + STOPPED_MARK_MIN_WORK_MS) }).mark).toBe(true);
  });

  test('証拠が無ければ印も通知も付けない（端子出力の無出力だけでは断定しない）', () => {
    // 修正前の対照: 同じ無出力（running → standby）だけで黄色になり、通知も鳴り得た。
    const id = 202;
    const t0 = 12_000_000;
    noteTurnStarted(id, { startedAt: iso(t0) });
    noteStoppedMarkSessionState(id, 'standby');
    expect(confirmTurnEnd(id, undefined)).toEqual({ mark: false, notify: false });
  });

  test('fallback の記録（無出力と変更ファイルだけを根拠にした Hub の推定）は証拠にしない', () => {
    const id = 203;
    const t0 = 13_000_000;
    noteTurnStarted(id, { startedAt: iso(t0) });
    expect(confirmTurnEnd(id, { at: iso(t0 + 60_000), fallback: true })).toEqual({ mark: false, notify: false });
    // 推定を落としてもターンは開いたまま。本物の証拠では印が付く。
    expect(isStoppedMarkTurnInProgress(id)).toBe(true);
    expect(confirmTurnEnd(id, { at: iso(t0 + 61_000) }).mark).toBe(true);
  });

  test('長さを測れないセッションは証拠だけで印にし、通知は出さない', () => {
    // 画面を開いた時点で既に動いていた、Hub が開始時刻を伝えない接続等。
    expect(confirmTurnEnd(206, { at: iso(16_000_000) })).toEqual({ mark: true, notify: false });
  });

  test('同じ証拠が重ねて届いても 2 回目は印にしない', () => {
    const id = 207;
    const t0 = 17_000_000;
    noteTurnStarted(id, { startedAt: iso(t0) });
    expect(confirmTurnEnd(id, { at: iso(t0 + 6000) }).mark).toBe(true);
    expect(confirmTurnEnd(id, { at: iso(t0 + 6000) }).mark).toBe(false);
  });

  test('同じ provider ターンの再送だけを拒否する（親レビュー R4）', () => {
    const id = 219;
    const t0 = 17_500_000;
    // 同じ turn_id の再送は 1 回目だけ数える。
    expect(confirmTurnEnd(id, { at: iso(t0 + 6000), turnId: 'turn-1' }).mark).toBe(true);
    expect(confirmTurnEnd(id, { at: iso(t0 + 6000), turnId: 'turn-1' }).mark).toBe(false);
    expect(confirmTurnEnd(id, { at: iso(t0 + 9000), turnId: 'turn-1' }).mark).toBe(false);
    // 別のターンは、Hub 側の連番が同じでも通す（provider の turn_id が正本）。
    expect(confirmTurnEnd(id, { at: iso(t0 + 12_000), turnId: 'turn-2' }).mark).toBe(true);
  });

  test('同じ番号を繰り返す Hub の下でも、本物の 2 ターン目を取りこぼさない（親レビュー R4）', () => {
    // 親の実行: 新規 id・startedAt 空・at=10 秒と 20 秒の 2 つの本物の終了。修正前は
    // Hub の連番（confirmed_turn）が同じ番号のままだったため 2 回目を再送として落とした。
    // provider の turn_id が違い、かつ Hub が開始も伝えているので両方拾える。
    const id = 222;
    const t0 = 23_200_000;
    noteTurnStarted(id, { turnId: 'turn-1', startedAt: iso(t0) });
    expect(confirmTurnEnd(id, { at: iso(t0 + 10_000), turnId: 'turn-1', startedAt: iso(t0) }).mark).toBe(true);
    noteTurnStarted(id, { turnId: 'turn-2', startedAt: iso(t0 + 10_000) });
    const second = confirmTurnEnd(id, { at: iso(t0 + 20_000), turnId: 'turn-2', startedAt: iso(t0 + 10_000) });
    expect(second.mark).toBe(true);
    expect(second.startedAt).toBe(t0 + 10_000);
  });

  test('端末へ直接入力したターンでも開始が届くので、無出力の処理中に前の印を再表示しない（親レビュー R4）', () => {
    const id = 223;
    const t0 = 24_000_000;
    noteTurnStarted(id, { turnId: 'turn-1', startedAt: iso(t0) });
    expect(confirmTurnEnd(id, { at: iso(t0 + 30_000), turnId: 'turn-1', startedAt: iso(t0) }).mark).toBe(true);
    // UI 入力は無い。provider の task_started だけが次のターンの開始を伝える。
    noteStoppedMarkSessionState(id, 'standby');
    noteTurnStarted(id, { turnId: 'turn-2', startedAt: iso(t0 + 40_000) });
    expect(isStoppedMarkTurnInProgress(id)).toBe(true);
    expect(confirmTurnEnd(id, { at: iso(t0 + 60_000), fallback: true })).toEqual({ mark: false, notify: false });
    expect(isStoppedMarkTurnInProgress(id)).toBe(true);
  });

  test('再接続は Hub の開いているターンから復元する（親レビュー R5）', () => {
    const id = 224;
    const t0 = 24_500_000;
    noteTurnStarted(id, { turnId: 'turn-1', startedAt: iso(t0) });
    expect(confirmTurnEnd(id, { at: iso(t0 + 10_000), turnId: 'turn-1', startedAt: iso(t0) }).mark).toBe(true);
    noteTurnStarted(id, { turnId: 'turn-2', startedAt: iso(t0 + 20_000) });
    // ブラウザの追跡が消える（リロード・再接続）。Hub はまだ turn-2 を開いている。
    forgetTurnEndNotify(id);
    expect(isStoppedMarkTurnInProgress(id)).toBe(false);
    // 接続直後に Hub が snapshot と一緒に送る turn_start で復元する。印を消して
    // 回避するのではなく、Hub の確かな状態から戻す。
    expect(noteTurnStarted(id, { turnId: 'turn-2', startedAt: iso(t0 + 20_000) })).toBe(true);
    expect(isStoppedMarkTurnInProgress(id)).toBe(true);
    expect(confirmTurnEnd(id, { at: iso(t0 + 30_000), turnId: 'turn-2', startedAt: iso(t0 + 20_000) }).mark).toBe(true);
  });

  test('前のターンの証拠が遅れて届いても、次の実行中ターンを終了扱いにしない（親レビュー R1）', () => {
    // 親の入力順: running(t) → standby(t+10秒) → 次の入力後 running(t+20秒) →
    // 前ターンの終了証拠 startedAt=t / at=t+15秒。
    const id = 208;
    const t = 18_000_000;
    noteTurnStarted(id, { startedAt: iso(t) });
    noteStoppedMarkSessionState(id, 'running');
    noteStoppedMarkSessionState(id, 'standby');
    noteTurnStarted(id, { startedAt: iso(t + 20_000) }); // 次の入力
    expect(confirmTurnEnd(id, { at: iso(t + 15_000), startedAt: iso(t) })).toEqual({ mark: false, notify: false });
    expect(isStoppedMarkTurnInProgress(id)).toBe(true);
    // 今のターンが本当に終わった証拠では、ちゃんと閉じて印を付ける。
    expect(confirmTurnEnd(id, { at: iso(t + 26_000) }).mark).toBe(true);
    expect(isStoppedMarkTurnInProgress(id)).toBe(false);
  });

  test('前のターンの印のあと、次のターンが途中で無出力になっても新しい印は付かない', () => {
    const id = 210;
    const t0 = 20_000_000;
    noteTurnStarted(id, { startedAt: iso(t0) });
    expect(confirmTurnEnd(id, { at: iso(t0 + 8000) }).mark).toBe(true); // 前ターンの印
    noteStoppedMarkSessionState(id, 'standby');
    noteStoppedMarkSessionState(id, 'running');
    noteStoppedMarkSessionState(id, 'standby'); // 処理の途中の無出力
    // 証拠が無いので判定は呼ばれない。Hub の推定（fallback）が届いても証拠にしない。
    expect(confirmTurnEnd(id, { at: iso(t0 + 16_000), fallback: true })).toEqual({ mark: false, notify: false });
  });

  test('error / disconnected へ移ったら記憶を捨て、遅れた証拠でも印にしない', () => {
    const id = 211;
    const t0 = 21_000_000;
    noteTurnStarted(id, { startedAt: iso(t0) });
    noteStoppedMarkSessionState(id, 'disconnected');
    expect(confirmTurnEnd(id, { at: iso(t0 + 60_000) }).mark).toBe(false);
    // 次のターンの入力が来ればまた対象になる。
    noteTurnStarted(id, { startedAt: iso(t0 + 70_000) });
    expect(confirmTurnEnd(id, { at: iso(t0 + 78_000) }).mark).toBe(true);
  });

  test('forgetTurnEndNotify で印の記憶を破棄する', () => {
    const id = 212;
    const t0 = 22_000_000;
    noteTurnStarted(id, { startedAt: iso(t0) });
    forgetTurnEndNotify(id);
    // 開始時刻の記憶が消えているので、閾値未満の長さでも長さ不明として証拠だけで印にする
    // （記憶が残っていれば 5000ms 未満なので印にならない）。
    expect(confirmTurnEnd(id, { at: iso(t0 + 3000) }).mark).toBe(true);
  });

  test('isStoppedMarkTurnInProgress は入力で開いてから証拠で閉じるまでの間だけ真', () => {
    const id = 213;
    const t0 = 23_000_000;
    expect(isStoppedMarkTurnInProgress(id)).toBe(false);
    noteTurnStarted(id, { startedAt: iso(t0) });
    expect(isStoppedMarkTurnInProgress(id)).toBe(true);
    // 無出力で standby に落ちても作業中のまま（印を隠し続ける）。
    noteStoppedMarkSessionState(id, 'standby');
    expect(isStoppedMarkTurnInProgress(id)).toBe(true);
    expect(confirmTurnEnd(id, { at: iso(t0 + 8000) }).mark).toBe(true);
    expect(isStoppedMarkTurnInProgress(id)).toBe(false);
    noteTurnStarted(id, { startedAt: iso(t0 + 20_000) });
    expect(isStoppedMarkTurnInProgress(id)).toBe(true);
  });

  test('再描画で立つ running では新しいターンを開かない（親レビュー R2）', () => {
    // 親の入力順: running(t) → 終了証拠(t+30秒) → 新しい入力なしに端末再描画で
    // running(t+40秒) → standby(t+43秒)。再描画を新しい作業の開始と数えると、
    // 既存の未確認印が永久に隠れる（原計画「約3秒の再描画で印を消さない」の退行）。
    const id = 220;
    const t = 23_500_000;
    noteTurnStarted(id, { startedAt: iso(t) });
    expect(confirmTurnEnd(id, { at: iso(t + 30_000) }).mark).toBe(true);
    noteStoppedMarkSessionState(id, 'running'); // 再描画
    noteStoppedMarkSessionState(id, 'standby');
    expect(isStoppedMarkTurnInProgress(id)).toBe(false);
    // 印の記憶は残っているので、次の描画でも未確認印が出る。
    expect(confirmTurnEnd(id, { at: iso(t + 30_000) }).mark).toBe(false); // 同じ証拠は数え直さない
  });

  test('at が無い・読めない記録は証拠にしない（受信時刻で補わない）', () => {
    const id = 215;
    const t0 = 25_000_000;
    noteTurnStarted(id, { startedAt: iso(t0) });
    // `Date.parse` が解釈できない値だけを並べる（'0' のような緩い表記は実装依存で
    // 日付として通る。Hub は常に RFC3339 を送るのでここでは扱わない）。
    for (const at of [undefined, '', 'not-a-time', 'n/a']) {
      expect(confirmTurnEnd(id, { at })).toEqual({ mark: false, notify: false });
      expect(isStoppedMarkTurnInProgress(id)).toBe(true); // 記憶も変えない
    }
    expect(confirmTurnEnd(id, { at: iso(t0 + 6000) }).mark).toBe(true);
  });

  test('turn_end の started_at から長さを測る（Hub が開始を記録していれば）', () => {
    const id = 216, short = 217;
    const t0 = 26_000_000;
    expect(confirmTurnEnd(id, { at: iso(t0 + 30_000), startedAt: iso(t0) }).mark).toBe(true);
    // 2 秒で終わったターンは印にしない。
    expect(confirmTurnEnd(short, { at: iso(t0 + 2000), startedAt: iso(t0) }).mark).toBe(false);
  });

  test('Hub が確認した開始（入力の時刻）は証拠の started_at より優先する', () => {
    const id = 218;
    const t0 = 27_000_000;
    noteTurnStarted(id, { startedAt: iso(t0 + 10_000) });
    // 証拠の started_at からは 30 秒だが、Hub が確認した開始からは 2 秒しかない。
    expect(confirmTurnEnd(id, { at: iso(t0 + 12_000), startedAt: iso(t0) }).mark).toBe(false);
  });

  test('Hub が開始時刻を伝えない接続でも、入力でターンは開く（印は出る）', () => {
    const id = 221;
    const t0 = 28_000_000;
    expect(noteTurnStarted(id, {})).toBe(true);
    expect(isStoppedMarkTurnInProgress(id)).toBe(true);
    expect(confirmTurnEnd(id, { at: iso(t0 + 60_000) })).toEqual({ mark: true, notify: false });
  });
});

describe('confirmTurnEnd — 作業終了の通知', () => {
  test('ターンが通知の下限以上続いたら通知候補にする', () => {
    const id = 301;
    const t0 = 31_000_000;
    noteTurnStarted(id, { startedAt: iso(t0) });
    const decision = confirmTurnEnd(id, { at: iso(t0 + TURN_END_MIN_RUNNING_MS) });
    expect(decision.mark).toBe(true);
    expect(decision.notify).toBe(true);
    expect(decision.startedAt).toBe(t0);
  });

  test('通知の下限未満（印の下限以上）なら印だけ付けて通知しない', () => {
    const id = 302;
    const t0 = 32_000_000;
    noteTurnStarted(id, { startedAt: iso(t0) });
    const decision = confirmTurnEnd(id, { at: iso(t0 + TURN_END_MIN_RUNNING_MS - 1) });
    expect(decision).toEqual({ mark: true, notify: false, startedAt: t0 });
  });

  test('turn_end でも、長さが足りていれば通知候補にする', () => {
    const id = 303;
    const t0 = 33_000_000;
    const decision = confirmTurnEnd(id, { at: iso(t0 + 45_000), startedAt: iso(t0), turnId: 'turn-2' });
    expect(decision).toEqual({ mark: true, notify: true, startedAt: t0 });
  });
});
