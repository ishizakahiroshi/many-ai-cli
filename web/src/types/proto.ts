// TypeScript mirror for internal/proto/messages.go.
// Go side is the source of truth; update this file when JSON fields or message
// type values change in internal/proto/messages.go.

export type ProviderID = 'claude' | 'codex' | 'copilot' | 'cursor-agent' | 'common' | string;

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
  | 'approval_cleared'
  | 'approval_consumed'
  | 'approval_patterns_updated'
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
  sent_text?: string;
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
  usage_model?: string;
  usage_started_at?: string;
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

