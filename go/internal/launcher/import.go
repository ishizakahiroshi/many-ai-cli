package launcher

// import.go — 手元 PC が SSH-pull でリモートの `profile-export --json` を叩き、
// 接続プロファイルを取り込むためのクライアント側ロジック
// （plan_server-profile-export-import.md C2）。
//
// エージェントを使わない人向けの導線: UI の「リモートから取得」ボタンが
// host/user/鍵/ssh-port を渡し、本パッケージが ssh 越しに自己記述プロファイルを
// 取得 → Validate → フォーム補完用データとして返す。保存自体は既存の
// /api/servers（replace-all）に委ねる。

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"strings"
)

// FetchParams は SSH-pull の接続先を表す。鍵 (IdentityFile) はクライアントが指定する。
type FetchParams struct {
	Host         string
	User         string
	SSHPort      int
	IdentityFile string
	Binary       string // リモートのバイナリ名（既定 many-ai-cli）
}

// FetchRemoteProfile は ssh 越しに `<binary> profile-export --json` を実行し、
// 出力をパースして ExportedProfile を返す。SSH 接続先フィールドは Validate を
// 再利用して引数インジェクションを防ぐ（"-" 始まり・空白・制御文字を拒否）。
func FetchRemoteProfile(ctx context.Context, fp FetchParams) (*ExportedProfile, error) {
	binary := strings.TrimSpace(fp.Binary)
	if binary == "" {
		binary = "many-ai-cli"
	}

	// 接続先フィールドの検証は Validate を再利用（serve モードの最小プロファイルで）。
	probe := Profile{
		Name:         "_fetch",
		Type:         ProfileTypeSSH,
		Mode:         SSHModeServe,
		Host:         fp.Host,
		User:         fp.User,
		SSHPort:      fp.SSHPort,
		IdentityFile: fp.IdentityFile,
		Binary:       binary,
	}
	if err := Validate(&ProfilesFile{Version: supportedVersion, Profiles: []Profile{probe}}); err != nil {
		return nil, err
	}
	normalizeProfile(&probe) // "user@host" を分割
	if probe.Host == "" {
		return nil, fmt.Errorf("host is required")
	}

	args := buildSSHBaseArgs(probe)
	args = append(args, sshTarget(probe))
	// リモートのログインシェル経由で PATH を確保しつつ JSON を stdout に出させる。
	// binary は ShellQuote で 1 トークン化（serve モードの remoteCmd 構築に倣う）。
	remoteCmd := fmt.Sprintf("%s profile-export --json", ShellQuote(binary))
	args = append(args, "--", "bash", "-lc", remoteCmd)

	cmd := exec.CommandContext(ctx, sshExe, args...)
	cmd.SysProcAttr = noWindowSysProcAttr()
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		detail := strings.TrimSpace(stderr.String())
		if detail != "" {
			return nil, fmt.Errorf("ssh profile-export failed: %s", firstLine(detail))
		}
		return nil, fmt.Errorf("ssh profile-export failed: %w", err)
	}

	exported, err := parseExportedProfile(stdout.Bytes())
	if err != nil {
		return nil, err
	}
	return exported, nil
}

// parseExportedProfile は profile-export の出力から ExportedProfile を取り出す。
// ログインシェルが stdout に先頭ノイズを混ぜても拾えるよう、最初の '{' から
// 最後の '}' までを抜き出して JSON として解釈する。
func parseExportedProfile(out []byte) (*ExportedProfile, error) {
	var exported ExportedProfile
	if err := json.Unmarshal(out, &exported); err == nil {
		return &exported, nil
	}
	start := bytes.IndexByte(out, '{')
	end := bytes.LastIndexByte(out, '}')
	if start < 0 || end < start {
		return nil, fmt.Errorf("profile-export returned no JSON output")
	}
	if err := json.Unmarshal(out[start:end+1], &exported); err != nil {
		return nil, fmt.Errorf("parse profile-export output: %w", err)
	}
	return &exported, nil
}

// firstLine は s の最初の非空行を返す（ssh のエラーは複数行になりやすいため）。
func firstLine(s string) string {
	for _, line := range strings.Split(s, "\n") {
		if l := strings.TrimSpace(line); l != "" {
			return l
		}
	}
	return strings.TrimSpace(s)
}

// UniqueProfileName は name が未使用ならそのまま、衝突するなら採番（name-2,
// name-3 ...）して既存と重複しない名前を返す。
func UniqueProfileName(existing []Profile, name string) string {
	name = strings.TrimSpace(name)
	if name == "" {
		name = "remote"
	}
	used := make(map[string]bool, len(existing))
	for _, p := range existing {
		used[p.Name] = true
	}
	if !used[name] {
		return name
	}
	for i := 2; ; i++ {
		cand := fmt.Sprintf("%s-%d", name, i)
		if !used[cand] {
			return cand
		}
	}
}
