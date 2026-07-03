// Package orchestrate implements the "many-ai-cli orchestrate" subcommand family.
//
// plan_orchestration-spawn-ui-exposure.md C2: conductor セッションの AI が
// curl や Hub token を直接扱わずに子セッションを起動できるようにする薄いラッパー。
// 認証・自セッション ID の解決はすべてこのコマンド内部で env 経由に閉じ、
// AI に見せるのは role / prompt / (任意で) provider・model だけにする。
package orchestrate

import (
	"bytes"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"strconv"
	"time"
)

// hubPortEnv / sessionIDEnv / hubTokenEnv は wrapper.Run が conductor / orchestration
// child セッションの実 CLI プロセスにだけ設定する env（internal/wrapper/wrapper.go 参照）。
// AI はこれらを直接読み書きする必要はなく、本コマンドが内部で消費する。
const (
	hubPortEnv   = "MANY_AI_CLI_HUB_PORT"
	sessionIDEnv = "MANY_AI_CLI_SESSION_ID"
	hubTokenEnv  = "MANY_AI_CLI_HUB_TOKEN"
)

// Run は "many-ai-cli orchestrate <subcommand>" のエントリポイント。
func Run(args []string) error {
	if len(args) == 0 {
		return errors.New("orchestrate <spawn>")
	}
	switch args[0] {
	case "spawn":
		return runSpawn(args[1:])
	default:
		return fmt.Errorf("orchestrate: unknown subcommand %q", args[0])
	}
}

func runSpawn(args []string) error {
	fs := flag.NewFlagSet("orchestrate spawn", flag.ContinueOnError)
	role := fs.String("role", "", "child role (required, e.g. implementation/test/review)")
	provider := fs.String("provider", "", "override provider (default: resolved from the role mapping decided at conductor launch)")
	model := fs.String("model", "", "override model (default: resolved from the role mapping decided at conductor launch)")
	cwd := fs.String("cwd", "", "child working directory (default: parent session cwd)")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() < 1 {
		return errors.New(`orchestrate spawn --role <role> [--provider <provider> --model <model>] "<prompt>"`)
	}
	if *role == "" {
		return errors.New("orchestrate spawn: --role is required")
	}
	prompt := fs.Arg(0)

	sessionID, err := strconv.Atoi(os.Getenv(sessionIDEnv))
	if err != nil || sessionID <= 0 {
		return fmt.Errorf("orchestrate spawn: this session is not an orchestration session (missing/invalid %s)", sessionIDEnv)
	}
	hubPort := os.Getenv(hubPortEnv)
	if hubPort == "" {
		return fmt.Errorf("orchestrate spawn: %s is not set", hubPortEnv)
	}
	token := os.Getenv(hubTokenEnv)
	if token == "" {
		return fmt.Errorf("orchestrate spawn: %s is not set", hubTokenEnv)
	}
	hubURL := fmt.Sprintf("http://127.0.0.1:%s", hubPort)

	result, err := spawnChild(hubURL, token, sessionID, spawnChildRequest{
		Role:          *role,
		Provider:      *provider,
		Model:         *model,
		InitialPrompt: prompt,
		CWD:           *cwd,
	})
	if err != nil {
		return err
	}
	fmt.Printf("spawned child session #%d role=%s board=%s cwd=%s\n", result.SessionID, *role, result.BoardPath, result.CWD)
	return nil
}

type spawnChildRequest struct {
	Role          string `json:"role"`
	Provider      string `json:"provider,omitempty"`
	Model         string `json:"model,omitempty"`
	InitialPrompt string `json:"initial_prompt"`
	CWD           string `json:"cwd,omitempty"`
}

type spawnChildResponse struct {
	OK        bool   `json:"ok"`
	SessionID int    `json:"session_id"`
	BoardPath string `json:"board_path"`
	CWD       string `json:"cwd"`
	Error     string `json:"error"`
	Detail    string `json:"detail"`
}

// spawnChild は POST /api/sessions/:id/spawn-child を叩く。
// token は Authorization: Bearer ヘッダのみで渡し、argv / URL には一切乗せない
// （usage-relay と同じ、procfs/ps 経由の漏洩を避けるパターン）。
func spawnChild(hubURL, token string, sessionID int, body spawnChildRequest) (*spawnChildResponse, error) {
	payload, err := json.Marshal(body)
	if err != nil {
		return nil, fmt.Errorf("marshal request: %w", err)
	}
	url := fmt.Sprintf("%s/api/sessions/%d/spawn-child", hubURL, sessionID)
	req, err := http.NewRequest(http.MethodPost, url, bytes.NewReader(payload))
	if err != nil {
		return nil, fmt.Errorf("build request: %w", err)
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+token)

	client := &http.Client{Timeout: 25 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("http post: %w", err)
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("read response: %w", err)
	}
	var result spawnChildResponse
	if err := json.Unmarshal(raw, &result); err != nil {
		return nil, fmt.Errorf("parse response (status %d): %w", resp.StatusCode, err)
	}
	if resp.StatusCode != http.StatusOK || !result.OK {
		detail := result.Detail
		if detail == "" {
			detail = result.Error
		}
		return nil, fmt.Errorf("hub returned %d: %s", resp.StatusCode, detail)
	}
	return &result, nil
}
