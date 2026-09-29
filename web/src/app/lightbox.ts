// 画像・動画の拡大表示（#image-lightbox）。何も import しない葉モジュール。
// attachments.ts は app.js を import するので、作業メモ（別窓でも動く memo-panel.ts）から
// 使えるようにここへ切り出した。既存の呼び出し元は attachments.ts の re-export 経由で読む。
export function openLightbox(src, opts: any = {}) {
  const overlay = document.createElement('div');
  overlay.id = 'image-lightbox';
  overlay.classList.add('aac-wheel-overlay');
  const isVideo = opts.type === 'video';
  const media: any = document.createElement(isVideo ? 'video' : 'img');
  if (isVideo) {
    media.controls = true;
    media.autoplay = true;
    media.playsInline = true;
  }
  media.src = src;
  overlay.appendChild(media);
  document.body.appendChild(overlay);
  const close = () => {
    if (isVideo) {
      try { media.pause(); } catch (_) {}
    }
    overlay.remove();
    document.removeEventListener('keydown', onKey);
  };
  const onKey = (e) => { if (e.key === 'Escape') close(); };
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) close();
  });
  document.addEventListener('keydown', onKey);
}
