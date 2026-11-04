// pane-state-badge.ts — ペイン見出しの状態記号（通常のマルチ・別窓 Grid 共通）。
//
// 形・意味・色は、サイドバーのカード・セッション帯・ライブ帯と同じものを使う。
// 「どの記号を出すか」は session-list.ts の stateActivityDecoration、「その形」は
// stateIconSvgHtml が正本で、ここは呼ぶだけ。見出しだけが文字の ⚠ ● — を自前で選び、
// state の 3 値だけを見て描くと、同じセッションがカードと見出しで違う状態に見える
// （入力待ち・起動中・終わって未読が見出しに出ず、承認待ちはカードが旗・見出しが ⚠ になる）。
//
// 色と動き（実行中の回転・承認待ちの点滅）も新しく書かない。カードのピルと同じクラス
// （card-state-pill / card-state-icon）を付けて styles.css の既存の規則を共有する。
// ph-badge は見出し内で記号の位置を探すための名前として残す。

import { safeClassToken, stateActivityDecoration, stateIconSvgHtml, stateLabel } from './session-list.js';

/**
 * 見出しの記号を、セッションの今の状態に合わせて描き直す。戻り値は「承認待ちとして
 * 強調するか」（スロットの枠線用）。
 *
 * 呼ばれる回数が多い（状態が変わるたび・ペインの数だけ）ので、変わっていないときは
 * DOM に触らない。SVG を毎回作り直すと、実行中の回転が先頭へ巻き戻って止まって見える。
 */
export function applyPaneStateBadge(badge: HTMLElement, session: any): boolean {
  if (!session) return false;
  const state = session.state || 'standby';
  const activity = stateActivityDecoration(session);
  const className = `ph-badge card-state-pill ${safeClassToken(state)} ${activity.className}`.trim();
  if (badge.className !== className) badge.className = className;
  const description = activity.label || stateLabel(state);
  if (badge.getAttribute('title') !== description) {
    badge.setAttribute('title', description);
    badge.setAttribute('data-tooltip', description);
    badge.setAttribute('aria-label', description);
  }
  if (badge.dataset.iconKind !== activity.iconKind) {
    badge.dataset.iconKind = activity.iconKind;
    badge.innerHTML = `<span class="card-state-icon" aria-hidden="true">${stateIconSvgHtml(activity.iconKind)}</span>`;
  }
  return activity.className === 'awaiting-approval';
}
