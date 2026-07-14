// TEMPORARY debug overlay for the v0.5.0 action-bar regression investigation.
// 500ms polling shows action-bar className / children.length /
// approvalVisibleCache / approvalRawOptionsCache / active session id.
// Remove this file and its app-entry import once the root cause is fixed.

import { activeSessionId, approvalRawOptionsCache, approvalVisibleCache } from './state.js';

export function initDebugApprovalOverlay() {
  if (document.getElementById('debug-approval-overlay')) return;
  const el = document.createElement('div');
  el.id = 'debug-approval-overlay';
  el.style.cssText = [
    'position:fixed',
    'right:8px',
    'bottom:8px',
    'z-index:2147483647',
    'background:rgba(0,0,0,0.82)',
    'color:#0f0',
    'font:11px/1.35 ui-monospace,Consolas,monospace',
    'padding:6px 8px',
    'border:1px solid #0f0',
    'border-radius:4px',
    'max-width:60vw',
    'pointer-events:auto',
    'white-space:pre',
    'cursor:pointer',
  ].join(';');
  el.title = 'click to hide (reload to restore)';
  el.addEventListener('click', () => { el.remove(); });
  document.body.appendChild(el);

  const tick = () => {
    const bar = document.getElementById('action-bar');
    const id = activeSessionId;
    const opts = id != null ? approvalRawOptionsCache.get(id) : null;
    const optsLen = Array.isArray(opts) ? opts.length : (opts ? 1 : 0);
    const visible = id != null ? !!approvalVisibleCache.get(id) : false;
    const cs = bar ? getComputedStyle(bar) : null;
    const lines = [
      `[dbg] active=${id ?? 'null'}  visible=${visible}  opts=${optsLen}`,
      `bar.class=${bar ? JSON.stringify(bar.className) : 'null'}`,
      `bar.children=${bar ? bar.children.length : 0}  hidden=${bar ? bar.hidden : '-'}`,
      `bar.display=${cs ? cs.display : '-'}  vis=${cs ? cs.visibility : '-'}`,
      `collapsed(ls)=${localStorage.getItem('ai_cli_hub_action_bar_collapsed') ?? 'null'}`,
    ];
    el.textContent = lines.join('\n');
  };
  tick();
  setInterval(tick, 500);
}
