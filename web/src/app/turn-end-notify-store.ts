// turn-end-notify-store.ts — Hub が確認した「ターンの開始」と「ターンの終了」の対応から、
// 「作業を終えて止まった」印（タブの枠付き数字とカードの黄色い●）と作業終了の通知を
// 出してよいかを決める、DOM と window に一切触れない純関数群。
//
// 子 plan: docs/local/plan_ux-notify-palette-review_c1_notify.md 内部 C1。
// 呼び出し側の配線・実際の通知・設定・ベルの UI は同じ子 plan の C2。
//
// 2026-10-18 修正（docs/local/bugfix_session-stop-false-positive_2026-10-18.md）:
// 以前は「端子出力の running」を開始の根拠にしていた。しかし standby は Hub が
// 「端末出力が 3 秒途切れた」だけで作る値（rust/src/terminal/session.rs の idle_after →
// proto/core.rs の display_state）なので、長いツール待ちの途中でも「止まった」に見え、
// さらに画面の再描画で running になると新しいターンが始まったように見えた（親レビュー
// R2 の退行）。今は開始も終了も Hub が確認した出来事だけから決める。
//
// ターンの開始（呼び出し側 = ws-client.ts が渡す）:
//   - `turn_start`（Additive Rust-only 通知。Codex の provider トランスクリプトが書いた
//     task_started の確定点。端末に直接入力したターンでも届き、`turn_id` は provider
//     自身のターン identity）。UI へ接続した直後は、Hub が今開いているターンについて
//     これを送るので、リロード・再接続でも Hub の状態から復元できる（親レビュー R4/R5）。
//   - `user_turn_started`（rust/src/terminal/session/input.rs が確定ユーザー入力の送達時に
//     broadcast する。`started_at` は Hub が記録したそのターンの開始時刻）。リプレイや
//     再描画では出ないので、再描画を新しいターンの開始と誤らない。identity は持たない。
//
// ターンの終了（証拠）:
//   - `turn_end`（Additive Rust-only 通知。Codex の provider トランスクリプトが書いた
//     task_complete の確定点で、Git の差分計測や DONE カードと無関係に Hub が出す。
//     `turn_id` は provider 自身のターン identity、`started_at` は同じ `turn_id` の
//     `task_started` を Hub が確認できたときだけ入る。親レビュー R3/R4/R6）
//   - `done_summary`（rust/src/terminal/session/completion.rs の publish_completion を
//     通る経路。DONE マーカー・Codex の task_complete + Git 変更あり・Routine 完了は
//     fallback=false、非 Codex の無出力フォールバックは fallback=true）
// `fallback=true` は「無出力 + 変更ファイル」から Hub が作った推定そのものなので、終了の
// 証拠にしない。使うと、ファイルを編集して黙っただけの作業途中で元の不具合が再発する。

/**
 * 短い往復（入力のエコーや一瞬の応答）を通知対象から落とす下限。ターン（Hub が確認した
 * 開始から終了まで）がこの時間以上続いたときだけ通知候補にする。設定画面には出さない
 * （子 plan の判断ログ: 目視で合わなければここを直す）。
 */
export const TURN_END_MIN_RUNNING_MS = 20000;

/**
 * 通知候補を、実際に出してよいかへ絞り込む最終判定。
 *
 * - `isCandidate`: {@link confirmTurnEnd} の `notify` が真だった証拠か
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

// ---- 「作業を終えて止まった」印（サイドバーのタブの数字とカードの黄色い●） ----
//
// 印は音や OS 通知と違って目に入るだけなので、通知より短い作業も拾う。拾い損ねると、
// 開いていないのにグレーのままのカードができ、「黄色でなければ見た」と読めなくなる
// （2026-10-16 利用者要望: 止まったセッションが 3 つあると、どれを見に行けばよいか
// 分からない）。
//
// ただし、終了の証拠が無いまま端子出力だけで印を付けると、処理中のセッションが黄色に
// なる（元の不具合）。証拠が無いターンは印を付けない。

/**
 * 作業とみなす長さの下限。Hub は出力が 3 秒途切れるまで running のままにする
 * （rust/src/terminal/session.rs の idle_after）ため、画面の大きさを変えたときの
 * 再描画のような一瞬の出力でも running が約 3 秒立つ。それを落とす値にする。
 */
export const STOPPED_MARK_MIN_WORK_MS = 5000;

/** 入力で開いて、まだ終了を確認していないターンを持つセッション。 */
const openTurns = new Set<number>();
/** 開いているターンの開始時刻（epoch ms）。Hub が時刻を伝えない接続では入らない。 */
const openTurnStartedAt = new Map<number, number>();
/** 開いているターンの provider の identity（Hub の `turn_start.turn_id`）。 */
const openTurnIds = new Map<number, string>();
/** 直近に確認した終了時刻（epoch ms）。同じ証拠の数え直しを防ぐ。 */
const lastTurnEndedAt = new Map<number, number>();
/** 終了を確認済みの provider ターン。同じターンの再送だけを弾く（親レビュー R4）。 */
const closedTurnIds = new Map<number, Set<string>>();
/** セッションが終わったと分かる状態へ移ったセッション。次のターン開始まで印にしない。 */
const endedSessions = new Set<number>();
/** セッションが終わった状態（rust/src/terminal/session.rs の terminal() と同じ語）。 */
const TERMINAL_STATES = new Set(['completed', 'error', 'disconnected', 'done', 'timeout', 'dismissed']);

/**
 * Hub が確認したターンの開始のうち、判定に要る部分だけ。
 * `turn_start` / `user_turn_started` をそのまま渡してよい（余分なフィールドは見ない）。
 */
export interface TurnStartEvidenceLike {
  /**
   * provider 自身のターン identity（`turn_start.turn_id`）。確定ユーザー入力の
   * `user_turn_started` は持たない（Hub は provider の番号を知らないため）。
   */
  turnId?: string;
  /**
   * Hub が記録したそのターンの開始時刻（RFC3339）。Go 版 Hub のように伝えない
   * 接続では空。空でも「ターンが開いた」ことは記録する（作業中として印を隠すため）。
   */
  startedAt?: string;
}

/**
 * ターンの開始（Hub の `turn_start` = provider の `task_started`、または Hub が確認した
 * ユーザー入力の `user_turn_started`）でターンを開く。端子出力の running では開かない —
 * 再描画やリプレイで新しいターンが始まったことにしない（親レビュー R2）。provider の
 * `turn_start` は端末に直接入力したターンでも届くので、そのターンが無出力のまま進んで
 * いても、前のターンの未閲覧印を再表示しない（親レビュー R4）。
 *
 * 戻り値は「このセッションが作業中でなかったのに作業中になったか」。呼び出し側は真の
 * とき描き直して、未確認印を隠す。
 */
export function noteTurnStarted(id: number, evidence: TurnStartEvidenceLike): boolean {
  endedSessions.delete(id);
  const wasOpen = openTurns.has(id);
  openTurns.add(id);
  const turnId = normalizeTurnId(evidence?.turnId);
  if (turnId === undefined) {
    // identity を伝えない開始（確定ユーザー入力）では、前のターンの identity を
    // 引き継がない。続く provider の `turn_start` が同じターンの identity を入れる。
    openTurnIds.delete(id);
  } else {
    openTurnIds.set(id, turnId);
  }
  const startedAt = parseTime(evidence?.startedAt);
  if (startedAt === undefined) {
    // 開始時刻を伝えない接続では、前のターンの開始を引き継がないほうが安全。
    openTurnStartedAt.delete(id);
  } else {
    openTurnStartedAt.set(id, startedAt);
  }
  return !wasOpen;
}

/**
 * セッションの状態遷移のうち、セッションが終わったと分かる状態だけを見る。running は
 * 見ない（開始の根拠にしない）。終わったセッションへ遅れて届いた証拠で印を付けないため。
 */
export function noteStoppedMarkSessionState(id: number, state: string | undefined): void {
  if (state !== undefined && TERMINAL_STATES.has(state)) {
    openTurns.delete(id);
    openTurnStartedAt.delete(id);
    openTurnIds.delete(id);
    endedSessions.add(id);
  }
}

/**
 * Hub がターン終了を確認した証拠のうち、判定に要る部分だけ。
 * `done_summary` / `turn_end` をそのまま渡してよい（余分なフィールドは見ない）。
 */
export interface TurnEndEvidenceLike {
  /** Hub が記録したターン終了時刻（RFC3339）。無い記録は証拠として使わない。 */
  at?: string;
  /** Hub が「無出力 + 変更ファイル」から作った推定の記録か（`DoneSummary.fallback`）。 */
  fallback?: boolean;
  /** Hub が確認したこのターンの開始時刻（`turn_end` の `started_at`）。 */
  startedAt?: string;
  /** provider 自身のターン identity（`turn_end` の `turn_id`）。同じターンの再送だけを弾く。 */
  turnId?: string;
}

/** {@link confirmTurnEnd} の答え。 */
export interface TurnEndDecision {
  /** 未閲覧のセッションに「止まった」印を付けてよいか。 */
  mark: boolean;
  /** 作業終了の通知を出してよいか（音・OS 通知の個別設定は呼び出し側が見る）。 */
  notify: boolean;
  /** 長さを測るのに使った開始時刻（epoch ms）。通知本文の完了サマリー判定へ渡す。 */
  startedAt?: number;
}

const NO_TURN_END: TurnEndDecision = { mark: false, notify: false };

function parseTime(value: string | undefined): number | undefined {
  const parsed = Date.parse(String(value ?? ''));
  return Number.isFinite(parsed) && parsed > 0 ? parsed : undefined;
}

/** 空の identity は「伝えていない」として扱う（Go 版 Hub や古い経路）。 */
function normalizeTurnId(value: string | undefined): string | undefined {
  return typeof value === 'string' && value !== '' ? value : undefined;
}

/** 終了を確認済みの identity を覚える。古いものから捨てて増え続けないようにする。 */
function rememberClosedTurnId(id: number, turnId: string): void {
  let ids = closedTurnIds.get(id);
  if (!ids) {
    ids = new Set();
    closedTurnIds.set(id, ids);
  }
  ids.add(turnId);
  while (ids.size > 8) {
    const oldest = ids.values().next().value;
    if (oldest === undefined) break;
    ids.delete(oldest);
  }
}

/**
 * Hub が確認したターン終了の証拠から、印と通知を出してよいかを決める。呼び出し側は
 * 未閲覧のセッションのときだけ印を付け（isSessionCurrentlyViewed の判定は settings.ts に
 * ある）、通知は {@link shouldNotifyTurnEnd} へ渡す。
 *
 * - `fallback` が真の記録は証拠にしない。「無出力 + 変更ファイル」から Hub が作った推定で、
 *   作業途中の無出力でも作られる（schedule_fallback）
 * - `at` が無い（または読めない）記録は証拠にしない。時刻の分からない記録を受信時刻で
 *   補って確定扱いにすると、証拠不足の推定を本物としてしまう
 * - ターンの長さは、開いているターンの開始時刻（Hub が確認した開始の時刻）、無ければ
 *   証拠の `startedAt` で測る。長さが {@link STOPPED_MARK_MIN_WORK_MS} 以上なら印、
 *   {@link TURN_END_MIN_RUNNING_MS} 以上なら通知。どちらも測れないセッションは、
 *   ターン終了という確かな証拠があるので印だけ付ける（音や OS 通知は出さない）
 * - 開いているターンと違う provider の identity の証拠は拒否する。前のターンの完了が
 *   遅れて届いても、今のターンを終了扱いにしない（親レビュー R1/R6）。**このとき開いて
 *   いるターンは閉じない** — 閉じると、作業中の印が消えて古い未閲覧印が再表示される
 * - 開いているターンの開始より古い証拠も拒否する（同じく R1）。閉じない
 * - 同じ identity のターンをもう一度数えない（`closedTurnIds`）。identity を持たない
 *   証拠は、時刻が同じか古いものを数え直さない（`lastTurnEndedAt`）
 * - セッションが終わったと分かる状態のセッション（{@link noteStoppedMarkSessionState} が
 *   記録を捨てた後）は拒否する
 */
export function confirmTurnEnd(
  id: number,
  evidence: TurnEndEvidenceLike | undefined | null,
): TurnEndDecision {
  if (!evidence || evidence.fallback || endedSessions.has(id)) return NO_TURN_END;
  const ended = parseTime(evidence.at);
  if (ended === undefined) return NO_TURN_END;
  const turnId = normalizeTurnId(evidence.turnId);
  if (turnId !== undefined && closedTurnIds.get(id)?.has(turnId)) return NO_TURN_END;
  const opened = openTurns.has(id);
  const openTurnId = openTurnIds.get(id);
  // A late end of the previous turn carries the previous turn's identity. It is
  // not evidence about the turn that is open now, and it must not close it.
  if (opened && turnId !== undefined && openTurnId !== undefined && turnId !== openTurnId) {
    return NO_TURN_END;
  }
  const previous = lastTurnEndedAt.get(id);
  if (previous !== undefined && ended <= previous) return NO_TURN_END;
  const since = openTurnStartedAt.get(id) ?? parseTime(evidence.startedAt);
  if (opened && since !== undefined && ended < since) return NO_TURN_END;
  openTurns.delete(id);
  openTurnStartedAt.delete(id);
  openTurnIds.delete(id);
  lastTurnEndedAt.set(id, ended);
  if (turnId !== undefined) rememberClosedTurnId(id, turnId);
  if (since === undefined) return { mark: true, notify: false };
  const worked = ended - since;
  return {
    mark: worked >= STOPPED_MARK_MIN_WORK_MS,
    notify: worked >= TURN_END_MIN_RUNNING_MS,
    startedAt: since,
  };
}

/**
 * このセッションで、まだ終了を確認していないターンが開いているか。Hub が確認した入力
 * （{@link noteTurnStarted}）でだけ開くので、端子出力の running や再描画では変わらない。
 * 「作業中」かどうかを印の表示判定（session-list-tab-state.ts）へ渡す。
 */
export function isStoppedMarkTurnInProgress(id: number): boolean {
  return openTurns.has(id);
}

/** Hub 再起動やセッション終了で、そのセッション ID の記憶を破棄する。 */
export function forgetTurnEndNotify(id: number): void {
  openTurns.delete(id);
  openTurnStartedAt.delete(id);
  openTurnIds.delete(id);
  lastTurnEndedAt.delete(id);
  closedTurnIds.delete(id);
  endedSessions.delete(id);
}
