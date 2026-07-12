import { t } from '../i18n.js';

type TourState = {
  completed?: boolean;
  showAfter?: number;
};

const DB_NAME = 'many-ai-cli-ui';
const STORE_NAME = 'first-run';
const STATE_KEY = 'p39-first-run-tour';
const LATER_DELAY_MS = 3 * 24 * 60 * 60 * 1000;

function openStateDB(): Promise<IDBDatabase | null> {
  return new Promise((resolve) => {
    if (!('indexedDB' in window)) return resolve(null);
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(STORE_NAME)) db.createObjectStore(STORE_NAME);
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => resolve(null);
  });
}

async function loadTourState(): Promise<TourState> {
  const db = await openStateDB();
  if (!db) return {};
  return new Promise((resolve) => {
    const tx = db.transaction(STORE_NAME, 'readonly');
    const request = tx.objectStore(STORE_NAME).get(STATE_KEY);
    request.onsuccess = () => resolve((request.result as TourState | undefined) ?? {});
    request.onerror = () => resolve({});
    tx.oncomplete = () => db.close();
  });
}

async function saveTourState(next: TourState): Promise<void> {
  const db = await openStateDB();
  if (!db) return;
  await new Promise<void>((resolve) => {
    const tx = db.transaction(STORE_NAME, 'readwrite');
    tx.objectStore(STORE_NAME).put(next, STATE_KEY);
    tx.oncomplete = () => { db.close(); resolve(); };
    tx.onerror = () => { db.close(); resolve(); };
  });
}

function makeButton(label: string, className: string): HTMLButtonElement {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = className;
  button.textContent = label;
  return button;
}

export function initFirstRunTour(): void {
  const start = () => {
    const helpButton = document.getElementById('first-run-tour-btn') as HTMLButtonElement | null;
    if (!helpButton) return;
    helpButton.setAttribute('aria-label', t('first_run_tour_open_tooltip'));

    let overlay: HTMLElement | null = null;
    let slideIndex = 0;
    const slides = [
      { title: 'first_run_tour_slide1_title', body: 'first_run_tour_slide1_body' },
      { title: 'first_run_tour_slide2_title', body: 'first_run_tour_slide2_body' },
      { title: 'first_run_tour_slide3_title', body: 'first_run_tour_slide3_body' },
      { title: 'first_run_tour_slide4_title', body: 'first_run_tour_slide4_body' },
    ];

    const close = () => {
      overlay?.remove();
      overlay = null;
      document.removeEventListener('keydown', onKeydown);
      helpButton.focus();
    };

    const complete = () => {
      void saveTourState({ completed: true });
      close();
    };

    const later = () => {
      void saveTourState({ showAfter: Date.now() + LATER_DELAY_MS });
      close();
    };

    const render = () => {
      if (!overlay) return;
      const slide = slides[slideIndex];
      overlay.innerHTML = '';
      const dialog = document.createElement('section');
      dialog.className = 'first-run-tour-dialog';
      dialog.setAttribute('role', 'dialog');
      dialog.setAttribute('aria-modal', 'true');
      dialog.setAttribute('aria-labelledby', 'first-run-tour-title');

      const head = document.createElement('div');
      head.className = 'first-run-tour-head';
      const progress = document.createElement('span');
      progress.className = 'first-run-tour-progress';
      progress.textContent = t('first_run_tour_progress', { current: slideIndex + 1, total: slides.length });
      const closeButton = makeButton(t('first_run_tour_skip'), 'first-run-tour-close');
      closeButton.addEventListener('click', complete);
      head.append(progress, closeButton);

      const title = document.createElement('h2');
      title.id = 'first-run-tour-title';
      title.textContent = t(slide.title);
      const body = document.createElement('p');
      body.className = 'first-run-tour-body';
      body.textContent = t(slide.body);
      const preview = document.createElement('div');
      preview.className = `first-run-tour-preview first-run-tour-preview--${slideIndex + 1}`;
      preview.textContent = t(`first_run_tour_slide${slideIndex + 1}_preview`);

      const footer = document.createElement('div');
      footer.className = 'first-run-tour-footer';
      const laterButton = makeButton(t('first_run_tour_later'), 'first-run-tour-later');
      laterButton.addEventListener('click', later);
      const previous = makeButton(t('first_run_tour_back'), 'first-run-tour-secondary');
      previous.disabled = slideIndex === 0;
      previous.addEventListener('click', () => { slideIndex--; render(); });
      const next = makeButton(slideIndex === slides.length - 1 ? t('first_run_tour_finish') : t('first_run_tour_next'), 'first-run-tour-next');
      next.addEventListener('click', () => {
        if (slideIndex === slides.length - 1) complete();
        else { slideIndex++; render(); }
      });
      footer.append(laterButton, previous, next);
      dialog.append(head, title, body, preview, footer);
      overlay.appendChild(dialog);
      next.focus();
    };

    const onKeydown = (event: KeyboardEvent) => {
      if (!overlay) return;
      if (event.key === 'Escape') { event.preventDefault(); complete(); }
      if (event.key === 'ArrowRight' && slideIndex < slides.length - 1) { event.preventDefault(); slideIndex++; render(); }
      if (event.key === 'ArrowLeft' && slideIndex > 0) { event.preventDefault(); slideIndex--; render(); }
    };

    const open = () => {
      if (overlay) return;
      slideIndex = 0;
      overlay = document.createElement('div');
      overlay.id = 'first-run-tour';
      overlay.addEventListener('click', (event) => { if (event.target === overlay) complete(); });
      document.body.appendChild(overlay);
      document.addEventListener('keydown', onKeydown);
      render();
    };

    helpButton.addEventListener('click', open);
    void loadTourState().then((state) => {
      if (!state.completed && (!state.showAfter || state.showAfter <= Date.now())) open();
    });
  };

  if (typeof window.t === 'function') start();
  else document.addEventListener('i18n-ready', start, { once: true });
}
