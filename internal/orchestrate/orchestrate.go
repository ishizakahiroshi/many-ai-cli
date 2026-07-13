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
		return errors.New("orchestrate <spawn|send>")
	}
	switch args[0] {
	case "spawn":
		return runSpawn(args[1:])
	case "send":
		return runSend(args[1:])
	default:
		return fmt.Errorf("orchestrate: unknown subcommand %q", args[0])
	}
}

// hubEnv は conductor / child セッションの env から Hub 接続情報を解決する。
func hubEnv(subcommand string) (hubURL, token string, sessionID int, err error) {
	sessionID, convErr := strconv.Atoi(os.Getenv(sessionIDEnv))
	if convErr != nil || sessionID <= 0 {
		return "", "", 0, fmt.Errorf("orchestrate %s: this session is not an orchestration session (missing/invalid %s)", subcommand, sessionIDEnv)
	}
	hubPort := os.Getenv(hubPortEnv)
	if hubPort == "" {
		return "", "", 0, fmt.Errorf("orchestrate %s: %s is not set", subcommand, hubPortEnv)
	}
	token = os.Getenv(hubTokenEnv)
	if token == "" {
		return "", "", 0, fmt.Errorf("orchestrate %s: %s is not set", subcommand, hubTokenEnv)
	}
	return fmt.Sprintf("http://127.0.0.1:%s", hubPort), token, sessionID, nil
}

func runSpawn(args []string) error {
	fs := flag.NewFlagSet("orchestrate spawn", flag.ContinueOnError)
	role := fs.String("role", "", "child role (required, e.g. implementation/test/review)")
	provider := fs.String("provider", "", "override provider (default: resolved from the role mapping decided at conductor launch)")
	model := fs.String("model", "", "override model (default: resolved from the role mapping decided at conductor launch)")
	cwd := fs.String("cwd", "", "child working directory (default: parent session cwd)")
	force := fs.Bool("force", false, "spawn a new child even if a live child already exists for the role (default: rejected; use `orchestrate send` instead)")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() < 1 {
		return errors.New(`orchestrate spawn --role <role> [--provider <provider> --model <model>] [--force] "<prompt>"`)
	}
	if *role == "" {
		return errors.New("orchestrate spawn: --role is required")
	}
	prompt := fs.Arg(0)

	hubURL, token, sessionID, err := hubEnv("spawn")
	if err != nil {
		return err
	}

	result, err := spawnChild(hubURL, token, sessionID, spawnChildRequest{
		Role:          *role,
		Provider:      *provider,
		Model:         *model,
		InitialPrompt: prompt,
		CWD:           *cwd,
		Force:         *force,
	})
	if err != nil {
		return err
	}
	fmt.Printf("spawned child session #%d role=%s board=%s cwd=%s\n", result.SessionID, *role, result.BoardPath, result.CWD)
	return nil
}

// runSend は既存の子セッションへ追加指示を送る。同 role の生存子がいる限り spawn は
// 使わずこちらを使う（生存子の枠を消費せず、board へ宛先付き conductor 記帳が自動で残る）。
func runSend(args []string) error {
	fs := flag.NewFlagSet("orchestrate send", flag.ContinueOnError)
	role := fs.String("role", "", "target child role (required)")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() < 1 {
		return errors.New(`orchestrate send --role <role> "<text>"`)
	}
	if *role == "" {
		return errors.New("orchestrate send: --role is required")
	}
	text := fs.Arg(0)

	hubURL, token, sessionID, err := hubEnv("send")
	if err != nil {
		return err
	}

	result, err := sendChild(hubURL, token, sessionID, sendChildRequest{Role: *role, Text: text})
	if err != nil {
		return err
	}
	fmt.Printf("sent instruction to child session #%d role=%s board=%s\n", result.SessionID, *role, result.BoardPath)
	return nil
}

type spawnChildRequest struct {
	Role          string `json:"role"`
	Provider      string `json:"provider,omitempty"`
	Model         string `json:"model,omitempty"`
	InitialPrompt string `json:"initial_prompt"`
	CWD           string `json:"cwd,omitempty"`
	Force         bool   `json:"force,omitempty"`
}

type sendChildRequest struct {
	Role string `json:"role"`
	Text string `json:"text"`
}

type childAPIResponse struct {
	OK        bool   `json:"ok"`
	SessionID int    `json:"session_id"`
	BoardPath string `json:"board_path"`
	CWD       string `json:"cwd"`
	Error     string `json:"error"`
	Detail    string `json:"detail"`
}

// spawnChild は POST /api/sessions/:id/spawn-child を叩く。
func spawnChild(hubURL, token string, sessionID int, body spawnChildRequest) (*childAPIResponse, error) {
	return postChildAPI(fmt.Sprintf("%s/api/sessions/%d/spawn-child", hubURL, sessionID), token, body)
}

// sendChild は POST /api/sessions/:id/send-child を叩く。
func sendChild(hubURL, token string, sessionID int, body sendChildRequest) (*childAPIResponse, error) {
	return postChildAPI(fmt.Sprintf("%s/api/sessions/%d/send-child", hubURL, sessionID), token, body)
}

// postChildAPI は orchestration API への JSON POST 共通部。
// token は Authorization: Bearer ヘッダのみで渡し、argv / URL には一切乗せない
// （usage-relay と同じ、procfs/ps 経由の漏洩を避けるパターン）。
func postChildAPI(url, token string, body any) (*childAPIResponse, error) {
	payload, err := json.Marshal(body)
	if err != nil {
		return nil, fmt.Errorf("marshal request: %w", err)
	}
	req, err := http.NewRequest(http.MethodPost, url, bytes.NewReader(payload))
	if err != nil {
		return nil, fmt.Errorf("build request: %w", err)
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+token)

	// A user-facing spawn confirmation can legitimately take longer than the
	// old request timeout. Keep this bounded so a disconnected Hub still does
	// not leave the conductor command hanging indefinitely.
	client := &http.Client{Timeout: 3 * time.Minute}
	resp, err := client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("http post: %w", err)
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("read response: %w", err)
	}
	var result childAPIResponse
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
