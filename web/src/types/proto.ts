// TypeScript mirror for internal/proto/messages.go.
// Go side is the source of truth; update this file when JSON fields or message
// type values change in internal/proto/messages.go.

export type ProviderID = 'claude' | 'codex' | 'copilot' | 'cursor-agent' | 'opencode' | 'grok' | 'command-code' | 'common' | string;

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
  | 'reattach_replay_done'
  | 'reattach_reject'
  | 'snapshot'
  | 'session_update'
  | 'session_end'
  | 'session_removed'
  | 'session_dismiss'
  | 'session_history_reset'
  | 'pty_data'
  | 'agent_chat'
  | 'pty_input'
  | 'pty_input_ack'
  | 'pty_resize'
  | 'approval_marker_suppressed'
  | 'approval_consumed'
  // 画面 → Hub の問い直し（抑止告知の「再検出」）。Hub は今の記録を問い直した画面にだけ送る。
  | 'approval_resync'
  // 保留中の承認の記録（internal/hub/approval_record.go）。開閉の差分と、接続時のまとめ。
  | 'approval_state'
  | 'approval_snapshot'
	| 'auto_approval_applied'
  | 'approval_patterns_updated'
  | 'binary_stale'
  | 'commit_msg_suggested'
  | 'commit_msg_error'
  | 'input_deferred'
  | 'attach_request'
  | 'hub_shutdown'
  | 'ping'
  | 'usage_stat'
  | 'workflow_progress'
  // サブエージェントの木（親 plan: plan_subagent-tree-popup.md）。セッション単位の WS だけに送られる。
  | 'subagent_tree'
  | 'done_summary'
  | 'git_turn'
  | 'user_turn_started'
  | 'spawn_confirmation_requested'
  | 'spawn_confirmation_closed'
  // C5 (plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md): Hub → UI.
  // 承認待ちを抱えた親セッションの dismiss を Hub が拒否したことを伝える。
  // session_id は拒否された対象、reason は固定文言不要（UI 側は一律のトースト文言を出す）。
  // 型だけ先に用意しておく（Hub 側の送信実装は internal/ 配下のため本 C の範囲外）。
  | 'session_dismiss_refused';

// 'unknown' は Hub の フォールバック 専用（マーカーが無いままターンが終わった）。
// 「終わったが何が終わったか分からない」であって異常ではないので needs_action と分ける。
export type DoneSummaryKind = 'success' | 'failure' | 'aborted' | 'needs_action' | 'unknown' | string;
export interface DoneSummary {
  session_id: number;
  provider?: ProviderID;
  title?: string;
  text: string;
  kind: DoneSummaryKind;
  at: string;
  fallback?: boolean;
}

export interface ApprovalOption {
  num: number;
  label?: string;
  is_current?: boolean;
  send_text?: string;
  preserve_order?: boolean;
}

export type ApprovalRiskTier = 'low' | 'mid' | 'high';

// Mirror of internal/proto.ApprovalSummary. Raw is disclosure-only and must
// never be used as an authorization decision input in the browser.
export interface ApprovalSummary {
  command?: string;
  paths?: string[];
  risk: ApprovalRiskTier;
  raw?: string;
}

/**
 * Hub が持つ保留中の承認の記録（internal/proto.ApprovalRecord のミラー）。
 * マーカーは block の原文だけを持ち、選択肢はブラウザ側の Hub ブロック用パーサで解く。
 * ネイティブは Go 側で解いた question / context / options / summary を持つ。
 * sig は台帳と回答の引き当て用の参照で、同一性ではない（同一性は candidate_key + source_epoch）。
 */
export interface ApprovalRecord {
  candidate_key: string;
  candidate_shape?: string;
  source_epoch: number;
  sig?: string;
  /** native | marker */
  origin: string;
  /** go_vt | transcript */
  source?: string;
  kind?: string;
  block?: string;
  question?: string;
  context?: string;
  options?: ApprovalOption[];
  summary?: ApprovalSummary;
  detected_at?: string;
}

/** 閉じた記録と理由（internal/proto.ApprovalRecordClose のミラー）。 */
export interface ApprovalRecordClose {
  candidate_key: string;
  source_epoch: number;
  sig?: string;
  origin?: string;
  /** answered | answered_terminal | superseded | vanished | session_end | history_reset */
  reason: string;
}

/**
 * 1 セッションの記録の開閉（type='approval_state'。internal/proto.ApprovalState のミラー）。
 * open と close のどちらか一方だけが入る。version はセッションごとに開閉のたびに 1 増え、
 * 手元より古いものは捨てる（Hub はロックを外してから送るので、逆順に届くことがある）。
 * 例外は approval_resync への返事（問い直した画面にだけ届く）で、今の version と open を持ち、
 * 記録が無ければどちらも空（その版では記録が無い）。
 */
export interface ApprovalState {
  version: number;
  open?: ApprovalRecord;
  close?: ApprovalRecordClose;
}

/** approval_snapshot の 1 セッション分。record が無いセッションは「保留中の承認なし」。 */
export interface ApprovalSessionState {
  session_id: number;
  version: number;
  record?: ApprovalRecord;
}

export interface SessionMeta {
  label: string;
  pinned: boolean;
  color: string;
  note: string;
  auto_title: string;
}

export interface SessionActivity {
  output_idle: boolean;
  workflow_active: boolean;
  awaiting_user: boolean;
  awaiting_approval: boolean;
}

export interface AgentChatTool {
  id?: string;
  name: string;
  input?: string;
  result?: string;
}

export interface AgentChatMessage {
  role: 'user' | 'assistant' | string;
  kind?: string;
  text?: string;
  thinking?: string[];
  tools?: AgentChatTool[];
  ts?: string;
  message_id?: string;
}

export interface WfAgent {
  label: string;
  state: string;
  metrics?: string;
  detail?: WfAgentDetail;
}

// Mirror of internal/proto.WfAgentDetail. Populated only when the Hub could
// resolve a task ID for the running Workflow (see
// docs/local/plan_workflow-progress-agent-transcript-detail_c1_investigation-proto.md
// "発見2"). Fields sourced from prompt_preview/result_preview/last_tool_summary
// carry excerpted user/agent content and must never be logged, persisted, or
// forwarded outside the per-session WS payload.
export interface WfAgentDetail {
  model?: string;
  started_at?: number; // epoch ms
  last_progress_at?: number; // epoch ms
  duration_ms?: number;
  tokens?: number;
  tool_calls?: number;
  last_tool_name?: string;
  last_tool_summary?: string; // truncated, see C2 budget
  prompt_preview?: string; // truncated
  result_preview?: string; // truncated
}

export interface WfPhase {
  title: string;
  agents: WfAgent[];
}

export interface WorkflowProgress {
  detected: boolean;
  source?: string;
  name?: string;
  done: number;
  total: number;
  running: number;
  failed: number;
  pending: number;
  waiting_dynamic: number;
  percent: number;
  elapsed_sec?: number;
  tokens_raw?: string;
  phases?: WfPhase[];
  settled: boolean;
  settled_by?: string;
  task_detail_source?: string;
}

/**
 * サブエージェントの木の 1 ノード（internal/proto.SubagentNode のミラー）。
 * WfAgentDetail と同じ規律で、プロンプト本文・ツール結果・子の返答を保持する
 * フィールドは持たない。last_tool_summary は許可したキー（command / pattern /
 * path / file_path）の値を切り詰めたものだけ。
 */
export interface SubagentNode {
  id: string;
  /** 空 = 最上位（親の直接の子）。 */
  parent_id?: string;
  /** 1 = 最上位の子, 2 = 孫, ... */
  depth: number;
  /** 親が付けた短い名前（Claude: description, Codex: agent_nickname/agent_role）。プロンプトそのものではない。 */
  label?: string;
  agent_type?: string;
  model?: string;
  /** running | done | failed | unknown */
  state: string;
  started_at?: number; // epoch ms
  last_activity_at?: number; // epoch ms
  finished_at?: number; // epoch ms
  /** 読み取り側が数えきれていないときは未送（0 を「まだ 0 回」と誤解させない）。 */
  tool_calls?: number;
  last_tool_name?: string;
  last_tool_summary?: string;
}

/**
 * サブエージェントの木（internal/proto.SubagentTree のミラー）。セッション単位の
 * WS だけに送られ、ログ出力・永続化・外部送信はしない
 * （docs/local/plan_subagent-tree-popup.md 方針 3・6）。
 */
export interface SubagentTree {
  /** この木を作った adapters.subagents の読み方（セッションの provider と一致するとは限らない）。 */
  provider?: string;
  nodes?: SubagentNode[];
  /** 上限で落とした件数。 */
  omitted?: number;
  updated_at?: number; // epoch ms
}

/** relay ループ 1 本の状態（internal/proto/messages.go の RelayStatus のミラー）。 */
export interface RelayStatus {
  orchestration_id?: string;
  plan_path?: string;
  mode?: 'worktree' | 'same-tree' | string;
  state?: 'implementing' | 'reviewing' | 'fixing' | 'completed' | 'stopped' | string;
  reason?: string;
  completed_cs?: number;
  round?: number;
  max_rounds?: number;
  final_seen?: boolean;
  implementation_session_id?: number;
  /** 強い実装役（D-22）。未 spawn は 0 / 未送。 */
  strong_session_id?: number;
  active_implementer?: 'implementation' | 'implementation-strong' | string;
  escalate_after?: number;
  review_session_id?: number;
  review_path?: string;
  worktree_path?: string;
  branch?: string;
  base_commit?: string;
  updated_at?: string;
}

/** relay の時系列 1 行（RelayEvent のミラー）。 */
export interface RelayEvent {
  at?: string;
  kind?: string;
  c?: number;
  round?: number;
  text?: string;
  review_path?: string;
  commit?: string;
  files_changed?: number;
}

/** Hub が Claude PTY の受信ヘッダーから観測した board 外通信の要約。 */
export interface CrossSessionMessage {
  at?: string;
  receiver_session_id?: number;
  receiver_role?: string;
  sender?: string;
  text?: string;
}

export interface Message {
  type: MessageType;
  role?: string;
  session_id?: number;
  provider?: ProviderID;
  display_name?: string;
  cwd?: string;
  branch?: string;
  /** cwd が属する本体リポジトリのルート（hub/project_id.go）。worktree でも本体と同じ値。 */
  project_id?: string;
  pid?: number;
  input_seq?: number;
  shell?: string;
  version?: string;
  state?: SessionState;
	output_idle?: boolean;
	workflow_active?: boolean;
	awaiting_user?: boolean;
	awaiting_approval?: boolean;
	activity?: SessionActivity;
  workflow_progress?: WorkflowProgress;
  /** サブエージェントの木。type='subagent_tree' のメッセージでのみ届く。 */
  subagent_tree?: SubagentTree;
  exit_code?: number;
  token?: string | null;
  data?: string | Uint8Array;
  text?: string;
  agent_session_id?: string;
  // subscription profile（複数サブスクリプション管理）。空/未送は「CLI 自身の
  // ログイン環境」を意味する。ID は不透明な識別子で、認証情報は含まない。
  subscription_id?: string;
  subscription_name?: string;
  subscription_login?: boolean;
  messages?: AgentChatMessage[];
  cols?: number;
  rows?: number;
  log_path?: string;
  jsonl_path?: string;
  replay_b64?: string;
  // reattach 時に wrapper が申告する PTY 読み出し累計バイト数（wrapper→Hub 専用。
  // UI は受け取らない）。Hub が「切断中に取りこぼしたぶん」を算出するのに使う。
  pty_bytes?: number;
  replay?: boolean;
  replay_epoch?: number;
  approval_source_epoch?: number;
  reason?: string;
  approval_visible?: boolean;
  approval_sig?: string;
  approval_kind?: string;
  approval_source?: string;
  approval_question?: string;
  approval_context?: string;
  approval_options?: ApprovalOption[];
  approval_summary?: ApprovalSummary;
  approval_candidate_key?: string;
  approval_candidate_shape?: string;
  approval_consumed?: boolean;
  approval_consumed_epoch?: number;
  /** type='approval_state' でだけ届く。 */
  approval_state?: ApprovalState;
  /** type='approval_snapshot' でだけ届く（接続した画面にだけ送られる）。 */
  approval_snapshot?: ApprovalSessionState[];
  done_summary?: DoneSummary;
  turn?: number;
  files_changed?: number;
  added?: number;
  removed?: number;
  ended_at?: string;
  block?: string;
  sent_text?: string;
  commit_subject?: string;
  commit_body?: string;
  detected_at?: string;
  last_output_at?: string;
  /** provider 自身の transcript が最後に伸びた時刻（RFC 3339）。停滞判定用。 */
  transcript_grew_at?: string;
  started_at?: string;
  label?: string;
	 session_meta?: SessionMeta;
  model?: string;
  /** reasoning effort（"high" 等）。起動バナー / モデル変更行から Hub が検出した値。 */
  effort?: string;
  /** 起動要求に添えられた実行モード（auto / interactive / headless）。空は指定なし。 */
  execution_mode?: string;
  /** 起動要求に添えられた権限段（attended / bounded / full）。空は指定なし。 */
  permission_preset?: string;
  /**
   * wrapper が申告した「起動時に実際に付けた権限モード」（plan / acceptEdits /
   * dontAsk / auto / bypassPermissions / bounded）。空は「権限のフラグを 1 つも
   * 付けていない」。**起動時の 1 点の値**で、セッション中の切り替えは含まない。
   * 段（permission_preset）とは語彙が別。
   */
  permission_mode?: string;
  route?: string;
  parent_session_id?: number;
  /** このセッションが続きを引き受けた前任の ID。0 / 未送は通常起動。親子関係ではない。 */
  handoff_from?: number;
  auto?: boolean;
  depth?: number;
  orchestration_id?: string;
  board_path?: string;
  worktree_branch?: string;
	board_notify_pending?: boolean;
	/** 親セッションが回している relay ループ（開始順）。 */
	relays?: RelayStatus[];
	/** board 外の Claude 間通信を Hub が受信側 PTY で観測した履歴。 */
	cross_session_messages?: CrossSessionMessage[];
	spawn_confirmation_id?: string;
	initial_prompt?: string;
	/** epoch ms。spawn_confirmation_requested とその再送の両方に載る。 */
	spawn_requested_at_ms?: number;
	/**
	 * 確認ダイアログの「この役割では次回もこの段を使う」チェックボックスの初期状態。
	 * true = Hub がその役割の段を既に覚えている（段そのものは permission_preset）。
	 */
	remember_permission?: boolean;
	/**
	 * spawn_confirmation_closed の reason は5値のみ:
	 * approved | refused | superseded | parent_gone | spawn_failed。
	 * spawn_failed のときだけ text に失敗理由が入る。session_id は親。
	 */
	spawn_child_session_id?: number;
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
  claude_rate_limits_present?: boolean;
  claude_5h_field_present?: boolean;
  claude_5h_present?: boolean;
  claude_7d_field_present?: boolean;
  claude_7d_present?: boolean;
  codex_rate_limits_present?: boolean;
  codex_primary_present?: boolean;
  codex_primary_used_pct?: number;
  codex_primary_window_minutes?: number;
  codex_primary_reset?: number;
  codex_secondary_used_pct?: number;
  codex_secondary_present?: boolean;
  codex_secondary_window_minutes?: number;
  codex_secondary_reset?: number;
  codex_credits_present?: boolean;
  codex_has_credits?: boolean;
  codex_credits_unlimited?: boolean;
  codex_credits_balance?: string;
  codex_plan_type?: string;
  usage_observed_at?: string;
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
  // handoff_note: 残量の帯から依頼した引き継ぎメモの結果（子 plan:
  // plan_derived-session-launch_c4_handoff-routes.md 内部 C2）。note_ok=false は
  // 「60 秒内に合図が来なかった / ファイルが無い」で、その場合 note_path は空。
  note_ok?: boolean;
  note_path?: string;
  // C3: git 変更状況メタ（git_checked=true のメッセージのみ有効）
  git_checked?: boolean;
  git_files?: number;
  git_added?: number;
  git_deleted?: number;
  // binary_stale: 稼働中 Hub の実行ファイルがディスク上で差し替わったか
  // （= 再ビルドが未反映）。type='binary_stale' のメッセージでのみ届く。
  binary_stale?: boolean;
  [key: string]: unknown;
}

export interface SessionSnapshot {
  id: number;
  provider?: ProviderID;
  display_name?: string;
  cwd?: string;
  /** cwd の末尾セグメントから導いた表示用のキー（state.ts の deriveProjectKeyFromCwd）。 */
  project?: string;
  /** cwd が属する本体リポジトリのルート。サイドバーの箱はこちらを優先して使う。 */
  project_id?: string;
  branch?: string;
  label?: string;
	 pinned?: boolean;
	 color?: string;
	 note?: string;
	 auto_title?: string;
  model?: string;
  /** reasoning effort（"high" 等）。UI 3 箇所の統一表示に使う（usage 側の値が優先）。 */
  effort?: string;
  /** wrapper が申告した実行モード（"headless" のみ。空＝対話）。 */
  execution_mode?: string;
  /**
   * wrapper が申告した「起動時に実際に付けた権限モード」。空は指定なしで、
   * カードには何も出さない（「不明」を出さない）。ライブ値ではない。
   */
  permission_mode?: string;
  route?: string;
  shell?: string;
  parent_session_id?: number;
  /** このセッションが続きを引き受けた前任の ID。0 / 未送は通常起動。親子関係ではない。 */
  handoff_from?: number;
  role?: string;
  auto?: boolean;
  depth?: number;
  orchestration_id?: string;
  board_path?: string;
  worktree_branch?: string;
	board_notify_pending?: boolean;
  /** 親セッションが回している relay ループ（開始順）。 */
  relays?: RelayStatus[];
  /** board 外の Claude 間通信を Hub が受信側 PTY で観測した履歴。 */
  cross_session_messages?: CrossSessionMessage[];
  // 起動に使ったサブスクリプション profile。空/未送は CLI 自身のログイン環境。
  subscription_profile_id?: string;
  subscription_profile_name?: string;
  state?: SessionState;
	activity?: SessionActivity;
	output_idle?: boolean;
	workflow_active?: boolean;
	awaiting_user?: boolean;
	awaiting_approval?: boolean;
  last_output_at?: string;
  /** provider 自身の transcript が最後に伸びた時刻（RFC 3339）。停滞判定用。 */
  transcript_grew_at?: string;
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

