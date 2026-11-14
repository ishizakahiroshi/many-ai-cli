// PC（幅 721px 以上・マウス操作）では、サイドバー上端のアイコン列をヘッダーへ移す。
// 毎日使うのは「新しいセッション」だけなので、列の 1 行ぶんの高さを空け、
// 受信箱（看板から任された仕事）と「＋」だけを外に出し、残りは「⋯」メニューにまとめる
// （2026-11-14 利用者決定）。
//
// ボタンは作り直さず、同じ要素を移すだけにする。id・data 属性・既存のクリック処理が
// そのまま効く。スマホ幅・タッチ操作の広い画面では元の位置へ戻す（ドロワーが #new-session-bar を使うため）。

const DESKTOP_QUERY = '(min-width: 721px) and (pointer: fine)';

function tr(key: string, fallback: string): string {
  if (typeof window.t !== 'function' || !key) return fallback;
  const value = window.t(key);
  return value && value !== key ? value : fallback;
}

function decorate(btn: HTMLElement, asMenuItem: boolean): void {
  btn.classList.toggle('ui-button--menu-item', asMenuItem);
  btn.querySelector(':scope > .header-more-label')?.remove();
  if (!asMenuItem) return;
  const label = document.createElement('span');
  label.className = 'header-more-label';
  label.textContent = tr(btn.getAttribute('data-i18n-tooltip') ?? '', btn.getAttribute('aria-label') ?? '');
  btn.insertBefore(label, btn.querySelector('.memo-button-badge'));
}

function init(): void {
  const bar = document.getElementById('new-session-bar');
  const header = document.querySelector('body > header');
  const newBtn = document.getElementById('new-session-btn');
  if (!bar || !header || !newBtn) return;

  const anchor = document.createComment('new-session-bar-home');
  bar.before(anchor);
  const originalOrder = Array.from(bar.children) as HTMLElement[];
  const boardBtn = originalOrder.find(el => el.hasAttribute('data-open-board-handoffs')) ?? null;
  const menuTargets = originalOrder.filter(el => el !== newBtn && el !== boardBtn);

  const more = document.createElement('div');
  more.className = 'header-more';
  const moreBtn = document.createElement('button');
  moreBtn.type = 'button';
  moreBtn.id = 'header-more-btn';
  moreBtn.className = 'ui-button ui-button--secondary ui-button--compact header-more-btn';
  moreBtn.textContent = '⋯';
  moreBtn.setAttribute('aria-haspopup', 'menu');
  moreBtn.setAttribute('aria-expanded', 'false');
  const dot = document.createElement('span');
  dot.className = 'memo-button-badge header-more-dot';
  dot.setAttribute('aria-hidden', 'true');
  dot.hidden = true;
  moreBtn.appendChild(dot);
  const menu = document.createElement('div');
  menu.className = 'header-more-menu';
  menu.id = 'header-more-menu';
  menu.setAttribute('role', 'menu');
  menu.setAttribute('data-wheel-native', '');
  menu.hidden = true;
  more.append(moreBtn, menu);

  const refreshTip = (): void => {
    const tip = tr('header_more_tooltip', 'More');
    moreBtn.title = tip;
    moreBtn.setAttribute('aria-label', tip);
  };
  // メニューの中のメモの件数バッジが見えているときは、「⋯」にも同じ件数を出す（隠れても件数に気づけるように）。
  const refreshDot = (): void => {
    const badge = menu.querySelector<HTMLElement>('.memo-button-badge:not([hidden])');
    dot.hidden = !badge;
    dot.textContent = badge?.textContent ?? '';
  };
  new MutationObserver(refreshDot).observe(menu, {
    subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ['hidden'],
  });

  const setOpen = (open: boolean): void => {
    if (open) {
      for (const el of menuTargets) decorate(el, true);
      refreshTip();
    }
    menu.hidden = !open;
    moreBtn.setAttribute('aria-expanded', open ? 'true' : 'false');
  };
  moreBtn.addEventListener('click', () => setOpen(!!menu.hidden));
  document.addEventListener('click', event => {
    if (menu.hidden) return;
    const target = event.target as Node | null;
    if (target && moreBtn.contains(target)) return;
    setOpen(false);
  });
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape' && !menu.hidden) { setOpen(false); moreBtn.focus(); }
  });

  let hosted = false;
  const apply = (desktop: boolean): void => {
    if (desktop === hosted) return;
    hosted = desktop;
    if (desktop) {
      for (const el of menuTargets) { menu.appendChild(el); decorate(el, true); }
      bar.replaceChildren(...[boardBtn, newBtn, more].filter((el): el is HTMLElement => !!el));
      header.insertBefore(bar, header.querySelector(':scope > .header-sep'));
      refreshTip();
      refreshDot();
    } else {
      setOpen(false);
      for (const el of menuTargets) decorate(el, false);
      bar.replaceChildren(...originalOrder);
      anchor.after(bar);
    }
    bar.classList.toggle('header-hosted', desktop);
  };

  const mq = window.matchMedia(DESKTOP_QUERY);
  apply(mq.matches);
  mq.addEventListener('change', event => apply(event.matches));
}

init();
