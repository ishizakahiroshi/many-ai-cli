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

	// session_hint で UI 側から送る「承認 UI が可視」フラグ。
	ApprovalVisible bool `json:"approval_visible,omitempty"`

	// LastOutputAt: PTY 出力が最後に届いた時刻（ISO 8601 / RFC 3339）。
	// session_update で standby/waiting 遷移時に付与し、UI カードに「最終応答時刻」として表示する。
	LastOutputAt string `json:"last_output_at,omitempty"`

	// StartedAt: セッション登録時刻（ISO 8601 / RFC 3339）。UI カードに起動時刻として表示する。
	StartedAt string `json:"started_at,omitempty"`

	// Label: セッション識別用の任意ラベル（UI カード 3 行目に【ラベル】として表示）。
	Label string `json:"label,omitempty"`

	// Model: 使用モデル名（例: "claude-sonnet-4-5", "gpt-4o"）。UI カードに表示する。
	Model string `json:"model,omitempty"`

	// FirstMessage: セッション内で最初に確定されたユーザー入力（UI カード表示用）。
	FirstMessage string `json:"first_message,omitempty"`

	// LastMessage: セッション内で最後に確定されたユーザー入力（UI カード表示用）。
	LastMessage string `json:"last_message,omitempty"`

	// attach_file: Hub → wrapper。保存済み画像の絶対パスと PTY 注入文字列。
	Path   string `json:"path,omitempty"`
	Inject string `json:"inject,omitempty"`

	// attach_request: UI → Hub。ファイルバイナリを base64 エンコードした文字列。
	ImageData string `json:"image_data,omitempty"`
	Filename  string `json:"filename,omitempty"` // 元ファイル名（拡張子の決定に使用）
}
