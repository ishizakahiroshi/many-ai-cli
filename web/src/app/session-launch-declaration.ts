// session-launch-declaration.ts — wrapper が起動時に申告する値（権限モード・実行モード）を
// session_update から取り込む純関数と、実行モードの見え方を決める純関数。
//
// この 2 つは register 直後の announce で 1 度だけ届き、空の値は omitempty で送られない。
// session_update の取り込みで拾い忘れると、起動した直後に画面へ載ったセッションは、再接続
// （snapshot の全件読み直し）まで値が無いまま残る。2026-11-04 には execution_mode の拾い忘れで、
// ヘッドレスで動く子に「Headless」の印が出ず、入力欄も有効のままになった。取り込む項目をここ
// 1 か所に集め、fixture で「届けば取る・届かなければ今の値を保つ」を固定する。

export interface LaunchDeclaration {
  permission_mode?: string;
  execution_mode?: string;
}

/**
 * session_update で届いた起動時の申告を、セッションの手元の記録へ取り込む。
 * 届かなかった項目（undefined）は今の値を保つ。空文字は「申告なし」として上書きする。
 */
export function mergeLaunchDeclaration(cur: LaunchDeclaration, m: LaunchDeclaration): void {
  if (m.permission_mode !== undefined) cur.permission_mode = m.permission_mode;
  if (m.execution_mode !== undefined) cur.execution_mode = m.execution_mode;
}

export type ExecutionModeKind = 'headless' | 'interactive';

/**
 * 画面に出す実行モードの種類。値は wrapper の申告が正本で、ヘッドレスのときだけ
 * "headless" が来る。空・未申告・知らない値はすべて対話として扱う（端末へ打ち込める側へ倒す）。
 */
export function executionModeKind(mode: unknown): ExecutionModeKind {
  return typeof mode === 'string' && mode.trim() === 'headless' ? 'headless' : 'interactive';
}
