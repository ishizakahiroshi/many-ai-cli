package wrapper

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	"many-ai-cli/internal/securefile"
)

// 委譲案内のファイル注入経路 — セッション単位の口を持たない provider 向け。
//
// 2026-08-29 の調査（plan_cross-session-messaging-generalization.md C6）で、
// セッション単位でシステムプロンプトへ追記できるのは claude（--append-system-prompt-file）と
// grok（--rules）だけだと分かった。codex / copilot / cursor-agent にはその口が無く、
// 読ませる手段は指示ファイルしかない。
//
// **この経路ではセッションごとの切り替えができない。** 新規セッションパネルのチェックボックスは
// claude / grok にしか効かず、この 3 つは設定（user_prefs.spawn.delegation_auto）単位の
// オン・オフになる。挙動が揃わないことを承知のうえで入れる判断（2026-08-29 ユーザー）。
//
// 承認ルール（approval_rules.go）とは**別ファイル・別マーカー**にする。
// 一度あちらへ相乗りさせて「マーカーの書式仕様」と「機能の案内」を混ぜてしまい、
// 責務が違うとして撤去した経緯がある（rulesVersion 19 → 20）。同じ轍を踏まない。
//
// 本文は delegationPromptText を唯一の正本として共有する。経路が違っても AI が読む内容は同じにする。
//
// 生成物の回収について: 利用者の AGENTS.md 等へ書き込むので、**設定を切ったときとセッションが
// 消えたときに必ず外す**（hub の injectApprovalTargets / removeApprovalTargets と同じ寿命に乗せる）。
// 置き去りは `many-ai-cli doctor` が DelegationResidueNeedle で検出する。
const (
	delegationBlockStart  = "<!-- many-ai-cli:delegation -->"
	delegationBlockEnd    = "<!-- /many-ai-cli:delegation -->"
	delegationFileVersion = "1"
)

// DelegationResidueNeedle は置き去りブロックを探すための検索文字列。
// 旧名（any-ai-cli）で注入された版は存在しない（本ブロックは 2026-08-29 が初版）が、
// 将来 prefix が変わっても当たるよう共通部分だけを見る。
const DelegationResidueNeedle = "ai-cli:delegation"

// centralDelegationPath は ~/.many-ai-cli/delegation.md のパスを返す。
func centralDelegationPath() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".many-ai-cli", "delegation.md")
}

func delegationFileContent() string {
	return fmt.Sprintf("<!-- version: %s -->\n## many-ai-cli Delegation\n\n%s", delegationFileVersion, delegationPromptText)
}

// SyncDelegationFile は中央ファイルを最新版へ揃える（version 不一致か不存在なら書き直す）。
// 権限まわりは SyncRulesFile と同じ扱いにする（同じディレクトリへ置くため）。
func SyncDelegationFile() error {
	path := centralDelegationPath()
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("mkdir %s: %w", dir, err)
	}
	if err := os.Chmod(dir, 0o700); err != nil { // #nosec G302 -- ディレクトリには実行ビットが必要
		return fmt.Errorf("chmod %s: %w", dir, err)
	}
	_ = securefile.EnsurePrivateDir(dir)
	if data, err := os.ReadFile(path); err == nil {
		firstLine := strings.TrimSpace(string(data))
		if idx := strings.IndexByte(string(data), '\n'); idx >= 0 {
			firstLine = strings.TrimSpace(string(data[:idx]))
		}
		if firstLine == fmt.Sprintf("<!-- version: %s -->", delegationFileVersion) {
			return nil
		}
	}
	return os.WriteFile(path, []byte(delegationFileContent()), 0o644) // #nosec G306 -- 各 AI CLI が読む共有ファイル（秘密情報なし）
}

// providerUsesDelegationBlock は「ファイル注入で案内を渡す provider か」。
// claude と grok はセッション単位の口があるのでこちらでは扱わない（二重に渡さない）。
// opencode は読ませる経路が未調査のため対象外。
func providerUsesDelegationBlock(provider string) bool {
	switch provider {
	case "codex", "copilot", "cursor-agent":
		return true
	default:
		return false
	}
}

// scanBlockPresent は指定マーカーの開始行がファイルに含まれるかを返す。
func scanBlockPresent(path, startMarker string) (bool, error) {
	f, err := os.Open(path) // #nosec G304 -- provider 既知の instruction file パス
	if err != nil {
		return false, err
	}
	defer f.Close()
	scanner := bufio.NewScanner(f)
	scanner.Buffer(make([]byte, 64*1024), 8*1024*1024)
	for scanner.Scan() {
		if strings.TrimSpace(scanner.Text()) == startMarker {
			return true, nil
		}
	}
	if err := scanner.Err(); err != nil {
		return false, err
	}
	return false, nil
}

// blockContainsVersion は注入済みブロックの中身が指定 version かを返す。
func blockContainsVersion(path, startMarker, endMarker, version string) (bool, error) {
	data, err := os.ReadFile(path) // #nosec G304 -- provider 既知の instruction file パス
	if err != nil {
		return false, err
	}
	blockRe := regexp.MustCompile(`(?s)` + regexp.QuoteMeta(startMarker) + `(.*?)` + regexp.QuoteMeta(endMarker))
	m := blockRe.FindSubmatch(data)
	if m == nil {
		return false, nil
	}
	return strings.Contains(string(m[1]), fmt.Sprintf("<!-- version: %s -->", version)), nil
}

// appendBlock はファイル末尾へブロックを追記する。
func appendBlock(path, startMarker, endMarker, body string) error {
	block := strings.Join([]string{startMarker, strings.TrimSpace(body), endMarker, ""}, "\n")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil { // #nosec G301 -- 利用者プロジェクト内の通常ディレクトリ
		return fmt.Errorf("mkdir %s: %w", filepath.Dir(path), err)
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644) // #nosec G302 G304 -- AGENTS.md 等は共有ドキュメント
	if err != nil {
		return fmt.Errorf("open %s: %w", path, err)
	}
	defer f.Close()
	_, err = fmt.Fprintf(f, "\n%s", block)
	return err
}

// removeBlock はファイルからブロックを取り除く。無ければ何もしない。
func removeBlock(path, startMarker, endMarker string) error {
	content, err := os.ReadFile(path) // #nosec G304 -- provider 既知の instruction file パス
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return fmt.Errorf("read %s: %w", path, err)
	}
	blockRe := regexp.MustCompile(`(?s)\n?` + regexp.QuoteMeta(startMarker) + `.*?` + regexp.QuoteMeta(endMarker) + `\n?`)
	newContent := blockRe.ReplaceAllString(string(content), "")
	if newContent == string(content) {
		return nil
	}
	return os.WriteFile(path, []byte(newContent), 0o644) // #nosec G703 G306 G304 -- provider 既知の instruction file パスのみ（HTTP 入力なし）。共有ドキュメントのため 0644 が意図
}

// InjectDelegation は委譲案内のブロックを注入する。対象外の provider では何もしない。
// 既に注入済みでも version が古ければ入れ替える。
func InjectDelegation(provider, path string) error {
	if !providerUsesDelegationBlock(provider) {
		return nil
	}
	if err := SyncDelegationFile(); err != nil {
		return fmt.Errorf("sync delegation file: %w", err)
	}
	already, err := scanBlockPresent(path, delegationBlockStart)
	if err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("scan %s: %w", path, err)
	}
	if already {
		current, cerr := blockContainsVersion(path, delegationBlockStart, delegationBlockEnd, delegationFileVersion)
		if cerr != nil {
			return fmt.Errorf("check delegation block version %s: %w", path, cerr)
		}
		if current {
			return nil
		}
		if rerr := RemoveDelegation(path); rerr != nil {
			return fmt.Errorf("remove stale delegation block %s: %w", path, rerr)
		}
	}
	body, err := os.ReadFile(centralDelegationPath())
	if err != nil {
		return fmt.Errorf("read central delegation file: %w", err)
	}
	return appendBlock(path, delegationBlockStart, delegationBlockEnd, string(body))
}

// RemoveDelegation は委譲案内のブロックを取り除く。provider は問わない
// （設定を切った後や、対象から外れた後の回収でも呼ばれるため）。
func RemoveDelegation(path string) error {
	return removeBlock(path, delegationBlockStart, delegationBlockEnd)
}
