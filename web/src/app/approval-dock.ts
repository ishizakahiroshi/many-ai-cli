// 承認バー（#action-bar）の下端をどこへ置くか。
//
// バーは #display-stack の下端へ absolute で重ねている（フローに置くと表示・伸縮のたび
// #terminal-area が縮んで pty_resize が飛び、同じ本文が scrollback へ積み上がる。
// 経緯は styles/approval.css 冒頭）。ただし #display-stack の下端はターミナル本文の
// 最下部そのものなので、CLI の最新行がバーの裏へ隠れる。承認のたびにバーを縮めて
// 下を覗く操作が要るのはこれが理由。
//
// デスクトップは入力欄・添付欄の帯（#input-bar-outer）に高さの余裕があるので、バーを
// #display-stack の外側までそのぶん下げ、ターミナル本文を一切覆わないようにする。
// 下げ幅は入力欄帯の実測（token-statusbar 用の padding は除く＝ステータスバーは覆わない）。
// モバイルは approval.css 側で position:relative に戻るため、この変数は効かない。

const SHIFT_VAR = '--approval-dock-shift';

export function initApprovalDock(): void {
  const stack = document.getElementById('display-stack');
  const outer = document.getElementById('input-bar-outer');
  if (!stack || !outer) return;

  let rafId = 0;
  const update = (): void => {
    rafId = 0;
    // 入力欄帯の下端から token-statusbar 用の padding を引いた位置がバー下端の狙い。
    const pad = parseFloat(getComputedStyle(outer).paddingBottom) || 0;
    const targetBottom = outer.getBoundingClientRect().bottom - pad;
    const shift = Math.max(0, Math.round(targetBottom - stack.getBoundingClientRect().bottom));
    document.documentElement.style.setProperty(SHIFT_VAR, shift + 'px');
  };
  const schedule = (): void => {
    if (rafId) return;
    rafId = requestAnimationFrame(update);
  };

  // 入力欄の伸縮・添付チップ・ライブ状態帯・各種バナーの出入りで帯の高さが変わる。
  // #display-stack は flex:1 なのでバナー 1 本の増減でも自分のサイズが変わり、
  // 2 要素を observe しておけば下げ幅の再計算が漏れない。
  if (typeof ResizeObserver === 'function') {
    const ro = new ResizeObserver(schedule);
    ro.observe(stack);
    ro.observe(outer);
  }
  window.addEventListener('resize', schedule);
  update();
}
