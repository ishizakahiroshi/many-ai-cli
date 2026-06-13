// detached-grid-launcher.ts — C5: Detached Grid プリセットと枚数指定
//
// プリセット → session 起動 / URL 生成ロジックを集約する。
// spawn-panel.ts や session-list.ts から呼び出して二重化を防ぐ。

import { token, escapeHtml } from './util.js';
import { sessions } from './state.js';
import { calcDetachedLayout, openDetachedGridForSessions } from './session-list.js';
import {
  STORAGE_DETACHED_GRID_PRESET_KEY,
  STORAGE_DETACHED_GRID_LAYOUT_KEY,
  STORAGE_DETACHED_GRID_COUNT_KEY,
  STORAGE_DETACHED_GRID_FAVORITES_KEY,
  getDetachedGridPrefs,
  setDetachedGridPrefs,
} from './user-prefs.js';

// ─── プリセット定義 ─────────────────────────────────────────────────────────

export type DetachedPresetId =
  | 'selected'
  | 'project'
  | 'current-multi'
  | 'claude+shell-2x2'
  | 'shell-2x2'
  | 'shell-3x3'
  | 'mixed-workspace'
  | 'custom';

export interface DetachedPreset {
  id: DetachedPresetId;
  labelKey: string;
  /** デフォルト layout */
  defaultLayout: string;
  /** デフォルト count (spawn 系プリセットのみ) */
  defaultCount?: number;
  /** URL のみで完結する場合 true（session spawn 不要） */
  urlOnly: boolean;
}

export const DETACHED_PRESETS: DetachedPreset[] = [
  {
    id: 'selected',
    labelKey: 'preset_selected_sessions',
    defaultLayout: '2x2',
    urlOnly: true,
  },
  {
    id: 'project',
    labelKey: 'preset_project_sessions',
    defaultLayout: '2x2',
    urlOnly: true,
  },
  {
    id: 'current-multi',
    labelKey: 'preset_current_multi',
    defaultLayout: '2x2',
    urlOnly: true,
  },
  {
    id: 'claude+shell-2x2',
    labelKey: 'preset_claude_shell_2x2',
    defaultLayout: '2x2',
    defaultCount: 4,
    urlOnly: false,
  },
  {
    id: 'shell-2x2',
    labelKey: 'preset_shell_2x2',
    defaultLayout: '2x2',
    defaultCount: 4,
    urlOnly: false,
  },
  {
    id: 'shell-3x3',
    labelKey: 'preset_shell_3x3',
    defaultLayout: '3x3',
    defaultCount: 9,
    urlOnly: false,
  },
  {
    id: 'mixed-workspace',
    labelKey: 'preset_mixed_workspace',
    defaultLayout: '2x2',
    urlOnly: true,
  },
  {
    id: 'custom',
    labelKey: 'preset_custom',
    defaultLayout: '2x2',
    defaultCount: 4,
    urlOnly: false,
  },
];

// ─── 起動ヘルパー ────────────────────────────────────────────────────────────

/** Detached Grid URL を生成する（session ids 既知の場合）。 */
export function buildDetachedGridUrl(sessionIds: number[], layout?: string): string {
  const params = new URLSearchParams(window.location.search);
  const tokenVal = params.get('token') || token;
  const resolvedLayout = layout || calcDetachedLayout(sessionIds.length);
  const idsStr = sessionIds.join(',');
  return `/?view=detached-grid&layout=${encodeURIComponent(resolvedLayout)}&session_ids=${idsStr}&token=${tokenVal}`;
}

/**
 * spawn-grid API を呼び出して複数 session を起動し、
 * WS で session id が揃い次第 Detached Grid を開く。
 */
export async function spawnGridAndOpen(opts: {
  preset: 'shell' | 'ai+shell';
  layout: string;
  count: number;
  cwd: string;
  labelPrefix?: string;
  provider?: string;
}): Promise<void> {
  const prevMax = sessions.size > 0 ? Math.max(...Array.from(sessions.keys())) : 0;

  const res = await fetch(`/api/spawn-grid?token=${token}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      preset: opts.preset,
      layout: opts.layout,
      count: opts.count,
      cwd: opts.cwd,
      label_prefix: opts.labelPrefix || 'grid',
      provider: opts.provider || '',
    }),
  });
  if (!res.ok) {
    const msg = await res.text().catch(() => '');
    throw new Error(`spawn-grid failed: ${msg}`);
  }
  const data = await res.json();
  const expectedCount: number = typeof data.count === 'number' ? data.count : opts.count;
  const layout: string = data.layout || opts.layout;

  // WS で session が expectedCount 個登録されるのを待ってから別窓を開く
  _waitForNewSessionsAndOpen(prevMax, expectedCount, layout);
}

const SPAWN_GRID_TIMEOUT_MS = 15000;
const SPAWN_GRID_POLL_MS = 300;

function _waitForNewSessionsAndOpen(prevMax: number, expectedCount: number, layout: string): void {
  const deadline = Date.now() + SPAWN_GRID_TIMEOUT_MS;

  function poll() {
    const allIds = Array.from(sessions.keys());
    const newIds = allIds.filter(id => id > prevMax);
    if (newIds.length >= expectedCount) {
      const sortedIds = newIds.sort((a, b) => a - b);
      const url = buildDetachedGridUrl(sortedIds, layout);
      window.open(url, '_blank');
      return;
    }
    if (Date.now() < deadline) {
      setTimeout(poll, SPAWN_GRID_POLL_MS);
    } else {
      // タイムアウト: 取れた分だけ開く
      const newIds2 = Array.from(sessions.keys()).filter(id => id > prevMax).sort((a, b) => a - b);
      if (newIds2.length > 0) {
        const url = buildDetachedGridUrl(newIds2, layout);
        window.open(url, '_blank');
      }
    }
  }
  setTimeout(poll, SPAWN_GRID_POLL_MS);
}

// ─── プリセット実行 ───────────────────────────────────────────────────────────

export interface LaunchPresetOptions {
  presetId: DetachedPresetId;
  layout?: string;
  count?: number;
  cwd?: string;
  provider?: string;
  /** 'selected' プリセット用: 選択中の session id */
  selectedIds?: number[];
  /** 'project' プリセット用: project group キー */
  projectKey?: string;
}

/**
 * プリセットに応じた Detached Grid を起動する。
 * エラー時は Error を throw する。
 */
export async function launchDetachedPreset(opts: LaunchPresetOptions): Promise<void> {
  const preset = DETACHED_PRESETS.find(p => p.id === opts.presetId);
  if (!preset) throw new Error(`Unknown preset: ${opts.presetId}`);

  const layout = opts.layout || preset.defaultLayout;

  switch (opts.presetId) {
    case 'selected': {
      const ids = opts.selectedIds || [];
      if (ids.length === 0) throw new Error('No sessions selected');
      openDetachedGridForSessions(ids);
      break;
    }

    case 'project': {
      const key = opts.projectKey || '';
      const projectIds = Array.from(sessions.values())
        .filter(s => {
          const cwdStr = s.cwd || '';
          const name = cwdStr.replace(/\\/g, '/').split('/').filter(p => p.length > 0).pop() || '';
          const projectKey = name || '__no_project__';
          return projectKey === key && (s.state === 'running' || s.state === 'waiting' || (s.state || 'standby') === 'standby');
        })
        .map(s => s.id);
      if (projectIds.length === 0) throw new Error(`No sessions for project: ${key}`);
      openDetachedGridForSessions(projectIds);
      break;
    }

    case 'current-multi': {
      const mgr = (window as any).multiPaneManager;
      if (!mgr || !Array.isArray(mgr.slots)) throw new Error('No multi pane manager');
      const slotIds: number[] = mgr.slots
        .filter((s: any) => s && s.session)
        .map((s: any) => s.session.id as number);
      if (slotIds.length === 0) throw new Error('No sessions in current multi layout');
      openDetachedGridForSessions(slotIds);
      break;
    }

    case 'claude+shell-2x2': {
      const count = opts.count ?? 4;
      const provider = opts.provider || 'claude';
      await spawnGridAndOpen({
        preset: 'ai+shell',
        layout,
        count,
        cwd: opts.cwd || '',
        labelPrefix: `${provider}-shell`,
        provider,
      });
      break;
    }

    case 'shell-2x2': {
      await spawnGridAndOpen({
        preset: 'shell',
        layout: opts.layout || '2x2',
        count: opts.count ?? 4,
        cwd: opts.cwd || '',
        labelPrefix: 'shell',
      });
      break;
    }

    case 'shell-3x3': {
      await spawnGridAndOpen({
        preset: 'shell',
        layout: opts.layout || '3x3',
        count: opts.count ?? 9,
        cwd: opts.cwd || '',
        labelPrefix: 'shell',
      });
      break;
    }

    case 'mixed-workspace': {
      // active AI sessions + Shell sessions を全て含む
      const mixedIds = Array.from(sessions.values())
        .filter(s => s.state === 'running' || s.state === 'waiting')
        .map(s => s.id);
      if (mixedIds.length === 0) throw new Error('No active sessions');
      openDetachedGridForSessions(mixedIds);
      break;
    }

    case 'custom': {
      const count = opts.count ?? 4;
      await spawnGridAndOpen({
        preset: 'shell',
        layout,
        count,
        cwd: opts.cwd || '',
        labelPrefix: 'grid',
      });
      break;
    }

    default:
      throw new Error(`Unhandled preset: ${opts.presetId}`);
  }

  // prefs に直近設定を保存
  const savedPrefs = getDetachedGridPrefs();
  setDetachedGridPrefs({
    ...savedPrefs,
    lastPreset: opts.presetId,
    lastLayout: layout,
    lastCount: opts.count ?? preset.defaultCount ?? 4,
  });
}

// ─── Launcher ダイアログ ─────────────────────────────────────────────────────

/**
 * Detached Grid Launcher ダイアログを表示する。
 * opts.selectedIds / opts.projectKey が渡された場合は対応プリセットを強調する。
 */
export function openDetachedGridLauncher(opts?: {
  selectedIds?: number[];
  projectKey?: string;
  cwd?: string;
}): void {
  // 既存ダイアログがあれば閉じる
  const existing = document.getElementById('detached-grid-launcher-overlay');
  if (existing) existing.remove();

  const prefs = getDetachedGridPrefs();
  const currentCwd = opts?.cwd || '';

  const overlay = document.createElement('div');
  overlay.id = 'detached-grid-launcher-overlay';
  overlay.className = 'detached-grid-launcher-overlay';

  const tw = typeof window.t === 'function' ? window.t : (k: string, fb?: string) => fb || k;

  // プリセットリストを生成
  const presetListHtml = DETACHED_PRESETS.map(p => {
    const isFav = (prefs.favoritePresets || []).includes(p.id);
    const label = tw(p.labelKey, p.id);
    return (
      `<div class="dgl-preset-item" data-preset="${p.id}">` +
      `<button class="dgl-preset-fav${isFav ? ' is-fav' : ''}" data-preset="${p.id}" type="button" title="${tw('preset_fav_toggle', 'Toggle favorite')}">★</button>` +
      `<span class="dgl-preset-label">${label}</span>` +
      `<span class="dgl-preset-badge">${p.urlOnly ? '' : (p.defaultCount ? `×${p.defaultCount}` : '')}</span>` +
      `<button class="dgl-preset-launch" data-preset="${p.id}" type="button">${tw('preset_launch', 'Open')}</button>` +
      `</div>`
    );
  }).join('');

  const savedLayouts = ['1x1', '1x2', '2x2', '2x3', '3x3', '4x3'];
  const layoutOpts = savedLayouts.map(l =>
    `<option value="${l}"${prefs.lastLayout === l ? ' selected' : ''}>${l}</option>`
  ).join('');

  overlay.innerHTML =
    `<div class="dgl-dialog">` +
    `<div class="dgl-header">` +
    `<span class="dgl-title">${tw('dgl_title', 'Open Detached Grid')}</span>` +
    `<button class="dgl-close" id="dgl-close-btn" type="button">✕</button>` +
    `</div>` +
    `<div class="dgl-body">` +
    `<div class="dgl-presets">${presetListHtml}</div>` +
    `<div class="dgl-options">` +
    `<label class="dgl-opt-label">${tw('dgl_layout', 'Layout')}</label>` +
    `<select class="dgl-layout-select" id="dgl-layout-select">${layoutOpts}</select>` +
    `<label class="dgl-opt-label">${tw('dgl_count', 'Count')}</label>` +
    `<input class="dgl-count-input" id="dgl-count-input" type="number" min="1" max="18" value="${prefs.lastCount || 4}">` +
    `<label class="dgl-opt-label">${tw('dgl_cwd', 'CWD')}</label>` +
    `<input class="dgl-cwd-input" id="dgl-cwd-input" type="text" value="${escapeHtml(currentCwd)}">` +
    `</div>` +
    `</div>` +
    `</div>`;

  document.body.appendChild(overlay);

  function close() { overlay.remove(); }

  document.getElementById('dgl-close-btn')?.addEventListener('click', close);
  overlay.addEventListener('mousedown', (e) => {
    if (e.target === overlay) close();
  });

  // プリセット起動ボタン
  overlay.querySelectorAll<HTMLButtonElement>('.dgl-preset-launch').forEach(btn => {
    btn.addEventListener('click', async () => {
      const presetId = btn.dataset.preset as DetachedPresetId;
      const layout = (document.getElementById('dgl-layout-select') as HTMLSelectElement)?.value || '2x2';
      const count = parseInt((document.getElementById('dgl-count-input') as HTMLInputElement)?.value || '4', 10);
      const cwd = (document.getElementById('dgl-cwd-input') as HTMLInputElement)?.value.trim() || currentCwd;
      close();
      try {
        await launchDetachedPreset({
          presetId,
          layout,
          count,
          cwd,
          selectedIds: opts?.selectedIds,
          projectKey: opts?.projectKey,
        });
      } catch (err: any) {
        if (typeof (window as any).showToast === 'function') {
          (window as any).showToast(`Detached Grid: ${err.message}`);
        } else {
          console.error('[detached-grid-launcher]', err);
        }
      }
    });
  });

  // お気に入りトグル
  overlay.querySelectorAll<HTMLButtonElement>('.dgl-preset-fav').forEach(btn => {
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      const presetId = btn.dataset.preset as DetachedPresetId;
      const savedPrefs = getDetachedGridPrefs();
      const favs: DetachedPresetId[] = (savedPrefs.favoritePresets || []) as DetachedPresetId[];
      const idx = favs.indexOf(presetId);
      if (idx >= 0) {
        favs.splice(idx, 1);
        btn.classList.remove('is-fav');
      } else {
        favs.push(presetId);
        btn.classList.add('is-fav');
      }
      setDetachedGridPrefs({ ...savedPrefs, favoritePresets: favs });
    });
  });
}

// window に公開（spawn-panel.ts / session-list.ts から参照可）
(window as any).openDetachedGridLauncher = openDetachedGridLauncher;
(window as any).launchDetachedPreset = launchDetachedPreset;
