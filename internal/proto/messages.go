package proto

// TypeSessionDismissed は Hub → wrapper の「このセッションは意図的に閉じられた」通知。
// wrapper が WS の EOF から意図を推定すると postReattachGuard(10s) 以内では
// 「回線不調」に倒れ、2 秒後に再接続してセッションが復活してしまう
// （bugfix_session-dismiss-ignored-within-reattach-guard_2026-08-13.md）。
//
// 他のメッセージ種別と違って文字列リテラルを直書きせず定数にしているのは、送信側
// （internal/hub）と受信側（internal/wrapper）が別パッケージで、綴りがずれても
// エラーにならず「静かに旧挙動へ戻る」だけだから。症状が復活しても原因に辿り着けない。
const TypeSessionDismissed = "session_dismissed"

// Message は Hub・Wrapper・UI 間で交わす WebSocket メッセージ。
//
// 状態モデルは output_idle / workflow_active / awaiting_user の 3 軸を正本とし、
// State は旧クライアント互換の表示ラベルとしてのみ残す。
type Message struct {
	Type      string `json:"type"`
	Role      string `json:"role,omitempty"`
	SessionID int    `json:"session_id,omitempty"`
	Provider  string `json:"provider,omitempty"`
	Display   string `json:"display_name,omitempty"`
	CWD       string `json:"cwd,omitempty"`
	Branch    string `json:"branch,omitempty"`
	PID       int    `json:"pid,omitempty"`
	// InputSeq identifies a Hub-to-wrapper pty_input frame so the wrapper can
	// acknowledge the frame after the bytes have been written to the PTY.
	InputSeq int64  `json:"input_seq,omitempty"`
	Shell    string `json:"shell,omitempty"`
	Version  string `json:"version,omitempty"`
	State    string `json:"state,omitempty"`
	// Three orthogonal session activity signals. State remains a compatibility
	// display label; consumers that need a safe interruption point use
	// output_idle && !workflow_active.
	OutputIdle       bool `json:"output_idle,omitempty"`
	WorkflowActive   bool `json:"workflow_active,omitempty"`
	AwaitingUser     bool `json:"awaiting_user,omitempty"`
	AwaitingApproval bool `json:"awaiting_approval,omitempty"`
	// Activity carries all four flags atomically, including false transitions.
	Activity         *SessionActivity  `json:"activity,omitempty"`
	WorkflowProgress *WorkflowProgress `json:"workflow_progress,omitempty"`
	ExitCode         int               `json:"exit_code,omitempty"`
	// Signal carries the POSIX signal name (e.g. "killed", "terminated") when
	// session_end's process was terminated by a signal rather than exiting
	// normally with a non-zero code. Unix-only; always empty on Windows.
	Signal    string `json:"signal,omitempty"`
	Token     string `json:"token,omitempty"`
	HomeDir   string `json:"home_dir,omitempty"`
	CodexHome string `json:"codex_home,omitempty"`
	ClaudeDir string `json:"claude_dir,omitempty"`
	// AgentSessionID is the provider-owned transcript ID. It is sent by the
	// wrapper during register/reattach and is never rendered in the session card.
	AgentSessionID string `json:"agent_session_id,omitempty"`
	// SubscriptionID names the subscription profile this session was launched
	// with. The wrapper reports it back from its environment so the Hub records
	// what actually ran, not what it intended to run. Empty means "the CLI's own
	// login environment", which is the default and the only pre-v0.8 behaviour.
	// It is an opaque id, never a credential.
	SubscriptionID string `json:"subscription_id,omitempty"`
	// SubscriptionName is the display label the Hub resolves from config for
	// SubscriptionID. Hub → UI only.
	SubscriptionName  string             `json:"subscription_name,omitempty"`
	Data              []byte             `json:"data,omitempty"`     // wrapper内部用: PTY生バイト列（base64エンコード）
	Text              string             `json:"text,omitempty"`     // pty_output: ANSIを除去したプレーンテキスト / pty_input: ユーザー入力文字列
	AgentChatMessages []AgentChatMessage `json:"messages,omitempty"` // agent_chat: structured transcript messages
	Cols              int                `json:"cols,omitempty"`     // pty_resize / register / registered
	Rows              int                `json:"rows,omitempty"`     // pty_resize / register / registered

	// reattach: wrapper が Hub クラッシュ後に元セッション情報を復元するための情報。
	LogPath   string `json:"log_path,omitempty"`
	JSONLPath string `json:"jsonl_path,omitempty"`
	ReplayB64 string `json:"replay_b64,omitempty"`
	Reason    string `json:"reason,omitempty"`
	// PTYBytes: reattach 時に wrapper が申告する「PTY から読み出した累計バイト数」。
	// Hub 側の受信済み累計との差が「切断中に取りこぼしたバイト数」になり、
	// ReplayB64 の末尾からその長さだけを切り出して既存 UI へ配信する。
	// ReplayB64 全体を送ると切断前に表示済みの内容が二重描画されるため、
	// 差分の算出にこの値が要る（docs/local/archive/v0.4.0/
	// bugfix_codex-terminal-reconnect-replay-duplication_2026-07-06.md）。
	// 0 は「古い wrapper で未申告」を意味し、Hub は再配信を行わない。
	PTYBytes int64 `json:"pty_bytes,omitempty"`

	// Replay marks pty_data that restores already-produced terminal output. The
	// field is optional so older wrappers and UIs continue to treat ordinary
	// pty_data frames as before. ReplayEpoch changes for each independent
	// terminal restoration, while ApprovalSourceEpoch identifies the logical
	// prompt generation used by approval deduplication.
	Replay              bool   `json:"replay,omitempty"`
	ReplayEpoch         uint64 `json:"replay_epoch,omitempty"`
	ApprovalSourceEpoch uint64 `json:"approval_source_epoch,omitempty"`

	// TokenStatusbar: registered ack で Hub が返す「トークン常時表示バーが有効か」。
	// wrapper はこれを見て claude 起動時に --settings で statusLine を渡すか決める
	// （共有の .claude/settings.local.json は一切書き換えない方式）。
	// omitempty を付けない: false が wire から消えると「意図的 OFF」と「フィールド欠落/
	// 別 type のメッセージを受信」が区別できず、statusline 欠落の診断が潰れる
	// （docs/local/archive/v0.5.x/bugfix_statusline-settings-skip_2026-07-10.md）。
	TokenStatusbar bool `json:"token_statusbar"`

	// session_hint で UI 側から送る「承認 UI が可視」フラグ。
	ApprovalVisible bool `json:"approval_visible,omitempty"`

	// approval_detected / approval_cleared / approval_consumed:
	// Go 側 VT バッファから検出した native approval prompt の通知と、
	// UI 側で回答済みになった prompt の再検出抑止に使う。
	//
	// approval_marker_suppressed:
	// 構造が壊れた [MANY-AI-CLI] ブロックを配信せず捨てたことの告知。
	// Reason に classifyApprovalMarkerBlock の分類（marker_leak / option_start /
	// duplicate_option / box_rule）が入る。Block は壊れているため送らない。
	ApprovalSig      string           `json:"approval_sig,omitempty"`
	ApprovalKind     string           `json:"approval_kind,omitempty"`
	ApprovalSource   string           `json:"approval_source,omitempty"`
	ApprovalQuestion string           `json:"approval_question,omitempty"`
	ApprovalContext  string           `json:"approval_context,omitempty"`
	ApprovalOptions  []ApprovalOption `json:"approval_options,omitempty"`
	ApprovalSummary  *ApprovalSummary `json:"approval_summary,omitempty"`
	// ApprovalCandidateKey is a stable, presentation-independent identity for
	// one approval candidate. It deliberately excludes mutable context such as
	// status lines, borders, and option labels that can change during TUI reflow.
	ApprovalCandidateKey string `json:"approval_candidate_key,omitempty"`
	// ApprovalCandidateShape is sent only when replay needs to restore the
	// browser's answered-candidate suppression after a UI reconnect. It is the
	// normalized, label-free shape behind ApprovalCandidateKey.
	ApprovalCandidateShape string `json:"approval_candidate_shape,omitempty"`
	ApprovalConsumed       bool   `json:"approval_consumed,omitempty"`
	ApprovalConsumedEpoch  uint64 `json:"approval_consumed_epoch,omitempty"`
	// DoneSummary is emitted when an AI task reaches a terminal-looking state.
	// It is display-only and never authorizes an action.
	DoneSummary *DoneSummary `json:"done_summary,omitempty"`
	Block       string       `json:"block,omitempty"` // approval_marker: VT tail から抽出した [MANY-AI-CLI] ブロック全文
	SentText    string       `json:"sent_text,omitempty"`
	DetectedAt  string       `json:"detected_at,omitempty"`

	// LastOutputAt: PTY 出力が最後に届いた時刻（ISO 8601 / RFC 3339）。
	// session_update で standby/waiting 遷移時に付与し、UI カードに「最終応答時刻」として表示する。
	LastOutputAt string `json:"last_output_at,omitempty"`

	// TranscriptGrewAt: provider 自身の transcript（Codex の rollout JSONL 等）が
	// 最後に伸びた時刻（ISO 8601 / RFC 3339）。空は「まだ特定できていない」。
	// UI はこの時刻からの経過を 1Hz で自分で計算する（毎秒 broadcast しないため）。
	//
	// LastOutputAt では代用できない。Codex TUI は "Working (36m 09s)" のカウンタを
	// 毎秒再描画するので PTY 出力は途切れず、モデルが 1 個の出力を吐き続けている間も
	// LastOutputAt は更新され続ける。transcript はターンが進んだときだけ伸びるので、
	// 伸びていない時間がそのまま「同じ応答を生成し続けている時間」になる。
	// 由来: docs/local/bugfix_codex-long-silence-not-surfaced_2026-08-19.md
	TranscriptGrewAt string `json:"transcript_grew_at,omitempty"`

	// StartedAt: セッション登録時刻（ISO 8601 / RFC 3339）。UI カードに起動時刻として表示する。
	StartedAt string `json:"started_at,omitempty"`

	// Label: セッション識別用の任意ラベル（UI カード 3 行目に【ラベル】として表示）。
	Label string `json:"label,omitempty"`

	// SessionMeta はカード識別用の永続メタデータ。label を空文字へ戻す更新も
	// 確実に伝えるため、session_update ではこの入れ子オブジェクトで送る。
	SessionMeta *SessionMeta `json:"session_meta,omitempty"`

	// Model: 使用モデル名（例: "claude-sonnet-4-5", "gpt-5.5"）。UI カードに表示する。
	Model string `json:"model,omitempty"`

	// Route: spawn 時に明示された接続経路（"anthropic" / "openai" / "ollama"）。
	// env preset 注入に使う。未指定なら model 名から推定する。
	Route string `json:"route,omitempty"`

	// Lightweight orchestration metadata.
	ParentSessionID    int    `json:"parent_session_id,omitempty"`
	Auto               bool   `json:"auto,omitempty"`
	Depth              int    `json:"depth,omitempty"`
	OrchestrationID    string `json:"orchestration_id,omitempty"`
	BoardPath          string `json:"board_path,omitempty"`
	WorktreeBranch     string `json:"worktree_branch,omitempty"`
	BoardNotifyPending bool   `json:"board_notify_pending,omitempty"`
	// spawn_confirmation_requested is sent to browser UIs before an
	// orchestration child is created. The response travels by HTTP, never via
	// the conductor PTY, so the user remains the authority for the decision.
	SpawnConfirmationID string `json:"spawn_confirmation_id,omitempty"`
	InitialPrompt       string `json:"initial_prompt,omitempty"`

	// FirstMessage: セッション内で最初に確定されたユーザー入力（UI カード表示用）。
	FirstMessage string `json:"first_message,omitempty"`

	// LastMessage: セッション内で最後に確定されたユーザー入力（UI カード表示用）。
	LastMessage string `json:"last_message,omitempty"`

	// Inject: attach_file (deprecated) で使用していた PTY 注入文字列。
	//
	// Deprecated: 現行の attach フローは Hub 側で attach.Save → PTY へ直接 inject する
	// 経路に一本化済みで、この field を読む生きた経路は存在しない。旧バージョンの
	// wrapper から register/reattach 時に送られてきても無害に無視できるよう、
	// proto 互換性のためだけに残置している。新規コードからは参照しないこと。
	// 互換ウィンドウ経過後（旧 wrapper が出回らなくなった時点）に削除予定。
	Inject string `json:"inject,omitempty"`

	// attach_request: UI → Hub。ファイルバイナリを base64 エンコードした文字列。
	ImageData string `json:"image_data,omitempty"`
	Filename  string `json:"filename,omitempty"` // 元ファイル名（拡張子の決定に使用）

	// approval_patterns_updated: Hub → UI。リモート fetch で公式パターンに差分があった
	// 場合に通知する。Providers には差分があった provider 名のみが入る。
	Providers []string `json:"providers,omitempty"`

	// UIActiveSessionID: UI register 時に UI 側が現在表示中のセッション ID を伝える。
	// Hub は replay 時にアクティブセッションは全量、非アクティブは末尾に絞って送信する。
	// 0 の場合はアクティブセッション不明として扱う。
	UIActiveSessionID int `json:"ui_active_session_id,omitempty"`

	// git_stat: Hub → UI。セッション cwd の Git 変更統計。
	// GitChecked が true のメッセージでのみ git 統計が含まれる。
	// git 未インストール / 非 git ディレクトリの場合は 0 が入る。
	GitChecked bool `json:"git_checked,omitempty"` // このメッセージが git 統計を含むことを示すフラグ
	GitFiles   int  `json:"git_files,omitempty"`   // 変更ファイル数（git status --porcelain の行数）
	GitAdded   int  `json:"git_added,omitempty"`   // 追加行数
	GitDeleted int  `json:"git_deleted,omitempty"` // 削除行数

	// commit_msg_suggested / commit_msg_error: Hub → UI。Git タブ「Ask AI」で
	// 接続中の AI が生成したコミットメッセージ。CommitSubject が 1 行目、
	// CommitBody が本文。エラー時は commit_msg_error + Reason を返す。
	CommitSubject string `json:"commit_subject,omitempty"`
	CommitBody    string `json:"commit_body,omitempty"`

	// usage_stat: Hub → UI。セッション単位の累積トークン / コスト情報。
	// 数値メタデータのみを持ち、プロンプト本文などは一切含まない。
	// CostKnown が false の場合はコストが不明（価格表未登録モデル）。表示側は "$ —" とする。
	CostUSD        float64 `json:"cost_usd,omitempty"`
	CostKnown      bool    `json:"cost_known,omitempty"`
	TokensIn       int     `json:"tokens_in,omitempty"`
	TokensOut      int     `json:"tokens_out,omitempty"`
	TokensCache    int     `json:"tokens_cache,omitempty"`
	TokensTotal    int     `json:"tokens_total,omitempty"`
	CtxWindow      int     `json:"ctx_window,omitempty"`   // モデルのコンテキストウィンドウ上限（不明なら省略）
	CtxUsedPct     float64 `json:"ctx_used_pct,omitempty"` // Claude statusLine 算出済みの context 使用率%（0=未取得・Claude のみ）
	UsageModel     string  `json:"usage_model,omitempty"`
	UsageStartedAt string  `json:"usage_started_at,omitempty"`

	// statusbar 追加メタ（Claude statusLine ネイティブ算出値。Claude のみ・C2 relay 中継）。
	// rate_limits は Pro/Max のみ／early-session は 0。lines は AI 編集量（作業ツリー git diff とは別軸）。
	RateLimit5hPct              float64 `json:"rl_5h_pct,omitempty"`
	RateLimit5hReset            int64   `json:"rl_5h_reset,omitempty"`
	RateLimit7dPct              float64 `json:"rl_7d_pct,omitempty"`
	RateLimit7dReset            int64   `json:"rl_7d_reset,omitempty"`
	CodexRateLimitsPresent      bool    `json:"codex_rate_limits_present,omitempty"`
	CodexPrimaryUsedPct         float64 `json:"codex_primary_used_pct,omitempty"`
	CodexPrimaryWindowMinutes   int     `json:"codex_primary_window_minutes,omitempty"`
	CodexPrimaryReset           int64   `json:"codex_primary_reset,omitempty"`
	CodexSecondaryUsedPct       float64 `json:"codex_secondary_used_pct,omitempty"`
	CodexSecondaryWindowMinutes int     `json:"codex_secondary_window_minutes,omitempty"`
	CodexSecondaryReset         int64   `json:"codex_secondary_reset,omitempty"`
	CodexCreditsBalance         string  `json:"codex_credits_balance,omitempty"`
	CodexPlanType               string  `json:"codex_plan_type,omitempty"`
	LinesAdded                  int     `json:"lines_added,omitempty"`
	LinesRemoved                int     `json:"lines_removed,omitempty"`
	EffortLevel                 string  `json:"effort_level,omitempty"`
	Thinking                    bool    `json:"thinking,omitempty"`
	Exceeds200k                 bool    `json:"exceeds_200k,omitempty"`
	DurationMs                  int64   `json:"duration_ms,omitempty"`
	APIDurationMs               int64   `json:"api_duration_ms,omitempty"`
	OutputStyle                 string  `json:"output_style,omitempty"`
	VimMode                     string  `json:"vim_mode,omitempty"`
	AgentName                   string  `json:"agent_name,omitempty"`
	RepoHost                    string  `json:"repo_host,omitempty"`
	RepoOwner                   string  `json:"repo_owner,omitempty"`
	RepoName                    string  `json:"repo_name,omitempty"`
	RemainingPct                float64 `json:"remaining_pct,omitempty"`
	ReasoningOut                int     `json:"reasoning_output_tokens,omitempty"`

	// binary_stale: Hub → UI。稼働中 Hub の実行ファイルがディスク上で差し替わった
	// （= 再ビルドが反映されていない）状態かどうか。状態が変化した瞬間だけ配信する。
	// ポインタなのは false を確実に届けるため: omitempty で false が消えると
	// 「stale から復帰した」を伝えられず、バナーが出たまま固着する。
	BinaryStale *bool `json:"binary_stale,omitempty"`
}

// AgentChatMessage is the provider-neutral structured transcript payload used
// by the agent_chat API and WebSocket message.
type AgentChatMessage struct {
	Role      string          `json:"role"`
	Kind      string          `json:"kind,omitempty"`
	Text      string          `json:"text,omitempty"`
	Thinking  []string        `json:"thinking,omitempty"`
	Tools     []AgentChatTool `json:"tools,omitempty"`
	TS        string          `json:"ts,omitempty"`
	MessageID string          `json:"message_id,omitempty"`
}

type AgentChatTool struct {
	ID     string `json:"id,omitempty"`
	Name   string `json:"name"`
	Input  string `json:"input,omitempty"`
	Result string `json:"result,omitempty"`
}

// SessionActivity is the wire representation of a session's activity axes.
type SessionActivity struct {
	OutputIdle       bool `json:"output_idle"`
	WorkflowActive   bool `json:"workflow_active"`
	AwaitingUser     bool `json:"awaiting_user"`
	AwaitingApproval bool `json:"awaiting_approval"`
}

// WorkflowProgress is the Hub-authoritative workflow progress snapshot sent to
// browser UIs. Source and SettledBy identify which observation established the
// current values; tree details are present only when the VT tree is visible.
type WorkflowProgress struct {
	Detected         bool      `json:"detected"`
	Source           string    `json:"source,omitempty"`
	Name             string    `json:"name,omitempty"`
	Done             int       `json:"done"`
	Total            int       `json:"total"`
	Running          int       `json:"running"`
	Failed           int       `json:"failed"`
	Pending          int       `json:"pending"`
	WaitingDynamic   int       `json:"waiting_dynamic"`
	Percent          int       `json:"percent"`
	ElapsedSec       int       `json:"elapsed_sec,omitempty"`
	TokensRaw        string    `json:"tokens_raw,omitempty"`
	Phases           []WfPhase `json:"phases,omitempty"`
	Settled          bool      `json:"settled"`
	SettledBy        string    `json:"settled_by,omitempty"`
	TaskDetailSource string    `json:"task_detail_source,omitempty"`
}

type WfPhase struct {
	Title  string    `json:"title"`
	Agents []WfAgent `json:"agents"`
}

type WfAgent struct {
	Label   string         `json:"label"`
	State   string         `json:"state"`
	Metrics string         `json:"metrics,omitempty"`
	Detail  *WfAgentDetail `json:"detail,omitempty"`
}

// WfAgentDetail is populated only when the Hub could resolve a task ID for
// the running Workflow (see docs/local/plan_workflow-progress-agent-transcript-detail_c1_investigation-proto.md
// "発見2"). It augments the existing VT/journal-derived WfAgent with data
// read from the Claude Code task output file. Fields sourced from
// promptPreview/resultPreview/lastToolSummary carry excerpted user/agent
// content and must never be logged, persisted, or forwarded outside the
// per-session WS payload.
type WfAgentDetail struct {
	Model           string `json:"model,omitempty"`
	StartedAt       int64  `json:"started_at,omitempty"`       // epoch ms
	LastProgressAt  int64  `json:"last_progress_at,omitempty"` // epoch ms
	DurationMs      int64  `json:"duration_ms,omitempty"`
	Tokens          int    `json:"tokens,omitempty"`
	ToolCalls       int    `json:"tool_calls,omitempty"`
	LastToolName    string `json:"last_tool_name,omitempty"`
	LastToolSummary string `json:"last_tool_summary,omitempty"` // truncated, see C2 budget
	PromptPreview   string `json:"prompt_preview,omitempty"`    // truncated
	ResultPreview   string `json:"result_preview,omitempty"`    // truncated
}

// SessionMeta is user-editable, server-persisted identification metadata for a
// live session. AutoTitle is derived from the first user input and is displayed
// only when Label is empty.
type SessionMeta struct {
	Label     string `json:"label"`
	Pinned    bool   `json:"pinned"`
	Color     string `json:"color"`
	Note      string `json:"note"`
	AutoTitle string `json:"auto_title"`
}

// ApprovalRiskTier is the stable, provider-neutral risk contract shared by
// approval UI, auto-approval policy, and outbound notifications.
type ApprovalRiskTier string

const (
	ApprovalRiskLow  ApprovalRiskTier = "low"
	ApprovalRiskMid  ApprovalRiskTier = "mid"
	ApprovalRiskHigh ApprovalRiskTier = "high"
)

// ApprovalSummary is derived from ANSI-stripped VT text. Raw is supplied for
// disclosure only; consumers must not use it to grant an approval.
type ApprovalSummary struct {
	Command string           `json:"command,omitempty"`
	Paths   []string         `json:"paths,omitempty"`
	Risk    ApprovalRiskTier `json:"risk"`
	Raw     string           `json:"raw,omitempty"`
}

// DoneSummary is a normalized, secret-masked terminal summary. Kind is one of
// success, failure, aborted, or needs_action; Fallback marks an idle-derived
// summary created when the provider omitted the DONE marker.
type DoneSummary struct {
	SessionID int    `json:"session_id"`
	Provider  string `json:"provider,omitempty"`
	Title     string `json:"title,omitempty"`
	Text      string `json:"text"`
	Kind      string `json:"kind"`
	At        string `json:"at"`
	Fallback  bool   `json:"fallback,omitempty"`
}

type ApprovalOption struct {
	Num           int    `json:"num"`
	Label         string `json:"label,omitempty"`
	IsCurrent     bool   `json:"is_current,omitempty"`
	SendText      string `json:"send_text,omitempty"`
	PreserveOrder bool   `json:"preserve_order,omitempty"`
}
