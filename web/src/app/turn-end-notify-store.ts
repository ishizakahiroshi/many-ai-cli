// turn-end-notify-store.ts — セッションが running から standby へ戻った瞬間を検知し、
// 「作業終了の通知を出してよいか」を決める、DOM と window に一切触れない純関数群。
//
// 合図は状態の値そのものではなく遷移。Hub は同じ状態を繰り返し送ってくることがある
// （session_update は差分更新のたびに現在の state を含む）ため、「running から抜けた
// 瞬間」だけを 1 回捉える必要がある。
//
// 子 plan: docs/local/plan_ux-notify-palette-review_c1_notify.md 内部 C1。
// 呼び出し側の配線・実際の通知・設定・ベルの UI は同じ子 plan の C2。

/**
 * 短い往復（入力のエコーや一瞬の応答）を通知対象から落とす下限。running がこの時間
 * 以上続いてから standby に戻ったときだけ通知候補にする。設定画面には出さない
 * （子 plan の判断ログ: 目視で合わなければここを直す）。
 */
export const TURN_END_MIN_RUNNING_MS = 20000;

interface RunningWindow {
  /** このセッションが直近に running へ入った時刻（epoch ms）。 */
  since: number;
  /** この running 区間について、既に通知候補を 1 度返したか。 */
  consumed: boolean;
}

const runningWindows = new Map<number, RunningWindow>();

/**
 * セッションの状態遷移を観測し、「作業終了の通知候補か」を返す。
 *
 * - `nextState` が `running` で、直前が `running` でなかった場合は開始時刻を記録する
 *   だけで、それ自体は候補にしない（区間の始まり）
 * - `running` → `standby` のときだけ判定する。区間の長さが
 *   {@link TURN_END_MIN_RUNNING_MS} 未満なら候補にしない
 * - `waiting` への遷移（承認待ち）は対象外。既存の承認通知が別に出るため、ここでは
 *   何も記録も判定もしない
 * - 同じ running 区間について 1 度候補を返したら、次にその セッションが running へ
 *   入り直すまでは再度候補にしない（Hub が同じ状態を重ねて送ってきても多重に通知
 *   しないため）
 *
 * isNew（新規登録）のセッションや snapshot 一括適用では、直前の状態が実測できない
 * ため呼び出し側はこの関数を呼ばない設計にする（C2 側の配線）。
 */
export function observeTurnEndCandidate(
  id: number,
  prevState: string | undefined,
  nextState: string | undefined,
  now: number,
): boolean {
  if (nextState === 'running') {
    if (prevState !== 'running') {
      runningWindows.set(id, { since: now, consumed: false });
    }
    return false;
  }
  if (prevState !== 'running' || nextState !== 'standby') return false;
  const win = runningWindows.get(id);
  if (!win || win.consumed) return false;
  if (now - win.since < TURN_END_MIN_RUNNING_MS) return false;
  win.consumed = true;
  return true;
}

/**
 * このセッションが直近に running へ入った時刻（epoch ms）。記録が無ければ
 * `undefined`。通知本文に完了サマリーを使ってよいか（今回の running 開始より
 * 新しいか）を C2 側で判定するために使う。
 */
export function turnEndRunningStartedAt(id: number): number | undefined {
  return runningWindows.get(id)?.since;
}

/** Hub 再起動でセッション ID が振り直されたときに、そのセッションの記憶を破棄する。 */
export function forgetTurnEndNotify(id: number): void {
  runningWindows.delete(id);
}

/**
 * 通知候補を、実際に出してよいかへ絞り込む最終判定。
 *
 * - `isCandidate`: {@link observeTurnEndCandidate} が真を返した遷移か
 * - `bellOff`: このセッションのベル（個別の作業終了通知）が OFF か
 * - `isCurrentlyViewedSession`: 表示中タブで今まさに見ているセッションか
 *   （既存の承認通知 `showDesktopApprovalNotification` と同じ判定を渡す想定）
 * - `settingEnabled`: 「作業が終わったときも通知する」の全体設定が ON か
 *
 * 音・OS 通知それぞれの個別設定（notify-sound-enabled / desktop-notify-enabled）
 * はここでは見ない。それらは実際に鳴らす・出す関数側（settings.ts）が別途見る。
 */
export function shouldNotifyTurnEnd(
  isCandidate: boolean,
  bellOff: boolean,
  isCurrentlyViewedSession: boolean,
  settingEnabled: boolean,
): boolean {
  return isCandidate && !bellOff && !isCurrentlyViewedSession && settingEnabled;
}
