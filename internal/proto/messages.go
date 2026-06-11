package proto

// Message は Hub・Wrapper・UI 間で交わす WebSocket メッセージ。
//
// 状態モデルは「PTY 出力の有無」だけで決まる 4 状態:
//
//	standby   : wrapper 接続済み・初回 PTY 出力前
//	running   : 直近 IdleAfter 内に PTY 出力あり
//	waiting   : 出力静止 ≥ IdleAfter（承認待ち / プロンプト待ち / 通常停止 を区別しない）
//	completed : プロセス終了
//	error / disconnected : 異常終了 / 接続断
type Message struct {
	Type      string `json:"type"`
	Role      string `json:"role,omitempty"`
	SessionID int    `json:"session_id,omitempty"`
	Provider  string `json:"provider,omitempty"`
	Display   string `json:"display_name,omitempty"`
	CWD       string `json:"cwd,omitempty"`
	Branch    string `json:"branch,omitempty"`
	PID       int    `json:"pid,omitempty"`
	Shell     string `json:"shell,omitempty"`
	Version   string `json:"version,omitempty"`
	State     string `json:"state,omitempty"`
	ExitCode  int    `json:"exit_code,omitempty"`
	Token     string `json:"token,omitempty"`
	Data      []byte `json:"data,omitempty"` // wrapper内部用: PTY生バイト列（base64エンコード）
	Text      string `json:"text,omitempty"` // pty_output: ANSIを除去したプレーンテキスト / pty_input: ユーザー入力文字列
	Cols      int    `json:"cols,omitempty"` // pty_resize / register / registered
	Rows      int    `json:"rows,omitempty"` // pty_resize / register / registered

	// reattach: wrapper が Hub クラッシュ後に元セッション情報を復元するための情報。
	LogPath   string `json:"log_path,omitempty"`
	JSONLPath string `json:"jsonl_path,omitempty"`
	ReplayB64 string `json:"replay_b64,omitempty"`
	Reason    string `json:"reason,omitempty"`

	// TokenStatusbar: registered ack で Hub が返す「トークン常時表示バーが有効か」。
	// wrapper はこれを見て claude 起動時に --settings で statusLine を渡すか決める
	// （共有の .claude/settings.local.json は一切書き換えない方式）。
	TokenStatusbar bool `json:"token_statusbar,omitempty"`

	// session_hint で UI 側から送る「承認 UI が可視」フラグ。
	ApprovalVisible bool `json:"approval_visible,omitempty"`

	// approval_detected / approval_cleared / approval_consumed:
	// Go 側 VT バッファから検出した native approval prompt の通知と、
	// UI 側で回答済みになった prompt の再検出抑止に使う。
	ApprovalSig      string           `json:"approval_sig,omitempty"`
	ApprovalKind     string           `json:"approval_kind,omitempty"`
	ApprovalSource   string           `json:"approval_source,omitempty"`
	ApprovalQuestion string           `json:"approval_question,omitempty"`
	ApprovalContext  string           `json:"approval_context,omitempty"`
	ApprovalOptions  []ApprovalOption `json:"approval_options,omitempty"`
	SentText         string           `json:"sent_text,omitempty"`
	DetectedAt       string           `json:"detected_at,omitempty"`

	// LastOutputAt: PTY 出力が最後に届いた時刻（ISO 8601 / RFC 3339）。
	// session_update で standby/waiting 遷移時に付与し、UI カードに「最終応答時刻」として表示する。
	LastOutputAt string `json:"last_output_at,omitempty"`

	// StartedAt: セッション登録時刻（ISO 8601 / RFC 3339）。UI カードに起動時刻として表示する。
	StartedAt string `json:"started_at,omitempty"`

	// Label: セッション識別用の任意ラベル（UI カード 3 行目に【ラベル】として表示）。
	Label string `json:"label,omitempty"`

	// Model: 使用モデル名（例: "claude-sonnet-4-5", "gpt-5.5"）。UI カードに表示する。
	Model string `json:"model,omitempty"`

	// Route: spawn 時に明示された接続経路（"anthropic" / "openai" / "ollama"）。
	// env preset 注入に使う。未指定なら model 名から推定する。
	Route string `json:"route,omitempty"`

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

	// usage_stat: Hub → UI。セッション単位の累積トークン / コスト情報。
	// 数値メタデータのみを持ち、プロンプト本文などは一切含まない。
	// CostKnown が false の場合はコストが不明（価格表未登録モデル）。表示側は "$ —" とする。
	CostUSD        float64 `json:"cost_usd,omitempty"`
	CostKnown      bool    `json:"cost_known,omitempty"`
	TokensIn       int     `json:"tokens_in,omitempty"`
	TokensOut      int     `json:"tokens_out,omitempty"`
	TokensCache    int     `json:"tokens_cache,omitempty"`
	TokensTotal    int     `json:"tokens_total,omitempty"`
	UsageModel     string  `json:"usage_model,omitempty"`
	UsageStartedAt string  `json:"usage_started_at,omitempty"`
}

type ApprovalOption struct {
	Num           int    `json:"num"`
	Label         string `json:"label,omitempty"`
	IsCurrent     bool   `json:"is_current,omitempty"`
	SendText      string `json:"send_text,omitempty"`
	PreserveOrder bool   `json:"preserve_order,omitempty"`
}
