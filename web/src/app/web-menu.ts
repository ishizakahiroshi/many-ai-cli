/* ヘッダーの Web メニュー（Hub 自体への操作）。web/ ディレクトリの機能名ではない。 */

export function initWebMenu(): void {
  const btn = document.getElementById('web-menu-btn');
  const dropdown = document.getElementById('web-menu-dropdown');
  if (!btn || !dropdown) return;

  // header は backdrop-filter による stacking context を持つため、
  // ポップオーバーを body 直下へ移して z-index を確実に効かせる。
  if (dropdown.parentElement !== document.body) {
    document.body.appendChild(dropdown);
  }

  const positionDropdown = (): void => {
    const anchor = btn.getBoundingClientRect();
    const menu = dropdown.getBoundingClientRect();
    const margin = 6;
    const left = Math.max(
      margin,
      Math.min(anchor.right - menu.width, window.innerWidth - menu.width - margin),
    );
    const top = Math.max(
      margin,
      Math.min(anchor.bottom + 4, window.innerHeight - menu.height - margin),
    );
    dropdown.style.top = `${top}px`;
    dropdown.style.left = `${left}px`;
    dropdown.style.right = 'auto';
  };

  const closeDropdown = (): void => {
    dropdown.hidden = true;
    btn.setAttribute('aria-expanded', 'false');
  };

  btn.addEventListener('click', (event) => {
    event.stopPropagation();
    if (!dropdown.hidden) {
      closeDropdown();
      return;
    }
    dropdown.hidden = false;
    positionDropdown();
    btn.setAttribute('aria-expanded', 'true');
  });

  // 子ダイアログを開く外部公開以外は、選択後にメニューを閉じる。
  dropdown.querySelectorAll<HTMLElement>('.web-menu-item:not(#expose-btn)').forEach((item) => {
    item.addEventListener('click', closeDropdown);
  });

  const onOutsidePointer = (event: MouseEvent | TouchEvent): void => {
    if (dropdown.hidden) return;
    const target = event.target as Node | null;
    if (!target || btn.contains(target) || dropdown.contains(target)) return;
    // 外部公開の子ポップオーバーは body 直下に出るため、開いている間は
    // Web メニューを閉じず、ボタンの矩形を維持する。
    if (target instanceof Element && target.closest('.expose-pop')) return;
    closeDropdown();
  };

  document.addEventListener('mousedown', onOutsidePointer, true);
  document.addEventListener('touchstart', onOutsidePointer, true);
  document.addEventListener('keydown', (event: KeyboardEvent) => {
    if (event.key !== 'Escape' || dropdown.hidden) return;
    event.preventDefault();
    event.stopPropagation();
    closeDropdown();
  }, true);
  window.addEventListener('resize', () => {
    if (!dropdown.hidden) positionDropdown();
  });
}
