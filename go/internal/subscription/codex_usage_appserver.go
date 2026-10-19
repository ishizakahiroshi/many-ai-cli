package subscription

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strings"
)

var ErrCodexUsageUnavailable = errors.New("codex subscription usage unavailable")

type CodexAppServerUsage struct {
	Primary   *CodexAppServerWindow
	Secondary *CodexAppServerWindow
	PlanType  string
	Credits   *CodexAppServerCredits
}

type CodexAppServerCredits struct {
	HasCredits bool
	Unlimited  bool
	Balance    string
}

type CodexAppServerWindow struct {
	UsedPercent   float64
	WindowMinutes int
	ResetsAt      int64
}

var codexAppServerCommand = func(ctx context.Context, path string) *exec.Cmd {
	name, argv := shellSafeCommand(path, []string{"app-server", "--stdio"})
	return exec.CommandContext(ctx, name, argv...)
}

// ReadCodexAppServerUsage asks the Codex process bound to profileDir for its
// ChatGPT limits. Only the fixed read RPCs below are sent; raw replies, stderr,
// account email, and authentication material must never reach Hub logs or JSON.
func ReadCodexAppServerUsage(ctx context.Context, profileDir string) (CodexAppServerUsage, error) {
	if strings.TrimSpace(profileDir) == "" {
		return CodexAppServerUsage{}, ErrCodexUsageUnavailable
	}
	info, err := os.Stat(profileDir)
	if err != nil || !info.IsDir() {
		return CodexAppServerUsage{}, ErrCodexUsageUnavailable
	}
	path, err := lookPath("codex")
	if err != nil {
		return CodexAppServerUsage{}, ErrCodexUsageUnavailable
	}
	cmd := codexAppServerCommand(ctx, path)
	cmd.Env = mergeEnv(os.Environ(), []string{CodexHomeEnv + "=" + profileDir})
	cmd.Stderr = io.Discard
	stdin, err := cmd.StdinPipe()
	if err != nil {
		return CodexAppServerUsage{}, ErrCodexUsageUnavailable
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return CodexAppServerUsage{}, ErrCodexUsageUnavailable
	}
	if err := cmd.Start(); err != nil {
		return CodexAppServerUsage{}, ErrCodexUsageUnavailable
	}
	defer func() {
		_ = stdin.Close()
		if cmd.Process != nil {
			_ = cmd.Process.Kill()
		}
		_ = cmd.Wait()
	}()
	scanner := bufio.NewScanner(stdout)
	scanner.Buffer(make([]byte, 4096), 4<<20)
	request := func(id int, method string, params any) (json.RawMessage, error) {
		message := struct {
			Method string `json:"method"`
			ID     int    `json:"id"`
			Params any    `json:"params,omitempty"`
		}{method, id, params}
		encoded, err := json.Marshal(message)
		if err != nil {
			return nil, ErrCodexUsageUnavailable
		}
		if _, err := fmt.Fprintf(stdin, "%s\n", encoded); err != nil {
			return nil, ErrCodexUsageUnavailable
		}
		for scanner.Scan() {
			var reply struct {
				ID     *int            `json:"id"`
				Result json.RawMessage `json:"result"`
				Error  json.RawMessage `json:"error"`
			}
			if json.Unmarshal(scanner.Bytes(), &reply) != nil || reply.ID == nil || *reply.ID != id {
				continue
			}
			if len(reply.Error) > 0 || len(reply.Result) == 0 {
				return nil, ErrCodexUsageUnavailable
			}
			return reply.Result, nil
		}
		return nil, ErrCodexUsageUnavailable
	}
	if _, err := request(0, "initialize", map[string]any{"clientInfo": map[string]string{
		"name": "many_ai_cli", "title": "many-ai-cli", "version": "1",
	}}); err != nil {
		return CodexAppServerUsage{}, err
	}
	if _, err := fmt.Fprintln(stdin, `{"method":"initialized","params":{}}`); err != nil {
		return CodexAppServerUsage{}, ErrCodexUsageUnavailable
	}
	rawAccount, err := request(1, "account/read", map[string]bool{"refreshToken": false})
	if err != nil {
		return CodexAppServerUsage{}, err
	}
	var account struct {
		Account *struct {
			Type     string `json:"type"`
			PlanType string `json:"planType"`
		} `json:"account"`
	}
	if json.Unmarshal(rawAccount, &account) != nil || account.Account == nil || account.Account.Type != "chatgpt" {
		return CodexAppServerUsage{}, ErrCodexUsageUnavailable
	}
	rawLimits, err := request(2, "account/rateLimits/read", nil)
	if err != nil {
		return CodexAppServerUsage{}, err
	}
	var limits struct {
		RateLimits *struct {
			Primary   *appServerWindow `json:"primary"`
			Secondary *appServerWindow `json:"secondary"`
			Credits   *struct {
				HasCredits bool   `json:"hasCredits"`
				Unlimited  bool   `json:"unlimited"`
				Balance    string `json:"balance"`
			} `json:"credits"`
		} `json:"rateLimits"`
	}
	if json.Unmarshal(rawLimits, &limits) != nil || limits.RateLimits == nil {
		return CodexAppServerUsage{}, ErrCodexUsageUnavailable
	}
	primary := limits.RateLimits.Primary.toLocal()
	secondary := limits.RateLimits.Secondary.toLocal()
	if primary == nil && secondary == nil {
		return CodexAppServerUsage{}, ErrCodexUsageUnavailable
	}
	usage := CodexAppServerUsage{Primary: primary, Secondary: secondary, PlanType: account.Account.PlanType}
	if credits := limits.RateLimits.Credits; credits != nil {
		usage.Credits = &CodexAppServerCredits{HasCredits: credits.HasCredits, Unlimited: credits.Unlimited, Balance: credits.Balance}
	}
	return usage, nil
}

type appServerWindow struct {
	UsedPercent       *float64 `json:"usedPercent"`
	WindowDurationMin int      `json:"windowDurationMins"`
	ResetsAt          int64    `json:"resetsAt"`
}

func (w *appServerWindow) toLocal() *CodexAppServerWindow {
	if w == nil || w.UsedPercent == nil || *w.UsedPercent < 0 || *w.UsedPercent > 100 || w.WindowDurationMin <= 0 {
		return nil
	}
	return &CodexAppServerWindow{UsedPercent: *w.UsedPercent, WindowMinutes: w.WindowDurationMin, ResetsAt: w.ResetsAt}
}
