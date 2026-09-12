package hub

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/sessionlog"
	"many-ai-cli/internal/wrapper"
)

type spawnWrappedSpec struct {
	Context        context.Context
	Provider       string
	CWD            string
	Model          string
	ModelSelection string
	RiskConfirmed  bool
	Label          string
	PermissionMode string
	Sandbox        string
	AskForApproval string
	// AllowedTools は段 2（bounded）で provider へ渡す許可 tool の一覧。
	// 空なら --allowed-tools を付けない＝この項目が存在しなかった頃と同じ argv。
	// 値の出所は internal/hub/child_permission.go の表と config だけ。
	AllowedTools []string
	Route        string
	Utf8Session  bool
	// Effort / ExecutionMode / PermissionPreset は起動要求の共通 3 項目
	// （子 plan: docs/local/plan_derived-session-launch_c1_request-schema.md）。
	// すべて省略可で、空文字は「指定なし」= この 3 項目が存在しなかった頃と
	// 完全に同じ挙動。受理値の表と検証は internal/config/effort.go が持つ。
	Effort           string
	ExecutionMode    string
	PermissionPreset string
	// InitialPrompt は headless 起動でのみ使う最初の指示。対話起動では空のまま
	// で、プロンプトは今までどおり登録後に PTY へ注入される（注入経路は 1 本の
	// まま）。headless には注入する相手（プロンプト待ちの TUI）が無いので、
	// 起動時に argv / stdin で渡すしかない
	// （子 plan: docs/local/plan_child_execution_modes_headless.md 内部 C3）。
	InitialPrompt string
	// SubscriptionProfileID は起動に使うサブスクリプション profile。
	// 空なら CLI 自身のログイン環境をそのまま使う（従来動作）。
	SubscriptionProfileID string
	// SubscriptionLogin が true のとき、通常のセッションではなく公式 CLI の
	// ログインフロー（`claude auth login` 等）を PTY で走らせる。
	SubscriptionLogin bool
	// UsageProbe marks a short-lived internal session that must not be exposed
	// in the normal UI session feed.
	UsageProbe bool
}

const manyAICLIBinEnv = "MANY_AI_CLI_BIN"

// hubSpawnEnv gives every Hub-spawned session the exact executable that
// launched it. This avoids resolving an older distribution from PATH when an
// orchestration conductor invokes a newer subcommand such as relay.
func hubSpawnEnv(base []string, hubPort int, exe string) []string {
	return mergeEnvOverrides(sanitizeEnv(base), []string{
		"MANY_AI_CLI=1",
		fmt.Sprintf("MANY_AI_CLI_HUB_PORT=%d", hubPort),
		manyAICLIBinEnv + "=" + exe,
	})
}

func appendOpenCodePermissionArgs(wrapArgs []string, permissionMode string) []string {
	if permissionMode != "" && permissionMode != "default" {
		return append(wrapArgs, "--permission-mode", permissionMode)
	}
	return wrapArgs
}

func (s *Server) spawnWrappedSession(spec spawnWrappedSpec, wait time.Duration) (int, error) {
	if !validOrchestrationProvider(spec.Provider) {
		return 0, fmt.Errorf("invalid provider")
	}
	// profile は env を組み立てる前に解決する。存在しない / 無効化された profile を
	// 指定された場合はここで失敗させ、**別アカウントで黙って起動しない**。
	subEnv, _, subErr := s.subscriptionLaunch(spec.Provider, spec.SubscriptionProfileID)
	if subErr != nil {
		return 0, subErr
	}
	if strings.HasPrefix(spec.Model, "-") || strings.HasPrefix(spec.Label, "-") || !spawnValidModelLabel(spec.Model) || !spawnValidModelLabel(spec.Label) {
		return 0, fmt.Errorf("invalid model or label value")
	}
	wrapArgs := []string{"wrap", spec.Provider}
	resolvedModel := strings.TrimSpace(spec.Model)
	if spec.Label != "" {
		wrapArgs = append(wrapArgs, "--label="+spec.Label)
	}
	if spec.SubscriptionLogin {
		// ログイン用の使い捨てセッション。公式 CLI の login サブコマンドは
		// --model / --permission-mode 等を受け付けないので、通常の provider 別
		// フラグ組み立てを丸ごと飛ばす。
		wrapArgs = append(wrapArgs, "--subscription-login")
		return s.startWrapProcess(spec, wrapArgs, subEnv, "", wait)
	}
	switch spec.Provider {
	case "claude":
		mode := spec.ModelSelection
		if mode == "" {
			mode = "auto"
		}
		currentModel := s.getLastModel("claude")
		risk := evaluateClaudeRisk(currentModel, resolvedModel, spec.PermissionMode)
		if risk.HighRisk && mode != "required" {
			mode = "required"
		}
		if spawnNeedsRiskConfirmation(spec, mode == "required") {
			return 0, fmt.Errorf("risk confirmation required")
		}
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
		if spec.PermissionMode != "" && spec.PermissionMode != "default" {
			wrapArgs = append(wrapArgs, "--permission-mode", spec.PermissionMode)
		}
	case "codex":
		mode := spec.ModelSelection
		if mode == "" {
			mode = "auto"
		}
		currentModel := s.getLastModel("codex")
		risk := evaluateCodexRisk(currentModel, resolvedModel, spec.Sandbox, spec.AskForApproval)
		if risk.HighRisk && mode != "required" {
			mode = "required"
		}
		if spawnNeedsRiskConfirmation(spec, mode == "required") {
			return 0, fmt.Errorf("risk confirmation required")
		}
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
		if spec.Sandbox != "" {
			wrapArgs = append(wrapArgs, "--sandbox", spec.Sandbox)
		}
		if spec.AskForApproval != "" {
			wrapArgs = append(wrapArgs, "--ask-for-approval", spec.AskForApproval)
		}
	case "opencode":
		risk := evaluateOpenCodeRisk(spec.PermissionMode)
		if spawnNeedsRiskConfirmation(spec, risk.HighRisk) {
			return 0, fmt.Errorf("risk confirmation required")
		}
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
		wrapArgs = appendOpenCodePermissionArgs(wrapArgs, spec.PermissionMode)
	default:
		if spec.Provider != "shell" {
			// grok / copilot / cursor-agent の全許可は Claude / OpenCode と同じく確認必須。
			risk := evaluateBypassPermissionRisk(spec.PermissionMode)
			if spawnNeedsRiskConfirmation(spec, risk.HighRisk) {
				return 0, fmt.Errorf("risk confirmation required")
			}
		}
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
		// copilot / cursor-agent / grok も permission mode を wrapper へ渡す
		// （wrapper 側で各 CLI の承認バイパス指定に変換される）。shell は AI 固有フラグを使わない。
		if spec.Provider != "shell" && spec.PermissionMode != "" && spec.PermissionMode != "default" {
			wrapArgs = append(wrapArgs, "--permission-mode", spec.PermissionMode)
		}
	}
	// effort は provider 別 switch の外で 1 度だけ足す。どの provider が effort を
	// 受けるかは internal/config/effort.go の表だけが知っている。写像が無い
	// provider では spec.Effort が空（検証で弾かれている）なので何も足さない。
	if args := effortWrapArgs(spec.Provider, spec.Effort); len(args) > 0 {
		wrapArgs = append(wrapArgs, args...)
	}
	// 段 2 の許可 tool も同じく switch の外で 1 度だけ。空なら何も足さない。
	if args := allowedToolsWrapArgs(spec.AllowedTools); len(args) > 0 {
		wrapArgs = append(wrapArgs, args...)
	}
	// headless は provider 別 switch の外。ここまでで組んだ model / effort /
	// 権限のフラグはそのまま使い（headless 専用の写像を作らない・親 plan からの
	// 差分 5）、実行モードを 1 組のフラグで足すだけにする。
	headlessArgs, promptPath, headlessErr := s.headlessWrapArgs(spec.ExecutionMode, spec.InitialPrompt)
	if headlessErr != nil {
		return 0, headlessErr
	}
	wrapArgs = append(wrapArgs, headlessArgs...)
	effectiveRoute := spec.Route
	if effectiveRoute == "" {
		localCfg := s.snapshotLocalModels()
		known := collectOllamaModelIDs(s.modelsCache, localCfg)
		knownLmStudio := collectLMStudioModelIDs(s.modelsCache)
		effectiveRoute = RouteForModel(spec.Provider, resolvedModel, known, knownLmStudio)
	}
	if spec.Provider == "codex" && isLocalRoute(effectiveRoute) {
		wrapArgs = append(wrapArgs, "--codex-oss")
	}
	if spec.Utf8Session {
		wrapArgs = append(wrapArgs, "--utf8")
	}

	id, err := s.startWrapProcess(spec, wrapArgs, subEnv, effectiveRoute, wait)
	if err != nil {
		// The wrapper never started, so nothing will ever read the prompt
		// file. Take it back rather than leaving the user's own instruction
		// sitting in a temp directory (internal/doctor/residue.go の規律).
		removeHeadlessPromptFile(promptPath)
		return 0, err
	}
	if spawnRecordsLastModel(spec, resolvedModel, effectiveRoute) {
		_ = s.setLastModel(spec.Provider, resolvedModel)
	}
	return id, nil
}

// startWrapProcess launches `many-ai-cli wrap <provider> …` and waits for the
// resulting session to register. It is shared by the ordinary orchestration
// spawn and by the subscription login spawn, which needs the same environment,
// log, and process-attribute handling but none of the model/permission flags.
func (s *Server) startWrapProcess(spec spawnWrappedSpec, wrapArgs, subEnv []string, effectiveRoute string, wait time.Duration) (int, error) {
	exe, err := os.Executable()
	if err != nil {
		return 0, fmt.Errorf("executable error: %w", err)
	}
	cmd := exec.Command(exe, wrapArgs...)
	cmd.Dir = spec.CWD
	hubPort := s.currentHubPort()
	cmd.Env = hubSpawnEnv(os.Environ(), hubPort, exe)
	probeValue := "0"
	if spec.UsageProbe {
		probeValue = "1"
	}
	cmd.Env = mergeEnvOverrides(cmd.Env, []string{"MANY_AI_CLI_USAGE_PROBE=" + probeValue})
	loginValue := "0"
	if spec.SubscriptionLogin {
		loginValue = "1"
	}
	cmd.Env = mergeEnvOverrides(cmd.Env, []string{"MANY_AI_CLI_SUBSCRIPTION_LOGIN=" + loginValue})
	if s.parentShell != "" {
		cmd.Env = append(cmd.Env, "MANY_AI_CLI_PARENT_SHELL="+s.parentShell)
	}
	s.cfgMu.Lock()
	ollamaBaseURL := s.cfg.Ollama.BaseURL
	lmStudioBaseURL := s.cfg.LMStudio.BaseURL
	s.cfgMu.Unlock()
	if envPreset := EnvPresetForWithOllamaBase(spec.Provider, effectiveRoute, ollamaBaseURL, lmStudioBaseURL); len(envPreset) > 0 {
		cmd.Env = mergeEnvOverrides(cmd.Env, envPreset)
	}
	if len(subEnv) > 0 {
		cmd.Env = mergeEnvOverrides(cmd.Env, subEnv)
	}

	var stdinNull, spawnLog *os.File
	if f, devErr := os.OpenFile(os.DevNull, os.O_RDWR, 0); devErr == nil {
		stdinNull = f
		cmd.Stdin = stdinNull
	}
	spawnLogPath := filepath.Join(s.cfg.Hub.LogDir, "spawn", fmt.Sprintf("%s-%s.log", spec.Provider, time.Now().Format("20060102-150405.000")))
	if err := os.MkdirAll(filepath.Dir(spawnLogPath), sessionlog.PrivateDirMode); err == nil {
		if f, logErr := os.OpenFile(spawnLogPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, sessionlog.PrivateFileMode); logErr == nil {
			spawnLog = f
			cmd.Stdout = spawnLog
			cmd.Stderr = spawnLog
		}
	}
	setCmdSysProcAttr(cmd)
	if err := cmd.Start(); err != nil {
		if stdinNull != nil {
			_ = stdinNull.Close()
		}
		if spawnLog != nil {
			_ = spawnLog.Close()
		}
		return 0, err
	}
	s.safeGo("spawn_child_wait", func() {
		_ = cmd.Wait()
		if stdinNull != nil {
			_ = stdinNull.Close()
		}
		if spawnLog != nil {
			_ = spawnLog.Close()
		}
	})
	if spec.UsageProbe {
		s.updateUsageProbePID(spec.Label, cmd.Process.Pid)
	}
	id, err := s.waitForSessionByLabelContext(spec.Context, spec.Label, wait)
	if err != nil {
		// A wrapper that never registers has no Hub session to dismiss. Kill the
		// process after the bounded wait so spawn-child cannot leave an orphan
		// provider terminal behind.
		terminateUnregisteredWrap(cmd)
		return 0, err
	}
	return id, nil
}

func terminateUnregisteredWrap(cmd *exec.Cmd) {
	if cmd == nil || cmd.Process == nil {
		return
	}
	_ = cmd.Process.Kill()
}

func (s *Server) waitForSessionByLabelContext(ctx context.Context, label string, timeout time.Duration) (int, error) {
	if ctx == nil {
		ctx = context.Background()
	}
	if timeout <= 0 {
		timeout = time.Second
	}
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	deadline := time.NewTimer(timeout)
	defer deadline.Stop()
	for {
		s.sessionsMu.Lock()
		for id, ses := range s.sessions {
			if ses.Label == label {
				s.sessionsMu.Unlock()
				return id, nil
			}
		}
		s.sessionsMu.Unlock()
		select {
		case <-ctx.Done():
			return 0, ctx.Err()
		case <-deadline.C:
			return 0, context.DeadlineExceeded
		case <-ticker.C:
		}
	}
}

// validSpawnProvider reports whether provider is spawnable: a built-in AI
// CLI, "shell", or a config.yaml custom_providers entry. Custom providers go
// through config.EffectiveCustomProviders, so a broken or built-in-colliding
// entry (see internal/config/custom_provider.go) is rejected here exactly as
// it is dropped from the spawn dropdown in /api/info (misc_handlers.go).
//
// Kept in sync with web/src/app/spawn-panel.ts's injectCustomProviderOptions
// and with the native <option> values the spawn form already ships with —
// this is the check plan_provider-user-config.md's C2 needed to actually let
// a custom provider spawn, not just appear in the dropdown (敵対レビュー
// 2026-08-31 Finding 1: this switch used to be a fixed built-in list only).
func (s *Server) validSpawnProvider(provider string) bool {
	switch provider {
	case "claude", "codex", "copilot", "cursor-agent", "opencode", "grok", "command-code", "shell":
		return true
	}
	s.cfgMu.Lock()
	custom := s.cfg.CustomProviders
	s.cfgMu.Unlock()
	for _, p := range config.EffectiveCustomProviders(custom) {
		if p.ID == provider {
			return true
		}
	}
	return false
}

// validGridAIProvider reports whether provider is valid as the AI half of
// handleSpawnGrid's "ai+shell" preset: any provider validSpawnProvider
// accepts, except "shell" itself (that half of the pair is fixed to shell
// sessions already, so allowing "shell" here would be a self-referential no-op).
// Built-in providers, and now custom_providers entries too
// (plan_custom-provider-spawn-execution.md C2), both qualify.
func (s *Server) validGridAIProvider(provider string) bool {
	return provider != "shell" && s.validSpawnProvider(provider)
}

// resolveSpawnModel decides the --model value handleSpawn passes downstream.
// A custom provider gets none: built-in per-provider args/env injection
// (RouteForModel, Ollama/LM Studio routing, ANTHROPIC_*/OPENAI_* presets)
// only makes sense for the providers many-ai-cli knows the shape of, and
// none of it is meant to reach an arbitrary user CLI (decision 2,
// plan_custom-provider-spawn-execution.md). Extracted as its own function so
// this suppression is unit-testable without spawning a real wrap process.
func resolveSpawnModel(model string, isCustomProvider bool) string {
	if isCustomProvider {
		return ""
	}
	return strings.TrimSpace(model)
}

// validateLaunchRequestOptions trims and validates the three launch-request
// options shared by /api/spawn, spawn-child and the relay roles (子 plan:
// docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C1). It
// mutates the values in place so every entry point stores the same trimmed
// form. All three are optional: an empty value passes for every provider,
// which is what keeps callers that never send them unchanged.
//
// The decision of which providers accept an effort level, and which execution
// modes / permission presets this build can honour, lives in
// internal/config/effort.go — this function must not grow a provider switch.
func validateLaunchRequestOptions(provider string, effort, mode, preset *string) error {
	*effort = strings.TrimSpace(*effort)
	*mode = config.NormalizeExecutionMode(*mode)
	*preset = config.NormalizePermissionPreset(*preset)
	if err := config.ValidateEffort(provider, *effort); err != nil {
		return err
	}
	if err := config.ValidateExecutionMode(*mode); err != nil {
		return err
	}
	return config.ValidatePermissionPreset(*preset)
}

// effortWrapArgs returns the `wrap` flags that carry effort, or nil when
// effort is empty or the provider has no mapping. Keeping it a helper (rather
// than inlining config.EffortArgs at both call sites) means the two argument
// builders in this file stay identical, and the wrapper flag name is written
// down once.
// allowedToolsWrapArgs returns the `wrap` flag carrying the bounded tier's
// allowlist, or nil when there is none — so a launch without one produces
// exactly the argv it did before the tier existed.
//
// The values are re-validated here even though they came from the table and
// config.yaml: they end up in a child process's argument list, and on Windows
// some launches pass through a cmd.exe shim. An entry that does not look like a
// tool name or command pattern is dropped rather than passed on (config.Warnings
// already told the user about it when it came from their config file).
func allowedToolsWrapArgs(values []string) []string {
	kept := make([]string, 0, len(values))
	for _, value := range values {
		value = strings.TrimSpace(value)
		if config.ValidAllowedToolValue(value) {
			kept = append(kept, value)
		}
	}
	if len(kept) == 0 {
		return nil
	}
	return []string{"--allowed-tools", strings.Join(kept, ",")}
}

// headlessWrapArgs returns the `wrap` flags that make this launch headless, and
// the path of the prompt file it wrote. Both are empty for every other launch,
// so an interactive spawn produces exactly the argv it produced before this
// mode existed (親 plan 不変条件 1).
//
// The prompt is handed over as a file rather than as an argument because an
// argument is visible to every process on the machine (a process list), ends up
// in shell history where one is involved, and is bounded by the OS argv limit —
// while the prompt is the user's own work instruction, often long. The wrapper
// reads the file once and deletes it (takePromptFile), so the two halves are:
// the Hub writes, the wrapper collects.
//
// An empty prompt is allowed: a headless launch with no instruction is a
// no-instruction run, not an error, and the file is simply not written.
func (s *Server) headlessWrapArgs(executionMode, prompt string) ([]string, string, error) {
	if !config.IsHeadlessExecutionMode(executionMode) {
		return nil, "", nil
	}
	args := []string{"--headless"}
	if strings.TrimSpace(prompt) == "" {
		return args, "", nil
	}
	path, err := writeHeadlessPromptFile(prompt)
	if err != nil {
		return nil, "", err
	}
	return append(args, "--prompt-file", path), path, nil
}

// writeHeadlessPromptFile writes one launch's prompt into ~/.many-ai-cli/tmp
// with the same private modes as every other file many-ai-cli owns, and returns
// its path.
//
// The name is random so two launches cannot collide and so the file name itself
// says nothing about the work. The contents are never logged here or anywhere
// else on this path.
func writeHeadlessPromptFile(prompt string) (string, error) {
	dir, err := config.Dir()
	if err != nil {
		return "", err
	}
	dir = filepath.Join(dir, "tmp")
	if err := os.MkdirAll(dir, sessionlog.PrivateDirMode); err != nil {
		return "", fmt.Errorf("headless prompt dir: %w", err)
	}
	// Collect anything an earlier run left behind before writing a new one. The
	// wrapper deletes its own prompt file, but a kill skips that, and a leftover
	// here is the user's own instruction sitting on disk — so the cleanup gets a
	// path that does not depend on a graceful exit (the rule in
	// internal/doctor/residue.go, applied to our own directory).
	sweepStaleHeadlessPrompts(dir, time.Now())
	var raw [16]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "", fmt.Errorf("headless prompt name: %w", err)
	}
	path := filepath.Join(dir, "prompt-"+hex.EncodeToString(raw[:])+".md")
	if err := os.WriteFile(path, []byte(prompt), sessionlog.PrivateFileMode); err != nil {
		return "", fmt.Errorf("headless prompt file: %w", err)
	}
	return path, nil
}

func removeHeadlessPromptFile(path string) {
	if strings.TrimSpace(path) == "" {
		return
	}
	_ = os.Remove(path)
}

// headlessPromptMaxAge is how long a prompt file may sit unread before it is
// treated as abandoned. A wrapper reads its file within seconds of starting;
// an hour is long enough that a slow machine is never mistaken for a crash.
const headlessPromptMaxAge = time.Hour

// sweepStaleHeadlessPrompts deletes prompt files older than headlessPromptMaxAge
// from dir. It only ever touches the names this package writes.
func sweepStaleHeadlessPrompts(dir string, now time.Time) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	for _, entry := range entries {
		name := entry.Name()
		if entry.IsDir() || !strings.HasPrefix(name, "prompt-") || !strings.HasSuffix(name, ".md") {
			continue
		}
		info, err := entry.Info()
		if err != nil {
			continue
		}
		if now.Sub(info.ModTime()) > headlessPromptMaxAge {
			_ = os.Remove(filepath.Join(dir, name))
		}
	}
}

func effortWrapArgs(provider, effort string) []string {
	if effort == "" {
		return nil
	}
	if _, ok := config.EffortSupportFor(provider); !ok {
		return nil
	}
	return []string{"--effort", effort}
}

func (s *Server) handleSpawn(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Provider        string `json:"provider"`
		CWD             string `json:"cwd"`
		Model           string `json:"model"`
		ModelSelection  string `json:"model_selection_mode"`
		RiskConfirmed   bool   `json:"risk_confirmed"`
		Label           string `json:"label"`
		PermissionMode  string `json:"permission_mode"`
		Sandbox         string `json:"sandbox"`
		AskForApproval  string `json:"ask_for_approval"`
		Route           string `json:"route"`
		Utf8Session     bool   `json:"utf8_session"`
		IsolateWorktree *bool  `json:"isolate_worktree"`
		WorktreeCleanup string `json:"worktree_cleanup"`
		// Delegation は「子セッションへ委譲できることを AI に伝える」の指定。
		// nil は「画面が指定しなかった」で、config の user_prefs.spawn.delegation_auto を使う
		// （isolate_worktree と同じ扱い）。委譲できるかどうかではなく、AI が知るかどうかを
		// 決めるだけ。正本は internal/wrapper/delegation.go の冒頭。
		Delegation *bool `json:"delegation"`
		// SubscriptionProfileID は使用するサブスクリプション profile。
		// 省略・空文字は「Default CLI login」で、従来のリクエストと同一の挙動になる。
		SubscriptionProfileID string `json:"subscription_profile_id"`
		// Effort / ExecutionMode / PermissionPreset は起動要求の共通 3 項目
		// （子 plan: docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C1）。
		// 省略時（空文字）はこの 3 項目を知らない呼び出し元と 1 バイトも変わらない。
		// 受理値は internal/config/effort.go の表が持つ。
		Effort           string `json:"effort"`
		ExecutionMode    string `json:"execution_mode"`
		PermissionPreset string `json:"permission_preset"`
		// C1: plan_orchestration-spawn-ui-exposure.md — ツールバーの「オーケストレーション」
		// ボタン経由の起動でのみ true。詳細設定アコーディオンで役割を設定した場合のみ
		// OrchestrationRoles が埋まる（未設定ロールは省略 or nil）。
		Orchestration      bool                                    `json:"orchestration"`
		OrchestrationRoles map[string]*orchestrationRoleAssignment `json:"orchestration_roles"`
		// InitialPrompt (plan_session-handoff-board_c4_prompted-spawn.md C1):
		// 画面から渡す最初の指示。空文字（省略）は「従来どおり何も注入しない」で、
		// この分岐に一切入らない。配送は spawn-child が使っている
		// injectInitialPrompt を共有する（新しい配送方式を作らない）。
		InitialPrompt string `json:"initial_prompt"`
		// HandoffFrom (plan_session-handoff-board_c5_handoff-md.md 内部 C3):
		// 引き継ぎ元セッションの ID。看板 UI からの起動でのみ渡され、それ以外の
		// /api/spawn 呼び出しは省略（0）のままで従来と 1 バイトも変わらない。
		HandoffFrom int `json:"handoff_from"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if !s.validSpawnProvider(body.Provider) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid provider")
		return
	}
	// custom provider かどうかは以降で何度か使う（subscription 早期拒否・
	// model/route 抑止）。同じ判定を s.cfgMu 越しに何度も取らないよう1回だけ引く。
	s.cfgMu.Lock()
	isCustomProvider := s.cfg.IsCustomProviderID(body.Provider)
	s.cfgMu.Unlock()
	// dontAsk は Claude / Grok が --permission-mode としてそのまま解釈する値。
	// 既存の変換関数を通らない provider では today の "default" と同じく無視される
	// （子 plan: plan_derived-session-launch_c1_request-schema.md 内部 C1）。
	validPermModes := map[string]bool{
		"": true, "default": true, "plan": true,
		"acceptEdits": true, "auto": true, "bypassPermissions": true,
		"dontAsk": true,
	}
	validSandboxes := map[string]bool{
		"": true, "read-only": true, "workspace-write": true, "danger-full-access": true,
	}
	validApprovals := map[string]bool{
		"": true, "untrusted": true, "on-request": true, "never": true,
	}
	validModelSelection := map[string]bool{
		"": true, "auto": true, "explicit": true, "required": true,
	}
	if !validPermModes[body.PermissionMode] || !validSandboxes[body.Sandbox] || !validApprovals[body.AskForApproval] || !validModelSelection[body.ModelSelection] {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "bad request")
		return
	}
	if !validRoute(body.Route) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid route")
		return
	}
	// 起動要求の共通 3 項目。空文字は「指定なし」なので、この検証は 3 項目を
	// 送ってこない既存の呼び出しでは一切失敗しない。
	if err := validateLaunchRequestOptions(body.Provider, &body.Effort, &body.ExecutionMode, &body.PermissionPreset); err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
		return
	}
	// 実行モードを provider ごとに解決する。/api/spawn は画面の起動口なので
	// origin は常に ui 扱い＝ auto は対話のまま。**明示 headless だけが headless**
	// で、その provider に定義が無ければ 400（黙って対話へ倒さない・親 plan D2）。
	// 省略した呼び出しは空のまま素通りする（子 plan:
	// docs/local/plan_child_execution_modes_headless.md 内部 C1）。
	resolvedExecutionMode, execModeErr := config.ResolveExecutionMode(
		body.ExecutionMode, headlessCapableProvider(body.Provider, s.snapshotCfg()), launchOriginUI, false)
	if execModeErr != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", execModeErr.Error())
		return
	}
	body.ExecutionMode = resolvedExecutionMode
	if !validWorktreeCleanup(body.WorktreeCleanup) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid worktree cleanup policy")
		return
	}
	if body.HandoffFrom < 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid handoff_from")
		return
	}
	cwd := body.CWD
	if cwd == "" {
		cwd = s.hubCWD
	} else {
		// cwd が実在するディレクトリであることを確認する。
		info, statErr := os.Stat(cwd)
		if statErr != nil || !info.IsDir() {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "cwd does not exist or is not a directory")
			return
		}
	}

	// model / label の先頭 "-" はフラグ偽装を防ぐために禁止する。
	if strings.HasPrefix(body.Model, "-") {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid model value")
		return
	}
	if strings.HasPrefix(body.Label, "-") {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid label value")
		return
	}
	// シェルメタ文字・制御文字を弾く（Windows の cmd.exe /c シム経路への引数注入の多層防御）。
	if !spawnValidModelLabel(body.Model) || !spawnValidModelLabel(body.Label) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid model or label value")
		return
	}
	// ドライブ/FS ルートやホーム親ディレクトリ自身は cwd として弾く（AI がホーム配下を巻き込む事故防止）。
	if spawnCwdTooBroad(cwd) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "cwd is too broad (system root or home root)")
		return
	}

	// subscription profile は起動処理へ入る前に解決する。存在しない / 無効化された
	// profile を指定された場合は 400 で止め、**既定ログインへ黙って倒れない**。
	// 誤ったアカウントで走ることの方が、起動できないことより重い。
	if strings.TrimSpace(body.SubscriptionProfileID) != "" && body.Provider == "shell" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "shell sessions do not use subscription profiles")
		return
	}
	if strings.TrimSpace(body.SubscriptionProfileID) != "" && isCustomProvider {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "custom providers do not use subscription profiles")
		return
	}
	subEnv, _, subErr := s.subscriptionLaunch(body.Provider, body.SubscriptionProfileID)
	if subErr != nil {
		writeJSONError(w, http.StatusBadRequest, "invalid_subscription", errorDetail("subscription profile error", subErr))
		return
	}

	// P-33 C2: ordinary sessions can opt into a dedicated worktree. Keep the
	// metadata in the existing pending-registration path so the session is
	// associated with the isolated directory only after its wrapper connects.
	isolateWorktree := false
	cleanupPolicy := body.WorktreeCleanup
	s.cfgMu.Lock()
	if body.IsolateWorktree == nil {
		isolateWorktree = s.cfg.UserPrefs.Spawn.WorktreeAuto
	} else {
		isolateWorktree = *body.IsolateWorktree
	}
	if cleanupPolicy == "" {
		cleanupPolicy = s.cfg.UserPrefs.Spawn.WorktreeCleanup
	}
	delegation := s.cfg.UserPrefs.Spawn.DelegationAuto
	if body.Delegation != nil {
		delegation = *body.Delegation
	}
	s.cfgMu.Unlock()
	cleanupPolicy = effectiveWorktreeCleanup(cleanupPolicy)
	var isolated normalWorktree
	spawnStarted := false
	if isolateWorktree {
		if body.Label == "" {
			body.Label = fmt.Sprintf("worktree-%d", time.Now().UnixNano())
		}
		var worktreeErr error
		isolated, worktreeErr = prepareNormalWorktree(cwd, body.Label, time.Now())
		if worktreeErr != nil {
			writeJSONError(w, http.StatusBadRequest, "worktree_error", errorDetail("worktree error", worktreeErr))
			return
		}
		cwd = isolated.Path
		s.orchestration.mu.Lock()
		s.orchestration.pending[body.Label] = pendingChild{NormalWorktree: isolated, WorktreeCleanup: cleanupPolicy, WorktreeBranch: isolated.Branch, SpawnedAt: time.Now()}
		s.orchestration.mu.Unlock()
	}
	cleanupIsolated := func() {
		if !isolated.Created {
			return
		}
		if err := cleanupNormalWorktree(isolated, worktreeCleanupDelete); err != nil {
			s.logger.Warn("failed to clean up unregistered worktree", "path", isolated.Path, "err", err)
		}
	}
	defer func() {
		if spawnStarted || !isolated.Created {
			return
		}
		s.orchestration.mu.Lock()
		delete(s.orchestration.pending, body.Label)
		s.orchestration.mu.Unlock()
		cleanupIsolated()
	}()

	// C1: plan_orchestration-spawn-ui-exposure.md — オーケストレーション起動の場合、
	// 起動時点で conductor 用の orchestration_id を予約する。label が未指定なら生成して
	// 以降の wrapArgs 組み立て（--label=...）にもそのまま乗せる。
	var orchestrationID string
	if body.Orchestration {
		roles := make(map[string]orchestrationRoleAssignment, len(body.OrchestrationRoles))
		for role, ra := range body.OrchestrationRoles {
			if ra == nil {
				continue
			}
			if ra.Provider != "" && !validOrchestrationProvider(ra.Provider) {
				writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid orchestration role provider")
				return
			}
			if strings.HasPrefix(ra.Model, "-") || !spawnValidModelLabel(ra.Model) {
				writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid orchestration role model")
				return
			}
			roles[role] = *ra
		}
		if body.Label == "" {
			body.Label = fmt.Sprintf("orch-conductor-%d", time.Now().UnixNano())
		}
		orchestrationID = s.reserveOrchestrationConductor(body.Label, roles)
	}

	// C1 (plan_session-handoff-board_c4_prompted-spawn.md): initial_prompt が
	// 空文字なら spawnPendingLabelForInitialPrompt が "" を返し、この if に
	// 入らない。label の生成も pending への書き込みも一切起きないので、
	// 従来の /api/spawn（initial_prompt を知らない呼び出し元）と 1 バイトも
	// 挙動が変わらない。
	initialPrompt := sanitizeSpawnInitialPrompt(body.InitialPrompt)
	headlessSpawn := config.IsHeadlessExecutionMode(body.ExecutionMode)
	if lbl := spawnPendingLabelForInitialPrompt(body.Label, initialPrompt, time.Now()); lbl != "" {
		body.Label = lbl
		s.orchestration.mu.Lock()
		// isolate_worktree や orchestration=true が同じ label で既に pending
		// entry を作っていることがあるので、reserveOrchestrationConductor と
		// 同じく上書きせずマージする。
		meta := s.orchestration.pending[body.Label]
		// headless では注入しない（注入する相手が無い）。文面は起動時に
		// --prompt-file で渡す。pending 自体は残すので、引き継ぎ元の記録
		// （HandoffFrom）は対話起動と同じように残る
		// （子 plan: docs/local/plan_child_execution_modes_headless.md 内部 C3）。
		if !headlessSpawn {
			meta.InitialPrompt = initialPrompt
		}
		meta.HandoffFrom = body.HandoffFrom
		if meta.SpawnedAt.IsZero() {
			meta.SpawnedAt = time.Now()
		}
		s.orchestration.pending[body.Label] = meta
		s.orchestration.mu.Unlock()
	}

	exe, err := os.Executable()
	if err != nil {
		cleanupIsolated()
		writeJSONError(w, http.StatusInternalServerError, "executable_error", errorDetail("executable error", err))
		return
	}
	wrapArgs := []string{"wrap", body.Provider}
	resolvedModel := resolveSpawnModel(body.Model, isCustomProvider)

	if body.Label != "" {
		// --label=value 形式で渡す（空白区切りだと value が次フラグに化ける可能性がある）。
		wrapArgs = append(wrapArgs, "--label="+body.Label)
	}

	// Shell は AI 固有フラグ (model / route / permission) を使わない。
	// 以下の switch / effectiveRoute / EnvPresetFor / setLastModel を全てスキップ
	// するため、shell の場合は早期パスで exec.Command まで飛ばす。
	if body.Provider == "shell" {
		if body.Utf8Session {
			wrapArgs = append(wrapArgs, "--utf8")
		}
		hubPort := s.currentHubPort()
		cmd := exec.Command(exe, wrapArgs...)
		cmd.Dir = cwd
		cmd.Env = hubSpawnEnv(os.Environ(), hubPort, exe)
		if s.parentShell != "" {
			cmd.Env = append(cmd.Env, "MANY_AI_CLI_PARENT_SHELL="+s.parentShell)
		}
		var stdinNull, spawnLog *os.File
		if f, devErr := os.OpenFile(os.DevNull, os.O_RDWR, 0); devErr == nil {
			stdinNull = f
			cmd.Stdin = stdinNull
		} else {
			s.logger.Warn("spawn: failed to open os.DevNull for stdin (shell)", "err", devErr)
		}
		spawnLogPath := filepath.Join(s.cfg.Hub.LogDir, "spawn",
			fmt.Sprintf("%s-%s.log", body.Provider, time.Now().Format("20060102-150405.000")))
		if err := os.MkdirAll(filepath.Dir(spawnLogPath), sessionlog.PrivateDirMode); err == nil {
			if f, logErr := os.OpenFile(spawnLogPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, sessionlog.PrivateFileMode); logErr == nil {
				spawnLog = f
				cmd.Stdout = spawnLog
				cmd.Stderr = spawnLog
			} else {
				s.logger.Warn("spawn: failed to create spawn log file (shell)", "path", spawnLogPath, "err", logErr)
			}
		}
		setCmdSysProcAttr(cmd)
		if err := cmd.Start(); err != nil {
			cleanupIsolated()
			if stdinNull != nil {
				_ = stdinNull.Close()
			}
			if spawnLog != nil {
				_ = spawnLog.Close()
			}
			writeJSONError(w, http.StatusInternalServerError, "spawn_error", errorDetail("spawn error", err))
			return
		}
		spawnStarted = true
		s.logger.Debug("spawn: wrap process started",
			"provider", body.Provider, "pid", cmd.Process.Pid, "spawn_log", spawnLogPath)
		s.safeGo("spawn_wait", func() {
			waitErr := cmd.Wait()
			exitCode := 0
			if cmd.ProcessState != nil {
				exitCode = cmd.ProcessState.ExitCode()
			}
			s.logger.Debug("spawn: wrap process exited",
				"provider", body.Provider, "exit_code", exitCode, "wait_err", fmt.Sprintf("%v", waitErr))
			if stdinNull != nil {
				_ = stdinNull.Close()
			}
			if spawnLog != nil {
				_ = spawnLog.Close()
			}
		})
		writeJSON(w, map[string]bool{"ok": true})
		return
	}

	// permission_preset が明示されたときだけ、子セッションと同じ表
	// （internal/hub/child_permission.go）で段を解決し、空欄を埋める。明示値が
	// 優先で、preset が空なら何も埋めない＝この項目を知らない呼び出しと 1 バイトも
	// 変わらない。**RiskConfirmed はここでは触らない**: 高リスク権限の確認は人が
	// 押す経路の歯止めで、preset という近道で自動的に通してよいものではない
	// （埋めた結果が高リスクなら、下の switch がいつもどおり確認を要求する）。
	var allowedTools []string
	if body.PermissionPreset != "" {
		preset := permissionPresetApproval(body.Provider, body.PermissionPreset, s.snapshotCfg().Orchestration)
		if body.PermissionMode == "" {
			body.PermissionMode = preset.PermissionMode
		}
		if body.Sandbox == "" {
			body.Sandbox = preset.Sandbox
		}
		if body.AskForApproval == "" {
			body.AskForApproval = preset.AskForApproval
		}
		allowedTools = preset.AllowedTools
	}
	switch body.Provider {
	case "claude":
		mode := body.ModelSelection
		if mode == "" {
			mode = "auto"
		}
		// モデル未選択（auto）のときは last_model を --model へ再注入しない。
		// 以前は前回モデルを復活させていたが、それが claude CLI 側の既定モデル
		// （/model で選んだ 1M 窓モデルなど）を黙って上書きし、巨大コンテキストを
		// 200K へ縮める原因になっていた。明示選択が無ければ --model を付けず、
		// CLI 自身の既定（ユーザーが /model で決めた値）をそのまま尊重する。
		// last_model は risk 判定の基準値としてのみ参照する。
		currentModel := s.getLastModel("claude")
		risk := evaluateClaudeRisk(currentModel, resolvedModel, body.PermissionMode)
		if risk.HighRisk && mode != "required" {
			mode = "required"
		}
		if mode == "required" && !body.RiskConfirmed {
			writeJSONError(w, http.StatusBadRequest, "risk_confirmation_required", "risk confirmation required")
			return
		}
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
		if body.PermissionMode != "" && body.PermissionMode != "default" {
			wrapArgs = append(wrapArgs, "--permission-mode", body.PermissionMode)
		}
	case "codex":
		mode := body.ModelSelection
		if mode == "" {
			mode = "auto"
		}
		// claude 側と同じく、モデル未選択（auto）のときは last_model を
		// --model へ再注入しない。再注入は codex CLI 自身の既定モデルを黙って
		// 上書きしてしまうため。明示選択が無ければ --model を付けず CLI 既定を尊重する。
		// last_model は risk 判定の基準値としてのみ参照する。
		currentModel := s.getLastModel("codex")
		risk := evaluateCodexRisk(currentModel, resolvedModel, body.Sandbox, body.AskForApproval)
		if risk.HighRisk && mode != "required" {
			mode = "required"
		}
		if mode == "required" && !body.RiskConfirmed {
			writeJSONError(w, http.StatusBadRequest, "risk_confirmation_required", "risk confirmation required")
			return
		}
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
		if body.Sandbox != "" {
			wrapArgs = append(wrapArgs, "--sandbox", body.Sandbox)
		}
		if body.AskForApproval != "" {
			wrapArgs = append(wrapArgs, "--ask-for-approval", body.AskForApproval)
		}
	case "copilot":
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
	case "cursor-agent":
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
	case "opencode":
		risk := evaluateOpenCodeRisk(body.PermissionMode)
		if risk.HighRisk && !body.RiskConfirmed {
			writeJSONError(w, http.StatusBadRequest, "risk_confirmation_required", "risk confirmation required")
			return
		}
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
		wrapArgs = appendOpenCodePermissionArgs(wrapArgs, body.PermissionMode)
	case "grok":
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
	case "command-code":
		// 全許可（wrapper 側で --yolo に変換される）は Claude / OpenCode と同じく確認必須。
		// spawnWrappedSession の default 分岐と同じ判定を HTTP 経路にも置く。
		risk := evaluateBypassPermissionRisk(body.PermissionMode)
		if risk.HighRisk && !body.RiskConfirmed {
			writeJSONError(w, http.StatusBadRequest, "risk_confirmation_required", "risk confirmation required")
			return
		}
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
		if body.PermissionMode != "" && body.PermissionMode != "default" {
			wrapArgs = append(wrapArgs, "--permission-mode", body.PermissionMode)
		}
	}
	// effort は provider 別 switch の外で 1 度だけ足す（spawnWrappedSession と同じ形）。
	if args := effortWrapArgs(body.Provider, body.Effort); len(args) > 0 {
		wrapArgs = append(wrapArgs, args...)
	}
	if args := allowedToolsWrapArgs(allowedTools); len(args) > 0 {
		wrapArgs = append(wrapArgs, args...)
	}
	// headless も同じく switch の外。対話起動では 1 引数も増えない。
	headlessArgs, promptPath, headlessErr := s.headlessWrapArgs(body.ExecutionMode, initialPrompt)
	if headlessErr != nil {
		cleanupIsolated()
		writeJSONError(w, http.StatusInternalServerError, "spawn_error", errorDetail("headless prompt error", headlessErr))
		return
	}
	wrapArgs = append(wrapArgs, headlessArgs...)
	// route が未指定の場合は model 名から推定する。Anthropic / OpenAI の
	// 既定 route は env 注入を行わない（ユーザー shell の値を継承）。
	effectiveRoute := body.Route
	if effectiveRoute == "" {
		s.cfgMu.Lock()
		localCfg := append([]config.LocalModel(nil), s.cfg.LocalModels...)
		s.cfgMu.Unlock()
		known := collectOllamaModelIDs(s.modelsCache, localCfg)
		knownLmStudio := collectLMStudioModelIDs(s.modelsCache)
		effectiveRoute = RouteForModel(body.Provider, resolvedModel, known, knownLmStudio)
	}
	// Codex CLI は env (OPENAI_BASE_URL 等) だけでは provider を切り替えず、
	// CLI 引数 --oss / --profile で OSS (Ollama) provider に切替える設計。
	// route=ollama / lm-studio のときに --oss を渡さないと OpenAI 純正へ向かい認証エラーで落ちる。
	if body.Provider == "codex" && isLocalRoute(effectiveRoute) {
		wrapArgs = append(wrapArgs, "--codex-oss")
	}
	if body.Utf8Session {
		wrapArgs = append(wrapArgs, "--utf8")
	}
	hubPort := s.currentHubPort()
	cmd := exec.Command(exe, wrapArgs...)
	cmd.Dir = cwd
	cmd.Env = hubSpawnEnv(os.Environ(), hubPort, exe)
	// 画面の指定と config 既定を Hub 側で解決し、wrapper へは結論だけを渡す
	// （判定を 2 箇所に置かない）。0 も明示して、env が残った環境で意図せず ON にならないようにする。
	if delegation {
		cmd.Env = append(cmd.Env, wrapper.DelegationEnvName+"=1")
	} else {
		cmd.Env = append(cmd.Env, wrapper.DelegationEnvName+"=0")
	}
	if s.parentShell != "" {
		cmd.Env = append(cmd.Env, "MANY_AI_CLI_PARENT_SHELL="+s.parentShell)
	}
	s.cfgMu.Lock()
	ollamaBaseURL := s.cfg.Ollama.BaseURL
	lmStudioBaseURL := s.cfg.LMStudio.BaseURL
	s.cfgMu.Unlock()
	if envPreset := EnvPresetForWithOllamaBase(body.Provider, effectiveRoute, ollamaBaseURL, lmStudioBaseURL); len(envPreset) > 0 {
		cmd.Env = mergeEnvOverrides(cmd.Env, envPreset)
		s.logger.Debug("spawn: env preset applied",
			"provider", body.Provider, "route", effectiveRoute, "keys", envKeyList(envPreset))
	}
	if len(subEnv) > 0 {
		cmd.Env = mergeEnvOverrides(cmd.Env, subEnv)
		// キー名のみ出す（値はディレクトリパスと profile ID だが、env ダンプの
		// 習慣を作らないため envKeyList を通す）。
		s.logger.Debug("spawn: subscription profile applied",
			"provider", body.Provider, "keys", envKeyList(subEnv))
	}
	// Windows ConPTY (go-pty) は wrap プロセスの std handles が未設定だと
	// claude.exe / codex の起動に失敗してすぐ disconnect する。stdin は
	// os.DevNull、stdout/stderr は spawn ごとのログファイルに明示的にバインド
	// する。GUI から起動された Hub (コンソール無し) でも子プロセスの起動
	// 失敗時の panic / エラーメッセージを観測できるようにするため、
	// stdout/stderr は破棄せずファイルに残す。
	var stdinNull, spawnLog *os.File
	if f, devErr := os.OpenFile(os.DevNull, os.O_RDWR, 0); devErr == nil {
		stdinNull = f
		cmd.Stdin = stdinNull
	} else {
		s.logger.Warn("spawn: failed to open os.DevNull for stdin", "err", devErr)
	}
	spawnLogPath := filepath.Join(s.cfg.Hub.LogDir, "spawn",
		fmt.Sprintf("%s-%s.log", body.Provider, time.Now().Format("20060102-150405.000")))
	if err := os.MkdirAll(filepath.Dir(spawnLogPath), sessionlog.PrivateDirMode); err == nil {
		if f, logErr := os.OpenFile(spawnLogPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, sessionlog.PrivateFileMode); logErr == nil {
			spawnLog = f
			cmd.Stdout = spawnLog
			cmd.Stderr = spawnLog
		} else {
			s.logger.Warn("spawn: failed to create spawn log file", "path", spawnLogPath, "err", logErr)
		}
	}
	setCmdSysProcAttr(cmd)
	if err := cmd.Start(); err != nil {
		cleanupIsolated()
		// 読む相手が起動しなかったので、書いたプロンプトを引き取る。
		removeHeadlessPromptFile(promptPath)
		if stdinNull != nil {
			_ = stdinNull.Close()
		}
		if spawnLog != nil {
			_ = spawnLog.Close()
		}
		writeJSONError(w, http.StatusInternalServerError, "spawn_error", errorDetail("spawn error", err))
		return
	}
	spawnStarted = true
	s.logger.Debug("spawn: wrap process started",
		"provider", body.Provider, "pid", cmd.Process.Pid, "spawn_log", spawnLogPath)
	// ローカル LLM route のモデルは last_model に保存しない。
	// 残すと model 空欄の次回 spawn で fallback として再選択され、
	// Claude/Codex の純正起動のつもりがローカル LLM 経由になる罠を踏むため。
	// 純正 (anthropic/openai) のモデル選択は引き続き sticky に保存する。
	if resolvedModel != "" && !isLocalRoute(effectiveRoute) {
		if err := s.setLastModel(body.Provider, resolvedModel); err != nil {
			s.logger.Warn("failed to save last model", "provider", body.Provider, "error", err)
		}
	}
	s.safeGo("spawn_wait", func() {
		waitErr := cmd.Wait()
		exitCode := 0
		if cmd.ProcessState != nil {
			exitCode = cmd.ProcessState.ExitCode()
		}
		s.logger.Debug("spawn: wrap process exited",
			"provider", body.Provider, "exit_code", exitCode, "wait_err", fmt.Sprintf("%v", waitErr))
		if stdinNull != nil {
			_ = stdinNull.Close()
		}
		if spawnLog != nil {
			_ = spawnLog.Close()
		}
	})
	if orchestrationID != "" {
		writeJSON(w, map[string]any{"ok": true, "orchestration_id": orchestrationID})
		return
	}
	writeJSON(w, map[string]bool{"ok": true})
}

// handleSpawnGrid は複数 session を一括起動して Detached Grid 用の session_ids を返す。
// request: { preset, layout, count, cwd, label_prefix }
// response: { ok, layout, session_ids }
//
// 対応 preset:
//   - "shell"     : count 枚の Shell session を起動
//   - "ai+shell"  : AI session 1 枚 + Shell session (count-1) 枚を起動
//     provider フィールドで AI provider を指定（省略時 "claude"）
func (s *Server) handleSpawnGrid(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Preset      string `json:"preset"`
		Layout      string `json:"layout"`
		Count       int    `json:"count"`
		CWD         string `json:"cwd"`
		LabelPrefix string `json:"label_prefix"`
		Provider    string `json:"provider"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}

	// preset バリデーション
	validPresets := map[string]bool{
		"shell": true, "ai+shell": true,
	}
	if !validPresets[body.Preset] {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid preset")
		return
	}

	// layout バリデーション（1x1〜6x3 の範囲）
	validLayouts := map[string]bool{
		"": true, "1x1": true, "1x2": true, "2x2": true,
		"2x3": true, "3x3": true, "4x3": true, "6x3": true,
	}
	if !validLayouts[body.Layout] {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid layout")
		return
	}

	// count バリデーション（1〜18）
	if body.Count < 1 || body.Count > 18 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "count must be 1-18")
		return
	}

	// cwd 解決 + 検証
	cwd := body.CWD
	if cwd == "" {
		cwd = s.hubCWD
	} else {
		info, statErr := os.Stat(cwd)
		if statErr != nil || !info.IsDir() {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "cwd does not exist or is not a directory")
			return
		}
	}

	// layout 自動算出（省略時）
	layout := body.Layout
	if layout == "" {
		layout = calcGridLayout(body.Count)
	}

	// label_prefix の先頭 "-" はフラグ偽装を防ぐために禁止する。
	if strings.HasPrefix(body.LabelPrefix, "-") {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid label_prefix")
		return
	}
	if strings.HasPrefix(body.Provider, "-") {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid provider")
		return
	}
	// シェルメタ文字・制御文字を弾く（grid 経由の引数注入の多層防御）。
	// label_prefix のみ検証する（provider/model は固定 enum / validAIProviders 側で網羅）。
	if !spawnValidModelLabel(body.LabelPrefix) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid label_prefix")
		return
	}
	// 広域 cwd を弾く（spawn と同じ多層防御）。
	if spawnCwdTooBroad(cwd) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "cwd is too broad (system root or home root)")
		return
	}

	// AI provider バリデーション（ai+shell プリセット時のみ使用）。
	aiProvider := body.Provider
	if aiProvider == "" {
		aiProvider = "claude"
	}
	if body.Preset == "ai+shell" && !s.validGridAIProvider(aiProvider) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid ai provider for ai+shell preset")
		return
	}

	exe, err := os.Executable()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "executable_error", errorDetail("executable error", err))
		return
	}
	hubPort := s.currentHubPort()

	// 起動するセッションの (provider, label) リストを構築する
	type sessionSpec struct {
		provider string
		label    string
	}
	var specs []sessionSpec
	labelPrefix := body.LabelPrefix
	if labelPrefix == "" {
		labelPrefix = "grid"
	}

	switch body.Preset {
	case "shell":
		for i := 0; i < body.Count; i++ {
			specs = append(specs, sessionSpec{
				provider: "shell",
				label:    fmt.Sprintf("%s-%d", labelPrefix, i+1),
			})
		}
	case "ai+shell":
		// AI 1 枚 + Shell (count-1) 枚
		aiCount := 1
		shellCount := body.Count - aiCount
		if shellCount < 0 {
			shellCount = 0
		}
		specs = append(specs, sessionSpec{
			provider: aiProvider,
			label:    fmt.Sprintf("%s-%s-1", labelPrefix, aiProvider),
		})
		for i := 0; i < shellCount; i++ {
			specs = append(specs, sessionSpec{
				provider: "shell",
				label:    fmt.Sprintf("%s-shell-%d", labelPrefix, i+1),
			})
		}
	}

	// セッションを順次 spawn する
	for _, spec := range specs {
		wrapArgs := []string{"wrap", spec.provider, "--label=" + spec.label}
		cmd := exec.Command(exe, wrapArgs...)
		cmd.Dir = cwd
		cmd.Env = hubSpawnEnv(os.Environ(), hubPort, exe)
		if s.parentShell != "" {
			cmd.Env = append(cmd.Env, "MANY_AI_CLI_PARENT_SHELL="+s.parentShell)
		}
		if envPreset := EnvPresetFor(spec.provider, ""); len(envPreset) > 0 {
			cmd.Env = mergeEnvOverrides(cmd.Env, envPreset)
		}
		// stdin を DevNull に、stdout/stderr をログファイルに向ける（handleSpawn と同様）
		var stdinNull, spawnLog *os.File
		if f, devErr := os.OpenFile(os.DevNull, os.O_RDWR, 0); devErr == nil {
			stdinNull = f
			cmd.Stdin = stdinNull
		}
		spawnLogPath := filepath.Join(s.cfg.Hub.LogDir, "spawn",
			fmt.Sprintf("%s-%s.log", spec.provider, time.Now().Format("20060102-150405.000")))
		if mkErr := os.MkdirAll(filepath.Dir(spawnLogPath), sessionlog.PrivateDirMode); mkErr == nil {
			if f, logErr := os.OpenFile(spawnLogPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, sessionlog.PrivateFileMode); logErr == nil {
				spawnLog = f
				cmd.Stdout = spawnLog
				cmd.Stderr = spawnLog
			}
		}
		setCmdSysProcAttr(cmd)
		if startErr := cmd.Start(); startErr != nil {
			if stdinNull != nil {
				_ = stdinNull.Close()
			}
			if spawnLog != nil {
				_ = spawnLog.Close()
			}
			writeJSONError(w, http.StatusInternalServerError, "spawn_error", errorDetail("spawn error", startErr))
			return
		}
		s.logger.Debug("spawn-grid: wrap process started",
			"provider", spec.provider, "label", spec.label, "pid", cmd.Process.Pid)
		s.safeGo("spawn_grid_wait", func() {
			_ = cmd.Wait()
			if stdinNull != nil {
				_ = stdinNull.Close()
			}
			if spawnLog != nil {
				_ = spawnLog.Close()
			}
		})
	}

	writeJSON(w, map[string]any{
		"ok":     true,
		"layout": layout,
		"count":  len(specs),
	})
}

// calcGridLayout は session 数から適切な grid レイアウト文字列を返す。
// session-list.ts の calcDetachedLayout と対称的な実装。
func calcGridLayout(count int) string {
	switch {
	case count <= 1:
		return "1x1"
	case count <= 2:
		return "1x2"
	case count <= 4:
		return "2x2"
	case count <= 6:
		return "2x3"
	case count <= 9:
		return "3x3"
	case count <= 12:
		return "4x3"
	default:
		return "6x3"
	}
}

func (s *Server) getLastModel(provider string) string {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	if s.cfg.UserPrefs.Spawn.LastModel == nil {
		s.cfg.UserPrefs.Spawn.LastModel = map[string]string{}
	}
	return strings.TrimSpace(s.cfg.UserPrefs.Spawn.LastModel[provider])
}

func (s *Server) setLastModel(provider, model string) error {
	s.cfgMu.Lock()
	if s.cfg.UserPrefs.Spawn.LastModel == nil {
		s.cfg.UserPrefs.Spawn.LastModel = map[string]string{}
	}
	s.cfg.UserPrefs.Spawn.LastModel[provider] = model
	s.cfgMu.Unlock()
	return s.persistConfig()
}

// spawnNeedsRiskConfirmation は「ユーザーの確認が取れるまで起動してはいけない
// spawn か」を返す。usage probe は Hub が内部で起こす使い捨てセッションで、
// 確認を出す相手（押した人）が居ないため対象外にする。ここを通すと
// RiskConfirmed が立たないまま必ず risk confirmation required で落ちる。
// サブスクリプションログインの spawn が provider 別分岐へ入らないのと同じ扱い。
func spawnNeedsRiskConfirmation(spec spawnWrappedSpec, confirmationRequired bool) bool {
	if spec.UsageProbe {
		return false
	}
	return confirmationRequired && !spec.RiskConfirmed
}

// spawnRecordsLastModel は spawn したモデルを「次回の既定モデル」として
// 保存してよいかを返す。usage probe のモデル（低コストの固定値）はユーザーの
// 選択ではないので保存しない。保存すると次にユーザーが自分のモデルで起動する
// ときに「モデル変更＝高リスク」の確認が出る。
func spawnRecordsLastModel(spec spawnWrappedSpec, resolvedModel, effectiveRoute string) bool {
	if spec.UsageProbe || strings.TrimSpace(resolvedModel) == "" {
		return false
	}
	return !isLocalRoute(effectiveRoute)
}

// spawnInitialPromptMaxLen bounds the initial_prompt a plain /api/spawn can
// carry. jsonBodyMaxBytes already caps the whole request; this caps the
// prompt itself, the same way gitCommitBodyMaxLen caps a commit body
// (git_commit.go) — a separate, smaller ceiling on the one free-text field
// that gets typed into a running CLI.
const spawnInitialPromptMaxLen = 8192

// sanitizeSpawnInitialPrompt trims, strips control bytes, and caps the length
// of a spawn request's initial_prompt. It reuses sanitizeInjectText — the
// same filter the spawn-child injection path (orchestration.go) runs before
// writing to a PTY — so both callers agree on what "safe to inject" means
// instead of each carrying its own filter. An empty or all-whitespace input
// returns "", which callers treat as "no prompt was requested" (plan_
// session-handoff-board_c4_prompted-spawn.md C1: empty must not change
// spawn behavior at all).
func sanitizeSpawnInitialPrompt(raw string) string {
	s := strings.TrimSpace(sanitizeInjectText(raw))
	if s == "" {
		return ""
	}
	return truncateUTF8Bytes(s, spawnInitialPromptMaxLen)
}

// spawnPendingLabelForInitialPrompt decides the label to key
// s.orchestration.pending by when a spawn request carries a non-empty
// initial prompt, generating one if the caller (and no earlier step, such as
// isolate_worktree or orchestration=true) already assigned a label. It
// returns "" when there is nothing to deliver, in which case the caller must
// leave label handling and the pending map untouched entirely — this is the
// single choke point that keeps an empty initial_prompt byte-for-byte
// identical to a request that never mentions the field.
func spawnPendingLabelForInitialPrompt(label, initialPrompt string, now time.Time) string {
	if initialPrompt == "" {
		return ""
	}
	if label != "" {
		return label
	}
	return fmt.Sprintf("spawn-%d", now.UnixNano())
}

// spawnValidModelLabel は model / label / label_prefix 値が安全かを検証する。
// 空文字は「未指定」として許可。シェルメタ文字・制御文字・空白を含む値は拒否する。
// finding #22: Windows の cmd.exe /c シム経路への引数注入の多層防御。
func spawnValidModelLabel(s string) bool {
	if s == "" {
		return true
	}
	// 禁止文字: ASCII 制御文字・空白・シェルメタ文字
	for _, r := range s {
		if r < 0x20 || r == 0x7f {
			return false
		}
		switch r {
		case ' ', '\t', '"', '\'', '|', '&', '>', '<', '^', '%', '(', ')', ';', '`', '$':
			return false
		}
	}
	return true
}

// spawnCwdTooBroad は cwd がルートや主要システムディレクトリ自身である場合 true を返す。
// 配下の通常プロジェクトフォルダは許可（finding #1 折衷案）。
func spawnCwdTooBroad(cwd string) bool {
	if cwd == "" {
		return false
	}
	cwd = filepath.Clean(cwd)
	vol := filepath.VolumeName(cwd)
	// ドライブ/FS ルート
	if cwd == vol+"/" || cwd == vol+"\\" || cwd == vol+string(filepath.Separator) {
		return true
	}
	// ホームディレクトリ自身
	if home, err := os.UserHomeDir(); err == nil {
		if filepath.Clean(home) == cwd {
			return true
		}
		// 全ユーザーの親（/home, /Users, C:\Users 等）
		if filepath.Clean(filepath.Dir(home)) == cwd {
			return true
		}
	}
	// 主要 Unix システムディレクトリ＋全ユーザー親（自身のみ）。
	// /home / /Users は os.UserHomeDir() ベースの判定では捕まらないケース
	// （CI macOS では HOME=/Users/... なので /home が漏れる）があるため、
	// プラットフォームに依らず明示的に列挙する。
	unixBroad := map[string]bool{
		"/etc": true, "/usr": true, "/var": true,
		"/bin": true, "/sbin": true, "/lib": true, "/root": true,
		"/home": true, "/Users": true,
	}
	if unixBroad[cwd] {
		return true
	}
	// Windows 主要システムディレクトリ。NTFS はケースインセンシティブなので
	// `c:\windows` のような小文字入力でも `os.Stat` は同じ実体を返す一方、
	// map lookup は case-sensitive でガードをすり抜ける。approval_handler.go の
	// approvalTargetKey と同様、Windows パス判定で正規化してから照合する。
	winBroad := map[string]bool{
		`C:\Windows`:             true,
		`C:\Program Files`:       true,
		`C:\Program Files (x86)`: true,
		`C:\Users`:               true,
	}
	if runtime.GOOS == "windows" || isWindowsPath(cwd) {
		lower := strings.ToLower(cwd)
		for k := range winBroad {
			if strings.ToLower(k) == lower {
				return true
			}
		}
		return false
	}
	return winBroad[cwd]
}
