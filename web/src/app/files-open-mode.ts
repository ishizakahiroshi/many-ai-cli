// Files タブを押したときの開き方の判定。
//
// Files を全画面で開くと端末（CLI）が隠れて、ファイルを見ながら CLI に指示を出せない。
// 端末を見ているときは、端末を隠さずその隣へ分割ペインとして開く（境界線をドラッグして幅を変えられる）。
// 分割ペインの置き方そのもの（beginAutoSplit・境界線）は multi-pane.ts / flexible-pane-views.ts が持つ。
// ここは「押したタブを、今、隣へ置くか」を決めるだけで、DOM も通信も触らない。

/** 分割ペインに置いた Files の content 要素に付ける印。ツリーだけを出し、選んだファイルはモーダルで開く。 */
export const FILES_PANE_CLASS = 'files-tab-content--pane';

/** この幅以下では分割ペインを使わない（pane-placement-menu.ts の「狭い画面」と同じ基準）。 */
export const FILES_BESIDE_MIN_WIDTH_QUERY = '(max-width: 1000px)';

export type FilesOpenSurface = {
  /** 開く先の生きているセッションがある */
  hasSession: boolean;
  /** 狭い画面（分割ペインを並べられない幅） */
  compact: boolean;
  /** 別窓のタブ固定表示（表示するタブが別窓ごとに決まっている） */
  detached: boolean;
  /** #display-area が見えている */
  displayVisible: boolean;
  /** #display-area の mode-*（terminal / chat / files / git …） */
  displayMode: string | null;
  /** #multi-view が見えている */
  multiVisible: boolean;
  /** #multi-view の paneMode（overview = 全セッション一覧 / workspace = このセッションの分割） */
  multiPaneMode: string | null;
};

/**
 * Files タブを端末の隣へ開くなら true。
 * 端末だけを見ているとき、またはこのセッションの分割ペインを見ているときだけ。
 * チャット・Git・全セッション一覧などを見ているときは、従来どおりタブを切り替える
 * （ここで分割すると、見ていた画面と端末と Files の 3 枚に増えてしまう）。
 */
export function shouldOpenFilesBeside(surface: FilesOpenSurface): boolean {
  if (!surface.hasSession || surface.compact || surface.detached) return false;
  if (surface.displayVisible) return surface.displayMode === 'terminal';
  if (surface.multiVisible) return surface.multiPaneMode === 'workspace';
  return false;
}
