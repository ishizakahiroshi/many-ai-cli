// Package doctor provides local, non-mutating diagnostics for many-ai-cli.
package doctor

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"many-ai-cli/internal/config"
)

type Level string

const (
	OK   Level = "OK"
	Warn Level = "WARN"
	Fail Level = "FAIL"
)

type Check struct {
	Name    string `json:"name"`
	Level   Level  `json:"level"`
	Message string `json:"message"`
	Fix     string `json:"fix,omitempty"`
}

type Report struct {
	Checks []Check `json:"checks"`
}

// Run performs bounded, local checks only. It never writes configuration or logs.
func Run(ctx context.Context, cfg *config.Config) Report {
	return Report{Checks: []Check{
		providers(ctx),
		port(cfg),
		token(cfg),
		acl(),
		ollama(ctx, cfg),
		whisper(ctx, cfg),
		tailscale(cfg),
		logs(cfg),
	}}
}

func providers(ctx context.Context) Check {
	names := []string{"claude", "codex", "copilot", "cursor-agent", "opencode", "grok"}
	var found []string
	for _, name := range names {
		path, err := exec.LookPath(name)
		if err != nil {
			continue
		}
		versionCtx, cancel := context.WithTimeout(ctx, time.Second)
		out, err := exec.CommandContext(versionCtx, path, "--version").Output()
		cancel()
		if err == nil && strings.TrimSpace(string(out)) != "" {
			found = append(found, name+" ("+firstLine(string(out))+")")
		} else {
			found = append(found, name)
		}
	}
	if len(found) == 0 {
		return Check{"provider", Fail, "対応する AI CLI が PATH に見つかりません", "Claude Code または Codex CLI をインストールし、PATH を確認してください"}
	}
	return Check{"provider", OK, "検出: " + strings.Join(found, ", "), ""}
}

func port(cfg *config.Config) Check {
	addr := fmt.Sprintf("127.0.0.1:%d", cfg.Hub.Port)
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		return Check{"port", Warn, addr + " は使用中です（Hub が起動中の可能性があります）", "many-ai-cli status で Hub の状態を確認してください"}
	}
	_ = ln.Close()
	return Check{"port", OK, addr + " は空いています", ""}
}

func token(cfg *config.Config) Check {
	if strings.TrimSpace(cfg.Token) == "" {
		return Check{"token", Fail, "Hub トークンが未設定です", "設定を再生成するには ~/.many-ai-cli/config.yaml を安全な場所へ退避してから再起動してください"}
	}
	path, err := config.Path()
	if err != nil {
		return Check{"token", Warn, "Hub トークンは設定済みですが、設定ファイルを確認できません", "HOME/USERPROFILE を確認してください"}
	}
	info, err := os.Stat(path)
	if err != nil {
		return Check{"token", Warn, "Hub トークンは設定済みですが、設定ファイルを確認できません", "~/.many-ai-cli/config.yaml の読み取り権限を確認してください"}
	}
	if runtime.GOOS != "windows" && info.Mode().Perm()&0o077 != 0 {
		return Check{"token", Warn, "Hub トークンは設定済みですが、設定ファイルの権限が広すぎます", "chmod 600 ~/.many-ai-cli/config.yaml を実行してください"}
	}
	return Check{"token", OK, "Hub トークンと設定ファイル権限を確認しました（値は表示しません）", ""}
}

func acl() Check {
	dir, err := config.Dir()
	if err != nil {
		return Check{"ACL", Fail, "設定ディレクトリを特定できません: " + err.Error(), "HOME/USERPROFILE を確認してください"}
	}
	info, err := os.Stat(dir)
	if os.IsNotExist(err) {
		return Check{"ACL", Warn, "設定ディレクトリがまだありません", "many-ai-cli を一度起動して設定を作成してください"}
	}
	if err != nil {
		return Check{"ACL", Warn, "設定ディレクトリを確認できません: " + err.Error(), "ディレクトリの読み書き権限を確認してください"}
	}
	if runtime.GOOS != "windows" && info.Mode().Perm()&0o077 != 0 {
		return Check{"ACL", Warn, "設定ディレクトリの権限が広すぎます", "chmod 700 ~/.many-ai-cli を実行してください"}
	}
	return Check{"ACL", OK, "設定ディレクトリへアクセスできます", ""}
}

func ollama(ctx context.Context, cfg *config.Config) Check {
	base := config.EffectiveOllamaBaseURL(cfg.Ollama.BaseURL)
	reqCtx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(reqCtx, http.MethodGet, strings.TrimRight(base, "/")+"/api/tags", nil)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return Check{"Ollama", Warn, "Ollama に接続できません: " + base, "Ollama を起動するか ollama.base_url を確認してください"}
	}
	defer resp.Body.Close()
	if resp.StatusCode/100 != 2 {
		return Check{"Ollama", Warn, "Ollama が HTTP " + resp.Status, "Ollama の起動状態と base_url を確認してください"}
	}
	return Check{"Ollama", OK, "Ollama に接続できます", ""}
}

func whisper(ctx context.Context, cfg *config.Config) Check {
	url := strings.TrimSpace(cfg.Voice.Whisper.ServerURL)
	if url == "" {
		return Check{"Whisper", Warn, "Whisper サーバーは未設定です", "音声入力を使う場合は Settings で Whisper を設定してください"}
	}
	reqCtx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(reqCtx, http.MethodGet, url, nil)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return Check{"Whisper", Warn, "Whisper に接続できません", "Whisper runtime を起動するか server_url を確認してください"}
	}
	resp.Body.Close()
	return Check{"Whisper", OK, "Whisper endpoint に接続できます", ""}
}

func tailscale(cfg *config.Config) Check {
	path, err := exec.LookPath("tailscale")
	if err != nil {
		return Check{"Tailscale", Warn, "tailscale コマンドは見つかりません", "モバイル接続に Tailscale を使う場合はインストールしてください"}
	}
	checkCtx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	out, err := exec.CommandContext(checkCtx, path, "status", "--json").Output()
	if err != nil {
		return Check{"Tailscale", Warn, "tailscale は検出しましたが接続状態を確認できません", "tailscale login を実行して接続状態を確認してください"}
	}
	var status struct {
		BackendState string `json:"BackendState"`
		Self         struct {
			DNSName string `json:"DNSName"`
		} `json:"Self"`
	}
	if err := json.Unmarshal(out, &status); err != nil || !strings.EqualFold(status.BackendState, "Running") {
		return Check{"Tailscale", Warn, "Tailscale は未接続です", "tailscale up を実行してから再試行してください"}
	}
	host := strings.TrimSuffix(strings.TrimSpace(status.Self.DNSName), ".")
	if host != "" && len(cfg.Hub.AllowedHosts) > 0 {
		for _, allowed := range cfg.Hub.AllowedHosts {
			if strings.EqualFold(strings.TrimSuffix(strings.TrimSpace(allowed), "."), host) {
				return Check{"Tailscale", OK, "Tailscale は接続済みで allowed_hosts に登録済みです", ""}
			}
		}
		return Check{"Tailscale", Warn, "Tailscale は接続済みですが、この端末名は allowed_hosts に未登録です", "Hub UI の Expose から公開を有効にするか hub.allowed_hosts を確認してください"}
	}
	return Check{"Tailscale", OK, "Tailscale は接続済みです", ""}
}

func logs(cfg *config.Config) Check {
	dir := strings.TrimSpace(cfg.Hub.LogDir)
	if dir == "" {
		base, err := config.Dir()
		if err != nil {
			return Check{"log", Warn, "ログディレクトリを特定できません", "設定を確認してください"}
		}
		dir = filepath.Join(base, "logs")
	}
	info, err := os.Stat(dir)
	if os.IsNotExist(err) {
		return Check{"log", Warn, "ログディレクトリは未作成です", "Hub を起動すると必要に応じて作成されます"}
	}
	if err != nil || !info.IsDir() {
		return Check{"log", Fail, "ログディレクトリへアクセスできません", "hub.log_dir の書き込み権限を確認してください"}
	}
	var size int64
	var count int
	err = filepath.WalkDir(dir, func(path string, entry os.DirEntry, walkErr error) error {
		if walkErr != nil || entry.IsDir() {
			return walkErr
		}
		if info, statErr := entry.Info(); statErr == nil {
			size += info.Size()
			count++
		}
		return nil
	})
	if err != nil {
		return Check{"log", Warn, "ログディレクトリの容量を確認できません", "hub.log_dir の読み取り権限を確認してください"}
	}
	return Check{"log", OK, fmt.Sprintf("ログディレクトリへアクセスできます（%d files, %.1f MB）", count, float64(size)/(1024*1024)), ""}
}

func firstLine(s string) string {
	if i := strings.IndexByte(strings.TrimSpace(s), '\n'); i >= 0 {
		return strings.TrimSpace(s)[:i]
	}
	return strings.TrimSpace(s)
}
