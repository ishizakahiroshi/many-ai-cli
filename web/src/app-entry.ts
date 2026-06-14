// ESM entry point (generated). Imports modules in original load order for side effects.
import './i18n.js';
import './app/util.js';
import './app/user-prefs.js';
import './app/state.js';
import './app/multi-pane.js';
import './app.js';
import './app/session-list.js';
import './app/path-links.js';
import './app/terminal.js';
import './app/history-viewer.js';
import './app/settings.js';
import './app/ws-client.js';
import './app/approval-parser.js';
import './app/approval-ui.js';
import './app/approval.js';
import './app/approval-queue-tab.js';
import './app/session-swipe.js';
import './app/chat-history.js';
import './app/attachments.js';
import './app/spawn-panel.js';
import './app/voice.js';
import './app/voice-whisper.js';
import './app/git-view.js';
import './app/files-view.js';
import './app/workbench.js';
import './app/pwa.js';
import { initTokenStatusbar } from './app/token-statusbar.js';
import { initDetachedGridMode } from './app/detached-grid.js';
import { initServerModal } from './app/server-modal.js';
import { initMobileConnect } from './app/mobile-connect.js';
// ステータスバー初期化（/api/user-prefs から enabled を読む）
initTokenStatusbar();
// detached-grid モード判定（/?view=detached-grid の場合のみ初期化）
initDetachedGridMode();
// 🖥 Server モーダル（内蔵リモート接続）の配線
initServerModal();
// 📱 モバイル接続ウィザード（QR）の配線
initMobileConnect();
