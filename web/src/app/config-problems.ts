// config-problems.ts — 設定ファイル（config.yaml）の誤りを、直るまで消えない帯で出す。
//
// Hub は custom_providers の 1 件が誤っていても止まらず、その 1 件だけを外して起動する。
// 利用者が気づけるように、/api/info の config_problems を見て、誤りがある間は帯を出し続ける
// （× では閉じない。開くと 1 件ごとに: 何件目・id・どの欄・なぜ駄目か・外した結果・正しい書き方の例・
// 確かめるコマンド）。直ったら帯は消える。
//
// 読み直すきっかけ: 画面を開いたとき / このタブに戻ってきたとき / 開いている間は 30 秒ごと。
// Hub 側は呼ばれるたびに config.yaml を読み直す（読むだけ）ので、AI が裏で直したときも消える。
// 帯の中身は buildConfigBanner（config-problems-view.ts）が作る。ここは DOM と通信だけ。
import { t } from '../i18n.js';
import { apiFetch } from './util.js';
import { buildConfigBanner, parseConfigProblems, type ConfigBannerView } from './config-problems-view.js';

const BANNER_ID = 'config-problems-banner';
const POLL_MS = 30_000;
const MIN_GAP_MS = 2_000;

let lastKey = '';
let detailsOpen = false;
let lastFetchAt = 0;
let inFlight = false;

function host(): HTMLElement {
  const existing = document.getElementById(BANNER_ID);
  if (existing) return existing;
  const banner = document.createElement('div');
  banner.id = BANNER_ID;
  banner.hidden = true;
  banner.setAttribute('role', 'region');
  // 既存の帯（stale-binary-banner）の直前に置く。無ければ body の末尾。
  const anchor = document.getElementById('stale-binary-banner');
  if (anchor && anchor.parentElement) anchor.parentElement.insertBefore(banner, anchor);
  else document.body.appendChild(banner);
  return banner;
}

function line(className: string, text: string): HTMLElement {
  const el = document.createElement('div');
  el.className = className;
  el.textContent = text;
  return el;
}

function labelled(label: string, text: string): HTMLElement {
  const row = document.createElement('div');
  row.className = 'config-problems-row';
  const name = document.createElement('span');
  name.className = 'config-problems-label';
  name.textContent = label;
  const value = document.createElement('span');
  value.textContent = text;
  row.append(name, value);
  return row;
}

// 値はすべて textContent で入れる（id など利用者が書いた文字列を HTML として解釈しない）。
export function renderConfigBanner(banner: HTMLElement, view: ConfigBannerView | null): void {
  if (!view) {
    banner.hidden = true;
    banner.replaceChildren();
    banner.removeAttribute('data-kind');
    return;
  }
  banner.dataset.kind = view.kind;
  banner.setAttribute('aria-label', view.title);
  const details = document.createElement('details');
  details.className = 'config-problems-details';
  details.open = detailsOpen;
  details.addEventListener('toggle', () => {
    detailsOpen = details.open;
  });
  const summary = document.createElement('summary');
  summary.className = 'config-problems-summary';
  summary.append(line('config-problems-title', `⚠ ${view.title}`), line('config-problems-hint', view.summary));
  details.appendChild(summary);

  const body = document.createElement('div');
  body.className = 'config-problems-body';
  if (view.items.length > 0) {
    const list = document.createElement('ol');
    list.className = 'config-problems-list';
    for (const item of view.items) {
      const li = document.createElement('li');
      li.className = 'config-problems-item';
      li.appendChild(line('config-problems-heading', item.heading));
      li.appendChild(labelled(t('config_problems_label_field'), item.field));
      li.appendChild(labelled(t('config_problems_label_reason'), item.reason));
      li.appendChild(labelled(t('config_problems_label_effect'), item.effect));
      if (item.example) {
        li.appendChild(line('config-problems-label', t('config_problems_label_example')));
        const pre = document.createElement('pre');
        pre.className = 'config-problems-example';
        pre.textContent = item.example;
        li.appendChild(pre);
      }
      if (item.fixCommand) {
        li.appendChild(labelled(t('config_problems_label_fix'), item.fixCommand));
      }
      list.appendChild(li);
    }
    body.appendChild(list);
  }
  for (const note of view.notes) body.appendChild(line('config-problems-note', note));
  body.appendChild(
    line('config-problems-note', t('config_problems_check', { command: view.checkCommand })),
  );
  details.appendChild(body);
  banner.replaceChildren(details);
  banner.hidden = false;
}

export async function refreshConfigProblems(): Promise<void> {
  if (inFlight) return;
  inFlight = true;
  lastFetchAt = Date.now();
  try {
    const res = await apiFetch('/api/info', { cache: 'no-store' });
    if (!res.ok) return; // 取れないときは出ているものを変えない
    const info = await res.json();
    const view = buildConfigBanner(parseConfigProblems(info?.config_problems), t);
    // 中身が変わらないときは作り直さない（開いている説明が閉じてしまうため）
    const key = JSON.stringify(view);
    if (key === lastKey) return;
    lastKey = key;
    renderConfigBanner(host(), view);
  } catch (_) {
    // 通信できないときは出ているものを変えない
  } finally {
    inFlight = false;
  }
}

export function initConfigProblems(): void {
  // この帯の失敗で画面全体の起動を止めない。
  try {
    // 別窓（タブ 1 枚・グリッド）には出さない。本体の窓だけ。
    if (new URLSearchParams(location.search).get('view')) return;
    const start = () => {
      void refreshConfigProblems();
    };
    if (typeof window.t === 'function') start();
    else document.addEventListener('i18n-ready', start, { once: true });
    window.setInterval(() => {
      if (!document.hidden) void refreshConfigProblems();
    }, POLL_MS);
    const again = () => {
      if (!document.hidden && Date.now() - lastFetchAt > MIN_GAP_MS) void refreshConfigProblems();
    };
    document.addEventListener('visibilitychange', again);
    window.addEventListener('focus', again);
  } catch (_) {
    // 帯が出ないだけにとどめる
  }
}
