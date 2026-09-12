// Package orchestrate implements the "many-ai-cli orchestrate" subcommand family.
//
// plan_orchestration-spawn-ui-exposure.md C2: conductor セッションの AI が
// curl や Hub token を直接扱わずに子セッションを起動できるようにする薄いラッパー。
// 認証・自セッション ID の解決はすべてこのコマンド内部で env 経由に閉じ、
// AI に見せるのは role / prompt / (任意で) provider・model だけにする。
package orchestrate

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// hubPortEnv / sessionIDEnv / hubTokenEnv は wrapper.Run が conductor / orchestration
// child セッションの実 CLI プロセスにだけ設定する env（internal/wrapper/wrapper.go 参照）。
// AI はこれらを直接読み書きする必要はなく、本コマンドが内部で消費する。
const (
	hubPortEnv      = "MANY_AI_CLI_HUB_PORT"
	sessionIDEnv    = "MANY_AI_CLI_SESSION_ID"
	hubTokenEnv     = "MANY_AI_CLI_HUB_TOKEN"
	spawnTimeoutEnv = "MANY_AI_CLI_SPAWN_TIMEOUT"
)

const defaultSpawnClientTimeout = 5 * time.Minute

// Run は "many-ai-cli orchestrate <subcommand>" のエントリポイント。
func Run(args []string) error {
	if len(args) == 0 {
		return errors.New("orchestrate <spawn|send|relay>")
	}
	switch args[0] {
	case "spawn":
		return runSpawn(args[1:])
	case "send":
		return runSend(args[1:])
	case "relay":
		return runRelay(args[1:])
	default:
		return fmt.Errorf("orchestrate: unknown subcommand %q (want spawn|send|relay)", args[0])
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
	provider := fs.String("provider", "", "override provider; one of claude|codex|copilot|cursor-agent|opencode|grok (exact lowercase). Default: the role mapping decided at conductor launch, then the provider last used for this role, then the parent's provider")
	model := fs.String("model", "", "override model (default: resolved from the role mapping decided at conductor launch)")
	cwd := fs.String("cwd", "", "root for the child's working directory (default: parent session cwd). By default many-ai-cli creates an orchestration worktree under this directory and the child runs there, not directly in this directory; pass -same-tree to have the child run in this directory itself")
	sameTree := fs.Bool("same-tree", false, "run the child directly in -cwd instead of an orchestration worktree (default: use a worktree; the child then edits your working tree directly, so do not edit it in parallel)")
	force := fs.Bool("force", false, "spawn a new child even if a live child already exists for the role (default: rejected; use `orchestrate send` instead)")
	effort := fs.String("effort", "", "reasoning effort for the child; accepted values depend on the provider (claude: low|medium|high|xhigh|max, codex: minimal|low|medium|high|xhigh|max, opencode: the model's own variant name). Providers without an effort flag (copilot / cursor-agent / grok / command-code / custom) reject a non-empty value")
	executionMode := fs.String("execution-mode", "", "auto|interactive|headless (default: auto). headless runs the child through the provider's own non-interactive mode and needs a definition for that provider; asking for it where there is none is rejected rather than quietly downgraded")
	permission := fs.String("permission", "", "attended|bounded|full (default: the orchestration child default). attended sends the child's approval prompts to the Hub's approval panel; bounded runs the allowlisted actions without asking and denies the rest (copilot has no auto-deny, so there anything outside the list still prompts); full allows everything")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() < 1 {
		return errors.New(`orchestrate spawn --role <role> [--provider <provider> --model <model>] [--effort <level>] [--execution-mode auto|interactive|headless] [--permission attended|bounded|full] [--cwd <path>] [--same-tree] [--force] "<prompt>"`)
	}
	if *role == "" {
		return errors.New("orchestrate spawn: --role is required")
	}
	prompt := fs.Arg(0)

	if *sameTree {
		fmt.Println("warning: same-tree mode: the child edits your working tree directly; do not edit the repository in parallel")
	}

	hubURL, token, sessionID, err := hubEnv("spawn")
	if err != nil {
		return err
	}

	req := spawnChildRequest{
		Role:          *role,
		Provider:      *provider,
		Model:         *model,
		InitialPrompt: prompt,
		CWD:           *cwd,
		Force:         *force,
		// 3 項目はすべて省略可。空文字のときは JSON から丸ごと落ちるので、
		// 付けずに叩いた `orchestrate spawn` の body は従来と完全に一致する。
		Effort:           *effort,
		ExecutionMode:    *executionMode,
		PermissionPreset: *permission,
	}
	if *sameTree {
		t := true
		req.SameTree = &t
	}
	result, err := spawnChild(hubURL, token, sessionID, req)
	if err != nil {
		return err
	}
	cwdOut := result.CWD
	if strings.TrimSpace(*cwd) != "" && result.CWD != *cwd {
		cwdOut = fmt.Sprintf("%s (requested=%s)", result.CWD, *cwd)
	}
	fmt.Printf("spawned child session #%d role=%s board=%s cwd=%s\n", result.SessionID, *role, result.BoardPath, cwdOut)
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

func runRelay(args []string) error {
	if len(args) > 0 {
		switch args[0] {
		case "status":
			return runRelayStatus(args[1:])
		case "stop":
			return runRelayStop(args[1:])
		default:
			if !strings.HasPrefix(args[0], "-") {
				return fmt.Errorf("orchestrate relay: unknown action %q (want status|stop, or pass --plan to start)", args[0])
			}
		}
	}
	fs := flag.NewFlagSet("orchestrate relay", flag.ContinueOnError)
	plan := fs.String("plan", "", "plan markdown path (required)")
	maxRounds := fs.Int("max-rounds", 0, "maximum review rounds per C (default: 3)")
	impl := fs.String("impl", "", "implementation provider[/model][@effort]")
	review := fs.String("review", "", "review provider[/model][@effort]")
	strong := fs.String("strong", "", "strong implementation provider[/model][@effort]")
	escalateAfter := fs.Int("escalate-after", 0, "failed review rounds before strong implementation (default: 2)")
	sameTree := fs.Bool("same-tree", false, "run children in the parent's working tree")
	extraImpl := fs.String("extra-impl", "", "extra instructions for the implementation role")
	extraReview := fs.String("extra-review", "", "extra instructions for the review role")
	executionMode := fs.String("execution-mode", "", "auto|interactive|headless for every relay role (default: orchestration.relay_execution_mode, itself unset = interactive). headless runs each instruction as one non-interactive process of the provider's own print mode and takes its exit as the completion signal; a role whose provider has no headless definition is rejected here rather than quietly run interactively")
	permission := fs.String("permission", "", "attended|bounded|full for every relay role (default: orchestration.child_permission_default). attended sends the child's approval prompts to the Hub's approval panel, which nobody is watching during an unattended relay; bounded runs the allowlisted actions without asking and denies the rest (copilot has no auto-deny, so there anything outside the list still prompts); full allows everything")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if strings.TrimSpace(*plan) == "" {
		return errors.New("orchestrate relay: --plan is required")
	}
	planPath, err := filepath.Abs(strings.TrimSpace(*plan))
	if err != nil {
		return fmt.Errorf("orchestrate relay: resolve --plan: %w", err)
	}

	roles := map[string]relayRoleAssignment{}
	for role, raw := range map[string]string{
		"implementation":        *impl,
		"review":                *review,
		"implementation-strong": *strong,
	} {
		if strings.TrimSpace(raw) == "" {
			continue
		}
		provider, model, effort, err := parseRelayProviderModel(raw)
		if err != nil {
			return fmt.Errorf("orchestrate relay --%s: %w", relayFlagName(role), err)
		}
		// 実行モードと権限の段は役割共通の 1 フラグ。役割ごとの欄へ写して送る。
		// 不正な値は Hub の検証が 400 で返す（CLI 側に 2 つ目の検証表を置かない）。
		roles[role] = relayRoleAssignment{
			Provider: provider, Model: model, Effort: effort,
			ExecutionMode: *executionMode, PermissionPreset: *permission,
		}
	}
	extra := map[string]string{}
	if strings.TrimSpace(*extraImpl) != "" {
		extra["implementation"] = *extraImpl
	}
	if strings.TrimSpace(*extraReview) != "" {
		extra["review"] = *extraReview
	}

	hubURL, token, sessionID, err := hubEnv("relay")
	if err != nil {
		return err
	}
	mode := "worktree"
	if *sameTree {
		mode = "same-tree"
		fmt.Println("warning: same-tree mode: the relay children edit your working tree directly; do not edit the repository in parallel")
	}
	result, err := postChildAPI(fmt.Sprintf("%s/api/sessions/%d/relay", hubURL, sessionID), token, relayStartRequest{
		PlanPath:      planPath,
		MaxRounds:     *maxRounds,
		Mode:          mode,
		Roles:         roles,
		EscalateAfter: *escalateAfter,
		Extra:         extra,
	})
	if err != nil {
		return err
	}
	if result.Relay == nil {
		return errors.New("hub returned no relay status")
	}
	st := result.Relay
	fmt.Printf("relay started orchestration=%s mode=%s branch=%s worktree=%s implementation=#%d max_rounds=%d\n",
		st.OrchestrationID, st.Mode, st.Branch, st.WorktreePath, st.ImplementationSessionID, st.MaxRounds)
	return nil
}

func runRelayStatus(args []string) error {
	fs := flag.NewFlagSet("orchestrate relay status", flag.ContinueOnError)
	id := fs.String("id", "", "show only this orchestration id")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 0 {
		return errors.New("orchestrate relay status [--id <orchestration-id>]")
	}
	hubURL, token, sessionID, err := hubEnv("relay status")
	if err != nil {
		return err
	}
	result, err := getChildAPI(fmt.Sprintf("%s/api/sessions/%d/relay", hubURL, sessionID), token)
	if err != nil {
		return err
	}
	shown := 0
	for _, item := range result.Relays {
		st := item.Relay
		if strings.TrimSpace(*id) != "" && st.OrchestrationID != strings.TrimSpace(*id) {
			continue
		}
		fmt.Printf("relay orchestration=%s state=%s completed_cs=%d round=%d/%d branch=%s\n",
			st.OrchestrationID, st.State, st.CompletedCs, st.Round, st.MaxRounds, st.Branch)
		shown++
	}
	if shown == 0 {
		if strings.TrimSpace(*id) != "" {
			return fmt.Errorf("orchestrate relay status: relay %q not found", strings.TrimSpace(*id))
		}
		fmt.Println("no relays")
	}
	return nil
}

func runRelayStop(args []string) error {
	fs := flag.NewFlagSet("orchestrate relay stop", flag.ContinueOnError)
	id := fs.String("id", "", "orchestration id (optional when exactly one relay is active)")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 0 {
		return errors.New("orchestrate relay stop [--id <orchestration-id>]")
	}
	hubURL, token, sessionID, err := hubEnv("relay stop")
	if err != nil {
		return err
	}
	result, err := postChildAPI(fmt.Sprintf("%s/api/sessions/%d/relay-stop", hubURL, sessionID), token, relayControlRequest{
		OrchestrationID: strings.TrimSpace(*id),
	})
	if err != nil {
		return err
	}
	if result.Relay == nil {
		return errors.New("hub returned no relay status")
	}
	st := result.Relay
	fmt.Printf("relay stopped orchestration=%s state=%s completed_cs=%d round=%d/%d branch=%s\n",
		st.OrchestrationID, st.State, st.CompletedCs, st.Round, st.MaxRounds, st.Branch)
	return nil
}

func relayFlagName(role string) string {
	switch role {
	case "implementation":
		return "impl"
	case "implementation-strong":
		return "strong"
	case "review":
		return "review"
	default:
		return role
	}
}

// parseRelayProviderModel splits a relay role flag into its parts. The form
// is provider[/model][@effort]: the model and the effort are both optional,
// and a value without "@" parses exactly as it did before effort existed
// (子 plan: docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C3).
// Keeping effort inside the same argument is deliberate — one flag per role
// stays one flag per role.
func parseRelayProviderModel(raw string) (provider, model, effort string, err error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return "", "", "", errors.New("provider is required")
	}
	rest, effort, hasEffort := strings.Cut(raw, "@")
	effort = strings.TrimSpace(effort)
	if hasEffort && effort == "" {
		return "", "", "", errors.New("effort after \"@\" is empty")
	}
	provider, model, _ = strings.Cut(rest, "/")
	provider = strings.TrimSpace(provider)
	model = strings.TrimSpace(model)
	if provider == "" {
		return "", "", "", errors.New("provider is required")
	}
	if strings.Contains(provider, " ") || strings.Contains(model, " ") || strings.Contains(effort, " ") {
		return "", "", "", errors.New("provider/model@effort must not contain spaces")
	}
	return provider, model, effort, nil
}

type spawnChildRequest struct {
	Role          string `json:"role"`
	Provider      string `json:"provider,omitempty"`
	Model         string `json:"model,omitempty"`
	InitialPrompt string `json:"initial_prompt"`
	CWD           string `json:"cwd,omitempty"`
	Force         bool   `json:"force,omitempty"`
	// 起動要求の共通 3 項目。omitempty なので省略時は JSON に現れず、
	// Hub 側の検証も素通りする（子 plan:
	// docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C3）。
	Effort           string `json:"effort,omitempty"`
	ExecutionMode    string `json:"execution_mode,omitempty"`
	PermissionPreset string `json:"permission_preset,omitempty"`
	// SameTree is tri-state: nil (omitted) means "follow the user's
	// orchestration.worktree_auto setting" (default). Set to true only when
	// -same-tree was passed. There is deliberately no way to send an explicit
	// false from this CLI flag (see -same-tree help text).
	SameTree *bool `json:"same_tree,omitempty"`
}

type sendChildRequest struct {
	Role string `json:"role"`
	Text string `json:"text"`
}

type childAPIResponse struct {
	OK                      bool                   `json:"ok"`
	SessionID               int                    `json:"session_id"`
	BoardPath               string                 `json:"board_path"`
	CWD                     string                 `json:"cwd"`
	OrchestrationID         string                 `json:"orchestration_id"`
	ImplementationSessionID int                    `json:"implementation_session_id"`
	WorktreePath            string                 `json:"worktree_path"`
	Branch                  string                 `json:"branch"`
	Relay                   *relayStatusResponse   `json:"relay"`
	Relays                  []relayAPIItemResponse `json:"relays"`
	Error                   string                 `json:"error"`
	Detail                  string                 `json:"detail"`
}

type relayRoleAssignment struct {
	Provider string `json:"provider,omitempty"`
	Model    string `json:"model,omitempty"`
	// Effort は --impl claude/opus@high の "@" 以降。ExecutionMode と
	// PermissionPreset は役割共通の --execution-mode / --permission を各役割へ
	// 写した値。いずれも omitempty なので、付けずに叩いた relay の body は
	// 従来と完全に一致する。
	Effort           string `json:"effort,omitempty"`
	ExecutionMode    string `json:"execution_mode,omitempty"`
	PermissionPreset string `json:"permission_preset,omitempty"`
}

type relayStartRequest struct {
	PlanPath      string                         `json:"plan_path"`
	MaxRounds     int                            `json:"max_rounds,omitempty"`
	Mode          string                         `json:"mode,omitempty"`
	Roles         map[string]relayRoleAssignment `json:"roles,omitempty"`
	EscalateAfter int                            `json:"escalate_after,omitempty"`
	Extra         map[string]string              `json:"extra,omitempty"`
}

type relayControlRequest struct {
	OrchestrationID string `json:"orchestration_id,omitempty"`
}

type relayAPIItemResponse struct {
	Relay relayStatusResponse `json:"relay"`
}

type relayStatusResponse struct {
	OrchestrationID         string `json:"orchestration_id"`
	PlanPath                string `json:"plan_path"`
	Mode                    string `json:"mode"`
	State                   string `json:"state"`
	Reason                  string `json:"reason"`
	CompletedCs             int    `json:"completed_cs"`
	Round                   int    `json:"round"`
	MaxRounds               int    `json:"max_rounds"`
	ImplementationSessionID int    `json:"implementation_session_id"`
	WorktreePath            string `json:"worktree_path"`
	Branch                  string `json:"branch"`
}

// spawnChild は POST /api/sessions/:id/spawn-child を叩く。
func spawnChild(hubURL, token string, sessionID int, body spawnChildRequest) (*childAPIResponse, error) {
	return postChildAPI(fmt.Sprintf("%s/api/sessions/%d/spawn-child", hubURL, sessionID), token, body)
}

// sendChild は POST /api/sessions/:id/send-child を叩く。
func sendChild(hubURL, token string, sessionID int, body sendChildRequest) (*childAPIResponse, error) {
	return postChildAPI(fmt.Sprintf("%s/api/sessions/%d/send-child", hubURL, sessionID), token, body)
}

func getChildAPI(url, token string) (*childAPIResponse, error) {
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		return nil, fmt.Errorf("build request: %w", err)
	}
	req.Header.Set("Authorization", "Bearer "+token)
	client := &http.Client{Timeout: 30 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("http get: %w", err)
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

// postChildAPI は orchestration API への JSON POST 共通部。
// token は Authorization: Bearer ヘッダのみで渡し、argv / URL には一切乗せない
// （usage-relay と同じ、procfs/ps 経由の漏洩を避けるパターン）。
func spawnClientTimeout() time.Duration {
	if raw := strings.TrimSpace(os.Getenv(spawnTimeoutEnv)); raw != "" {
		if d, err := time.ParseDuration(raw); err == nil && d > 0 {
			return d
		}
	}
	return defaultSpawnClientTimeout
}

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

	// A user-facing spawn confirmation is held by the Hub with no deadline
	// (C1, plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md):
	// it survives this process being killed by its own tool-call timeout.
	// This client-side timeout only bounds how long *this* command waits; it
	// does not cancel the confirmation itself, so its wording must not read
	// as a refusal or prompt a duplicate spawn request.
	timeout := spawnClientTimeout()
	client := &http.Client{Timeout: timeout}
	resp, err := client.Do(req)
	if err != nil {
		if errors.Is(err, context.DeadlineExceeded) {
			return nil, fmt.Errorf("spawn confirmation pending in Hub (waited %s locally): user has not yet decided in browser. DO NOT retry spawn (duplicate request will be rejected). The child will start once approved, and its session ID will be delivered via orchestration notification. Once running, instruct it via `orchestrate send`: %w", client.Timeout, err)
		}
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
		if result.Error == "spawn_refused" {
			// The only case that actually means a human clicked refuse.
			return nil, fmt.Errorf("spawn refused by the user: %s", detail)
		}
		return nil, fmt.Errorf("hub returned %d: %s", resp.StatusCode, detail)
	}
	return &result, nil
}
