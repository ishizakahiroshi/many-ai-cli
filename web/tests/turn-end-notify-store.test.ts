import { describe, expect, test } from 'bun:test';
import {
  STOPPED_MARK_MIN_WORK_MS,
  TURN_END_MIN_RUNNING_MS,
  forgetTurnEndNotify,
  observeStoppedMarkCandidate,
  observeTurnEndCandidate,
  shouldNotifyTurnEnd,
  turnEndRunningStartedAt,
} from '../src/app/turn-end-notify-store.ts';

// 子 plan: docs/local/plan_ux-notify-palette-review_c1_notify.md 内部 C1。
//
// 固定したいのは「running → standby」という遷移そのものを 1 回だけ捉えられること。
// Hub は同じ状態を繰り返し送ってくることがあるため、値の一致ではなく遷移の有無で
// 判定できているかを見る。セッション ID は describe ブロックごとに別番号を使い、
// テスト間で状態が混ざらないようにする（モジュールが Map をトップレベルで持つため）。

describe('observeTurnEndCandidate', () => {
  test('running が閾値以上続いてから standby に戻ると候補になる', () => {
    const id = 1;
    const t0 = 1_000_000;
    expect(observeTurnEndCandidate(id, 'standby', 'running', t0)).toBe(false);
    expect(observeTurnEndCandidate(id, 'running', 'standby', t0 + TURN_END_MIN_RUNNING_MS)).toBe(true);
  });

  test('running が閾値未満で standby に戻ると候補にならない', () => {
    const id = 2;
    const t0 = 2_000_000;
    observeTurnEndCandidate(id, 'standby', 'running', t0);
    expect(observeTurnEndCandidate(id, 'running', 'standby', t0 + TURN_END_MIN_RUNNING_MS - 1)).toBe(false);
  });

  test('waiting への遷移は候補にならない', () => {
    const id = 3;
    const t0 = 3_000_000;
    observeTurnEndCandidate(id, 'standby', 'running', t0);
    // 承認待ちは既存の承認通知の対象であって、作業終了通知の対象ではない。
    expect(observeTurnEndCandidate(id, 'running', 'waiting', t0 + TURN_END_MIN_RUNNING_MS)).toBe(false);
  });

  test('同じ running→standby の遷移が重ねて届いても2回目は候補にならない', () => {
    const id = 4;
    const t0 = 4_000_000;
    observeTurnEndCandidate(id, 'standby', 'running', t0);
    expect(observeTurnEndCandidate(id, 'running', 'standby', t0 + TURN_END_MIN_RUNNING_MS)).toBe(true);
    // Hub が同じ状態を重ねて送ってきたケースを模して、同じ引数で再度呼ぶ。
    expect(observeTurnEndCandidate(id, 'running', 'standby', t0 + TURN_END_MIN_RUNNING_MS + 5000)).toBe(false);
  });

  test('再び running になってから standby に戻ると再度候補になる', () => {
    const id = 5;
    const t0 = 5_000_000;
    observeTurnEndCandidate(id, 'standby', 'running', t0);
    expect(observeTurnEndCandidate(id, 'running', 'standby', t0 + TURN_END_MIN_RUNNING_MS)).toBe(true);
    const t1 = t0 + TURN_END_MIN_RUNNING_MS + 1000;
    observeTurnEndCandidate(id, 'standby', 'running', t1);
    expect(observeTurnEndCandidate(id, 'running', 'standby', t1 + TURN_END_MIN_RUNNING_MS)).toBe(true);
  });

  test('running を経ずに standby へ来た未知のセッションは候補にならない', () => {
    expect(observeTurnEndCandidate(999, undefined, 'standby', 6_000_000)).toBe(false);
  });

  test('forgetTurnEndNotify で記憶を破棄すると、その後の standby は候補にならない', () => {
    const id = 6;
    const t0 = 7_000_000;
    observeTurnEndCandidate(id, 'standby', 'running', t0);
    forgetTurnEndNotify(id);
    expect(observeTurnEndCandidate(id, 'running', 'standby', t0 + TURN_END_MIN_RUNNING_MS)).toBe(false);
  });
});

describe('turnEndRunningStartedAt', () => {
  test('running へ入った時刻を保持し、候補判定後も読み出せる', () => {
    const id = 7;
    const t0 = 8_000_000;
    expect(turnEndRunningStartedAt(id)).toBeUndefined();
    observeTurnEndCandidate(id, 'standby', 'running', t0);
    expect(turnEndRunningStartedAt(id)).toBe(t0);
    observeTurnEndCandidate(id, 'running', 'standby', t0 + TURN_END_MIN_RUNNING_MS);
    expect(turnEndRunningStartedAt(id)).toBe(t0);
  });

  test('forgetTurnEndNotify の後は undefined に戻る', () => {
    const id = 8;
    observeTurnEndCandidate(id, 'standby', 'running', 9_000_000);
    forgetTurnEndNotify(id);
    expect(turnEndRunningStartedAt(id)).toBeUndefined();
  });
});

describe('shouldNotifyTurnEnd', () => {
  test('すべての条件を満たすときだけ真', () => {
    expect(shouldNotifyTurnEnd(true, false, false, true)).toBe(true);
  });

  test('候補でなければ偽', () => {
    expect(shouldNotifyTurnEnd(false, false, false, true)).toBe(false);
  });

  test('ベルが OFF なら偽', () => {
    expect(shouldNotifyTurnEnd(true, true, false, true)).toBe(false);
  });

  test('今見ているセッションなら偽', () => {
    expect(shouldNotifyTurnEnd(true, false, true, true)).toBe(false);
  });

  test('「作業が終わったときも通知する」設定が OFF なら偽', () => {
    expect(shouldNotifyTurnEnd(true, false, false, false)).toBe(false);
  });
});

// タブの数字とカードの黄色い●（止まった後まだ開いていない）の合図。通知より短い作業も
// 拾い、再描画だけの running（Hub の idle_after による約 3 秒）は拾わない。
describe('observeStoppedMarkCandidate', () => {
  test('通知の閾値に満たない作業でも、印の閾値以上なら候補になる', () => {
    const id = 101;
    const t0 = 11_000_000;
    expect(STOPPED_MARK_MIN_WORK_MS).toBeLessThan(TURN_END_MIN_RUNNING_MS);
    expect(observeStoppedMarkCandidate(id, 'standby', 'running', t0)).toBe(false);
    expect(observeStoppedMarkCandidate(id, 'running', 'standby', t0 + STOPPED_MARK_MIN_WORK_MS)).toBe(true);
  });

  test('再描画だけの約 3 秒の running は候補にならない', () => {
    const id = 102;
    const t0 = 12_000_000;
    observeStoppedMarkCandidate(id, 'standby', 'running', t0);
    expect(observeStoppedMarkCandidate(id, 'running', 'standby', t0 + 3200)).toBe(false);
  });

  test('承認待ちを挟んだ作業は、最初に動き出した時刻から測る', () => {
    const id = 103;
    const t0 = 13_000_000;
    observeStoppedMarkCandidate(id, 'standby', 'running', t0);
    expect(observeStoppedMarkCandidate(id, 'running', 'waiting', t0 + 2000)).toBe(false);
    observeStoppedMarkCandidate(id, 'waiting', 'running', t0 + 4000);
    expect(observeStoppedMarkCandidate(id, 'running', 'standby', t0 + 5000)).toBe(true);
  });

  test('同じ standby が重ねて届いても 2 回目は候補にならない', () => {
    const id = 104;
    const t0 = 14_000_000;
    observeStoppedMarkCandidate(id, 'standby', 'running', t0);
    expect(observeStoppedMarkCandidate(id, 'running', 'standby', t0 + STOPPED_MARK_MIN_WORK_MS)).toBe(true);
    expect(observeStoppedMarkCandidate(id, 'standby', 'standby', t0 + STOPPED_MARK_MIN_WORK_MS + 1000)).toBe(false);
  });

  test('画面を開いた時点で既に動いていたセッションは、長さを測れないので候補にする', () => {
    expect(observeStoppedMarkCandidate(105, 'running', 'standby', 15_000_000)).toBe(true);
  });

  test('running を経ずに standby へ来たセッションは候補にならない', () => {
    expect(observeStoppedMarkCandidate(106, undefined, 'standby', 16_000_000)).toBe(false);
    expect(observeStoppedMarkCandidate(106, 'waiting', 'standby', 16_000_100)).toBe(false);
  });

  test('error / disconnected へ移ったら作業の記録を捨てる', () => {
    const id = 107;
    const t0 = 17_000_000;
    observeStoppedMarkCandidate(id, 'standby', 'running', t0);
    expect(observeStoppedMarkCandidate(id, 'running', 'disconnected', t0 + STOPPED_MARK_MIN_WORK_MS)).toBe(false);
    expect(observeStoppedMarkCandidate(id, 'disconnected', 'standby', t0 + STOPPED_MARK_MIN_WORK_MS + 1)).toBe(false);
  });

  test('forgetTurnEndNotify で印の作業記録も破棄する', () => {
    const id = 108;
    const t0 = 18_000_000;
    observeStoppedMarkCandidate(id, 'standby', 'running', t0);
    forgetTurnEndNotify(id);
    // 開始時刻が無く、直前も running ではない扱いになる経路を確かめるため waiting から戻す。
    expect(observeStoppedMarkCandidate(id, 'waiting', 'standby', t0 + STOPPED_MARK_MIN_WORK_MS)).toBe(false);
  });
});
