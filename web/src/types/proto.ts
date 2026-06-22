// TypeScript mirror for internal/proto/messages.go.
// Go side is the source of truth; update this file when JSON fields or message
// type values change in internal/proto/messages.go.

export type ProviderID = 'claude' | 'codex' | 'copilot' | 'cursor-agent' | 'opencode' | 'grok' | 'common' | string;

export type SessionState =
  | 'standby'
  | 'running'
  | 'waiting'
  | 'completed'
  | 'error'
  | 'disconnected'
  | string;

export type MessageType =
  | 'register'
  | 'registered'
  | 'reattach'
  | 'reattach_ack'
  | 'reattach_reject'
  | 'snapshot'
  | 'session_update'
  | 'session_end'
  | 'session_removed'
  | 'session_dismiss'
  | 'session_history_reset'
  | 'pty_data'
  | 'pty_input'
  | 'pty_resize'
  | 'session_hint'
  | 'approval_detected'
  | 'approval_marker'
  | 'approval_cleared'
  | 'approval_consumed'
  | 'approval_patterns_updated'
  | 'commit_msg_suggested'
  | 'commit_msg_error'
  | 'input_deferred'
  | 'attach_request'
  | 'hub_shutdown'
  | 'ping'
  | 'usage_stat';

export interface ApprovalOption {
  num: number;
  label?: string;
  is_current?: boolean;
  send_text?: string;
  preserve_order?: boolean;
}

export interface Message {
  type: MessageType;
  role?: string;
  session_id?: number;
  provider?: ProviderID;
  display_name?: string;
  cwd?: string;
  branch?: string;
  pid?: number;
  shell?: string;
  version?: string;
  state?: SessionState;
  exit_code?: number;
  token?: string | null;
  data?: string | Uint8Array;
  text?: string;
  cols?: number;
  rows?: number;
  log_path?: string;
  jsonl_path?: string;
  replay_b64?: string;
  reason?: string;
  approval_visible?: boolean;
  approval_sig?: string;
  approval_kind?: string;
  approval_source?: string;
  approval_question?: string;
  approval_context?: string;
  approval_options?: ApprovalOption[];
  block?: string;
  sent_text?: string;
  commit_subject?: string;
  commit_body?: string;
  detected_at?: string;
  last_output_at?: string;
  started_at?: string;
  label?: string;
  model?: string;
  route?: string;
  first_message?: string;
  last_message?: string;
  inject?: string;
  image_data?: string;
  filename?: string;
  providers?: ProviderID[];
  ui_active_session_id?: number;
  sessions?: string | SessionSnapshot[];
  hub_instance?: string;
  // usage_stat フィールド: 数値メタデータのみ。本文は含まない。
  // cost_known が false のときはコスト不明（価格表未登録モデル）→ "$ —" 表示。
  cost_usd?: number;
  cost_known?: boolean;
  tokens_in?: number;
  tokens_out?: number;
  tokens_cache?: number;
  tokens_total?: number;
  ctx_window?: number;
  ctx_used_pct?: number; // Claude Code statusLine 算出済みの context 使用率%（0/未送=未取得。Claude のみ）
  usage_model?: string;
  usage_started_at?: string;
  // statusbar 追加メタ（Claude statusLine ネイティブ算出値・Claude のみ・0/未送=未取得）
  rl_5h_pct?: number;       // 5時間レート制限の使用率%（Pro/Max のみ）
  rl_5h_reset?: number;     // 同リセット時刻（unix epoch 秒）
  rl_7d_pct?: number;       // 週次レート制限の使用率%
  rl_7d_reset?: number;     // 同リセット時刻（unix epoch 秒）
  lines_added?: number;     // AI がこのセッションで追加した行数
  lines_removed?: number;   // AI がこのセッションで削除した行数
  effort_level?: string;    // reasoning effort（low/medium/high/xhigh/max）
  thinking?: boolean;       // 拡張思考の有効/無効
  exceeds_200k?: boolean;   // 直近 API 応答の総トークンが 200k 超か
  duration_ms?: number;     // セッション総経過時間（ms）
  api_duration_ms?: number; // うち API 応答待ち時間（ms）
  output_style?: string;    // Claude Code output_style.name
  vim_mode?: string;        // Claude Code vim.mode
  agent_name?: string;      // Claude Code agent.name
  repo_host?: string;       // workspace.repo.host
  repo_owner?: string;      // workspace.repo.owner
  repo_name?: string;       // workspace.repo.name
  remaining_pct?: number;   // Claude Code statusLine 算出済みの context 残り%
  reasoning_output_tokens?: number; // Codex token_count.info reasoning_output_tokens
  // C3: git 変更状況メタ（git_checked=true のメッセージのみ有効）
  git_checked?: boolean;
  git_files?: number;
  git_added?: number;
  git_deleted?: number;
  [key: string]: unknown;
}

export interface SessionSnapshot {
  id: number;
  provider?: ProviderID;
  display_name?: string;
  cwd?: string;
  project?: string;
  branch?: string;
  label?: string;
  model?: string;
  route?: string;
  shell?: string;
  state?: SessionState;
  last_output_at?: string;
  started_at?: string;
  first_message?: string;
  last_message?: string;
  end_reason?: string;
  log_path?: string;
  jsonl_path?: string;
  // C3: git 変更状況メタ（session_update で git_checked=true 時に蓄積）
  git_files?: number;
  git_added?: number;
  git_deleted?: number;
  [key: string]: unknown;
}

