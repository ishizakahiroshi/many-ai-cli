package wrapper

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"
)

// codexConfigMu は同一プロセス内の ~/.codex/config.toml RMW を直列化する。
// クロスプロセスは codexConfigFileLock で保護する。
var codexConfigMu sync.Mutex

// ---------------------------------------------------------------------------
// 共通定数・型
// ---------------------------------------------------------------------------

const (
	usageHookBlockStart = "# any-ai-cli:usage-hook-start"
	usageHookBlockEnd   = "# any-ai-cli:usage-hook-end"
)

// UsageHookParams は注入時に埋め込む接続パラメータ。
type UsageHookParams struct {
	HubURL    string
	Token     string
	SessionID int
	ExePath   string // many-ai-cli バイナリのフルパス（os.Executable() で解決済み）
}

// ---------------------------------------------------------------------------
// Claude: wrapper 所有の一時 settings ファイル経由で statusLine を渡す
// ---------------------------------------------------------------------------
//
// 共有ファイル .claude/settings.local.json には一切書き込まない（重要）。
// Claude Code 本体も権限承認のたびに同ファイルを書き換え、ユーザーも手編集する
// ため、後付け注入は衝突・手編集ミスで壊れ続ける（過去に単一バックスラッシュの
// Windows パスで JSON 全体が不正化し、Claude が settings を読めず statusLine が
// 無効化された。さらに旧実装は破損ファイルを安全のためスキップする設計だったので
// 一度壊れると永久に再注入できず「トークンが流れてこない」状態に固定された）。
//
// 代わりに claude 起動時に `claude --settings <temp>` を渡す。--settings は設定
// 階層のうちコマンドライン引数（local/project/user より上・managed の下）として
// マージされるため、temp に statusLine だけ書けば有効になる。temp は wrapper だけ
// が所有し起動ごとに作り直すので、共有衝突が原理的に発生せず、万一壊れても次回
// 起動で上書きされる。

// toShellPath は exe パスを Windows シェルでも安全に実行できる形へ変換する。
//
// Claude Code は Windows で statusLine コマンドを Git Bash（無ければ PowerShell）
// 経由で実行する。Git Bash はバックスラッシュをエスケープ文字として扱うため、
// `C:\dev\foo\many-ai-cli.exe` のような生の Windows パスはセパレータが消えて
// パスが壊れ、コマンドが「エラーも出さずに実行されない」（＝relay が一度も
// POST せずトークンがステータスバーに出ない症状の根本原因）。
// 正スラッシュ（`C:/dev/foo/many-ai-cli.exe`）なら Git Bash / PowerShell とも
// 引用符なしで実行でき、Codex の config.toml（TOML 文字列でも `\d` 等が不正
// エスケープになる）でも同じ変換で破損を回避できる。
// 公式ドキュメント: https://code.claude.com/docs/en/statusline.md の
// 「Windows configuration」節（バックスラッシュは無視されコマンドが沈黙する）。
//
// 注意: パスにスペースが含まれる場合の引用は Git Bash と PowerShell で必要な
// 形式（"..." vs `& "..."`）が異なり両立しないため、ここでは行わない
// （many-ai-cli の配置パスにスペースを含めない運用前提）。
func toShellPath(p string) string {
	// filepath.ToSlash は実行時 OS の PathSeparator しか変換しないため、
	// Linux CI 上で Windows パスを扱うテストでは `\` が残る。POSIX シェル
	// 用フックは常に `/` 区切りなので、無条件で `\` を `/` に置換する。
	return strings.ReplaceAll(p, `\`, "/")
}

// claudeStatusLineCmd は relay コマンド文字列を組み立てる。
//
// token は CLI 引数ではなく POSIX 環境変数プレフィックスで渡す
// （MANY_AI_CLI_HUB_TOKEN=<hex> <exe> usage-relay ...）。
// /proc/<pid>/cmdline / ps aux から他ユーザーが token を読み取れる経路を閉じるため。
// Claude Code は Windows でも Git Bash（POSIX）経由で statusLine を実行するので
// 同一構文で動作する。
func claudeStatusLineCmd(p UsageHookParams) string {
	return fmt.Sprintf("%s=%s %s usage-relay --provider claude --hub %s --session %d",
		hubTokenEnvName, p.Token, toShellPath(p.ExePath), p.HubURL, p.SessionID)
}

// hubTokenEnvName は relay 側 (internal/usagerelay/usagerelay.go) の hubTokenEnv と同値。
// 循環 import を避けるためここに複製する。両者の値は乖離させないこと。
const hubTokenEnvName = "MANY_AI_CLI_HUB_TOKEN"

// WriteClaudeStatuslineSettings は statusLine だけを含む wrapper 所有の一時
// settings JSON を書き出し、そのパスと後始末関数を返す。
// `claude --settings <path>` に渡して使う。
// JSON は json.Marshal で生成するため Windows パスのバックスラッシュも常に
// 正しくエスケープされ、手編集由来の破損は起こり得ない。
func WriteClaudeStatuslineSettings(p UsageHookParams) (path string, cleanup func(), err error) {
	type statusLineValue struct {
		Type    string `json:"type"`
		Command string `json:"command"`
		Padding int    `json:"padding"`
	}
	doc := map[string]any{
		"statusLine": statusLineValue{
			Type:    "command",
			Command: claudeStatusLineCmd(p),
			Padding: 0,
		},
	}
	body, err := json.MarshalIndent(doc, "", "  ")
	if err != nil {
		return "", nil, fmt.Errorf("marshal statusline settings: %w", err)
	}
	name := fmt.Sprintf("aac-claude-statusline-s%d-%d.json", p.SessionID, os.Getpid())
	path = filepath.Join(os.TempDir(), name)
	// 共用 /tmp での symlink 追従を防ぐ（AUDIT-4）: 既存（stale or 他ユーザーが張った
	// symlink）を除去してから O_EXCL で排他生成する。O_EXCL は symlink 先へ追従して書き込まず、
	// 競合時は失敗するため、事前に張られたリンク経由で任意ファイルを truncate されない。
	// 0600: Hub URL と token を含むため他ユーザーに読ませない。
	_ = os.Remove(path)
	f, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600) // #nosec G304 -- 固定名の wrapper 専用 temp（token を含むため 0600 が意図）
	if err != nil {
		return "", nil, fmt.Errorf("create statusline settings: %w", err)
	}
	if _, err := f.Write(append(body, '\n')); err != nil {
		_ = f.Close()
		_ = os.Remove(path)
		return "", nil, fmt.Errorf("write statusline settings: %w", err)
	}
	if err := f.Close(); err != nil {
		_ = os.Remove(path)
		return "", nil, fmt.Errorf("close statusline settings: %w", err)
	}
	cleanup = func() { _ = os.Remove(path) }
	return path, cleanup, nil
}

// ---------------------------------------------------------------------------
// Codex: ~/.codex/config.toml への Stop フック冪等注入
//
// Codex の Stop フック設定形式（要実機確認: フォーマットが確定次第更新）:
//
//	[[hooks.Stop]]
//	command = "<cmd>"
//
// マーカーコメントで自前注入ブロックを識別し、全セッション終了時に除去する。
// ---------------------------------------------------------------------------

// codexConfigPath は ~/.codex/config.toml のパスを返す。
func codexConfigPath() string {
	home, _ := os.UserHomeDir()
	codexHome := os.Getenv("CODEX_HOME")
	if codexHome == "" {
		codexHome = filepath.Join(home, ".codex")
	}
	return filepath.Join(codexHome, "config.toml")
}

// usageHookQuotePOSIX は s を POSIX シングルクォートで囲み、シェル再解釈時の
// 語分割・展開を防ぐ（埋め込みシングルクォートは close/quote/reopen イディオム）。
// Codex は config.toml の command 値（TOML 文字列）を取り出した後さらにシェルとして
// 解釈するため、exe パスにスペースが含まれると無クォートでは語分割で壊れる。
// toShellPath で正スラッシュ化済みのパスはバックスラッシュを含まないので、
// このシングルクォート化だけで安全に 1 引数化できる。
func usageHookQuotePOSIX(s string) string {
	return "'" + strings.ReplaceAll(s, "'", `'\''`) + "'"
}

// codexStopHookBlock は注入するブロックテキストを返す。
//
// token は CLI 引数ではなく POSIX 環境変数プレフィックスで渡す
// （MANY_AI_CLI_HUB_TOKEN=<hex> <exe> usage-relay ...）。
// /proc/<pid>/cmdline / ps aux から他ユーザーが token を読み取れる経路を閉じるため。
// Codex は config.toml の command 値を POSIX シェルで解釈するため同一構文で動作する。
//
// 注意: Codex の config.toml フォーマットは要実機確認。
// 現在は OpenAI Codex CLI の [[hooks.Stop]] TOML テーブル配列形式を想定。
func codexStopHookBlock(p UsageHookParams) string {
	// exe パスのみクォートする。HubURL（ポート番号のみ）と Token（hex）は型上
	// シェルメタ文字を持てないためクォート不要だが、exe パスはユーザーの配置場所
	// 次第でスペースを含み得るので語分割を防ぐ。
	cmd := fmt.Sprintf("%s=%s %s usage-relay --provider codex --hub %s --session %d",
		hubTokenEnvName, p.Token, usageHookQuotePOSIX(toShellPath(p.ExePath)), p.HubURL, p.SessionID)
	return strings.Join([]string{
		usageHookBlockStart,
		"[[hooks.Stop]]",
		fmt.Sprintf("command = %q", cmd),
		usageHookBlockEnd,
		"",
	}, "\n")
}

// InjectCodexStopHook は ~/.codex/config.toml に Stop フックを冪等注入する。
func InjectCodexStopHook(p UsageHookParams) error {
	codexConfigMu.Lock()
	defer codexConfigMu.Unlock()
	path := codexConfigPath()
	return withCodexConfigFileLock(path, func() error {
		// 既存ファイルを読む（無ければ空）。
		var content string
		if data, err := os.ReadFile(path); err == nil {
			content = string(data)
		} else if !os.IsNotExist(err) {
			return fmt.Errorf("read %s: %w", path, err)
		}

		// already 注入済みかどうか確認。
		if strings.Contains(content, usageHookBlockStart) {
			// 注入済み: コマンドだけ更新（セッション ID が変わる場合を考慮）。
			// ReplaceAllLiteralString を使う。ReplaceAllString だと newBlock 中の
			// `$name` / `${name}` が Regexp.Expand ルールで解釈され、many-ai-cli の
			// 実行ファイルパスに `$` を含むディレクトリ（例:
			// `C:\Users\alice\$portable\many-ai-cli.exe`）があると、対応するキャプチャ
			// グループが無いため無言で消える（TOML 上は構文的に壊れないので気づけない）。
			newBlock := codexStopHookBlock(p)
			blockRe := regexp.MustCompile(`(?s)` + regexp.QuoteMeta(usageHookBlockStart) + `.*?` + regexp.QuoteMeta(usageHookBlockEnd) + `\n?`)
			content = blockRe.ReplaceAllLiteralString(content, newBlock)
			return writeCodexConfig(path, content)
		}

		// 末尾に追記。
		if !strings.HasSuffix(content, "\n") && len(content) > 0 {
			content += "\n"
		}
		content += "\n" + codexStopHookBlock(p)

		return writeCodexConfig(path, content)
	})
}

// RemoveCodexStopHook は注入した Stop フックブロックを除去する。
func RemoveCodexStopHook() error {
	codexConfigMu.Lock()
	defer codexConfigMu.Unlock()
	path := codexConfigPath()
	return withCodexConfigFileLock(path, func() error {
		data, err := os.ReadFile(path)
		if os.IsNotExist(err) {
			return nil
		}
		if err != nil {
			return fmt.Errorf("read %s: %w", path, err)
		}

		content := string(data)
		if !strings.Contains(content, usageHookBlockStart) {
			return nil
		}

		blockRe := regexp.MustCompile(`(?s)\n?` + regexp.QuoteMeta(usageHookBlockStart) + `.*?` + regexp.QuoteMeta(usageHookBlockEnd) + `\n?`)
		newContent := blockRe.ReplaceAllString(content, "")
		if newContent == content {
			return nil
		}

		return writeCodexConfig(path, newContent)
	})
}

// withCodexConfigFileLock は config.toml 隣の .lock を O_EXCL で取り、RMW 区間を
// プロセス横断で直列化する。取得できなければ短時間リトライする。
func withCodexConfigFileLock(configPath string, fn func() error) error {
	lockPath := configPath + ".many-ai-cli.lock"
	const attempts = 50
	const delay = 20 * time.Millisecond
	var lockFile *os.File
	var err error
	for i := 0; i < attempts; i++ {
		lockFile, err = os.OpenFile(lockPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
		if err == nil {
			break
		}
		if !os.IsExist(err) {
			return fmt.Errorf("open codex config lock: %w", err)
		}
		// 生存 PID なし / 壊れたロックは奪取
		if data, rErr := os.ReadFile(lockPath); rErr == nil {
			if pid, cErr := strconv.Atoi(strings.TrimSpace(string(data))); cErr == nil && pid > 0 && processAlive(pid) {
				time.Sleep(delay)
				continue
			}
		}
		_ = os.Remove(lockPath)
	}
	if lockFile == nil {
		return fmt.Errorf("codex config lock busy: %s", lockPath)
	}
	_, _ = fmt.Fprintf(lockFile, "%d\n", os.Getpid())
	_ = lockFile.Close()
	defer func() { _ = os.Remove(lockPath) }()
	return fn()
}

// ScanCodexStopHookInjected は注入済みかどうかを確認する。
func ScanCodexStopHookInjected() (bool, error) {
	path := codexConfigPath()
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return false, nil
	}
	if err != nil {
		return false, fmt.Errorf("read %s: %w", path, err)
	}
	return strings.Contains(string(data), usageHookBlockStart), nil
}

func writeCodexConfig(path, content string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil { // #nosec G301 -- ~/.codex は秘密情報を持つ可能性があるため 0700
		return fmt.Errorf("mkdir %s: %w", filepath.Dir(path), err)
	}
	return os.WriteFile(path, []byte(content), 0o600) // #nosec G306,G703 -- ~/.codex/config.toml は Codex CLI の設定ファイル（0600 が意図）。path はホーム配下の固定設定パスで外部入力ではない
}
