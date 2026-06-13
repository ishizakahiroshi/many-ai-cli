package hub

import (
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/sessionlog"
)

func (s *Server) handleSpawn(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Provider       string `json:"provider"`
		CWD            string `json:"cwd"`
		Model          string `json:"model"`
		ModelSelection string `json:"model_selection_mode"`
		RiskConfirmed  bool   `json:"risk_confirmed"`
		Label          string `json:"label"`
		PermissionMode string `json:"permission_mode"`
		Sandbox        string `json:"sandbox"`
		AskForApproval string `json:"ask_for_approval"`
		Route          string `json:"route"`
		Utf8Session    bool   `json:"utf8_session"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.Provider != "claude" && body.Provider != "codex" && body.Provider != "copilot" && body.Provider != "cursor-agent" && body.Provider != "shell" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid provider")
		return
	}
	validPermModes := map[string]bool{
		"": true, "default": true, "plan": true,
		"acceptEdits": true, "auto": true, "bypassPermissions": true,
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

	exe, err := os.Executable()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "executable_error", errorDetail("executable error", err))
		return
	}
	wrapArgs := []string{"wrap", body.Provider}
	resolvedModel := strings.TrimSpace(body.Model)

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
		cmd.Env = append(sanitizeEnv(os.Environ()), "MANY_AI_CLI=1",
			fmt.Sprintf("MANY_AI_CLI_HUB_PORT=%d", hubPort))
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
			if stdinNull != nil {
				_ = stdinNull.Close()
			}
			if spawnLog != nil {
				_ = spawnLog.Close()
			}
			writeJSONError(w, http.StatusInternalServerError, "spawn_error", errorDetail("spawn error", err))
			return
		}
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

	switch body.Provider {
	case "claude":
		mode := body.ModelSelection
		if mode == "" {
			mode = "auto"
		}
		currentModel := s.getLastModel("claude")
		// Ollama route のモデルは fallback として復活させない。
		// 残すと model 空欄の spawn で前回の Ollama モデルが --model に注入され、
		// route=ollama と判定されて ANTHROPIC_BASE_URL=localhost:11434 が焼き付く。
		// その結果 Claude 単独起動のつもりが Ollama 経由になる罠を踏むため。
		if currentModel != "" && s.resolveRoute("claude", currentModel) == RouteOllama {
			currentModel = ""
		}
		if resolvedModel == "" {
			resolvedModel = currentModel
		}
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
		currentModel := s.getLastModel("codex")
		// Ollama route のモデルは fallback として復活させない（claude 側と同じ理由）。
		if currentModel != "" && s.resolveRoute("codex", currentModel) == RouteOllama {
			currentModel = ""
		}
		if resolvedModel == "" {
			resolvedModel = currentModel
		}
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
	}
	// route が未指定の場合は model 名から推定する。Anthropic / OpenAI の
	// 既定 route は env 注入を行わない（ユーザー shell の値を継承）。
	effectiveRoute := body.Route
	if effectiveRoute == "" {
		s.cfgMu.Lock()
		localCfg := append([]config.LocalModel(nil), s.cfg.LocalModels...)
		s.cfgMu.Unlock()
		known := collectOllamaModelIDs(s.modelsCache, localCfg)
		effectiveRoute = RouteForModel(body.Provider, resolvedModel, known)
	}
	// Codex CLI は env (OPENAI_BASE_URL 等) だけでは provider を切り替えず、
	// CLI 引数 --oss / --profile で OSS (Ollama) provider に切替える設計。
	// route=ollama のときに --oss を渡さないと OpenAI 純正へ向かい認証エラーで落ちる。
	if body.Provider == "codex" && effectiveRoute == RouteOllama {
		wrapArgs = append(wrapArgs, "--codex-oss")
	}
	if body.Utf8Session {
		wrapArgs = append(wrapArgs, "--utf8")
	}
	hubPort := s.currentHubPort()
	cmd := exec.Command(exe, wrapArgs...)
	cmd.Dir = cwd
	cmd.Env = append(sanitizeEnv(os.Environ()), "MANY_AI_CLI=1",
		fmt.Sprintf("MANY_AI_CLI_HUB_PORT=%d", hubPort))
	if s.parentShell != "" {
		cmd.Env = append(cmd.Env, "MANY_AI_CLI_PARENT_SHELL="+s.parentShell)
	}
	if envPreset := EnvPresetFor(body.Provider, effectiveRoute); len(envPreset) > 0 {
		cmd.Env = mergeEnvOverrides(cmd.Env, envPreset)
		s.logger.Debug("spawn: env preset applied",
			"provider", body.Provider, "route", effectiveRoute, "keys", envKeyList(envPreset))
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
		if stdinNull != nil {
			_ = stdinNull.Close()
		}
		if spawnLog != nil {
			_ = spawnLog.Close()
		}
		writeJSONError(w, http.StatusInternalServerError, "spawn_error", errorDetail("spawn error", err))
		return
	}
	s.logger.Debug("spawn: wrap process started",
		"provider", body.Provider, "pid", cmd.Process.Pid, "spawn_log", spawnLogPath)
	// Ollama route のモデルは last_model に保存しない。
	// 残すと model 空欄の次回 spawn で fallback として再選択され、
	// Claude/Codex の純正起動のつもりが Ollama 経由になる罠を踏むため。
	// 純正 (anthropic/openai) のモデル選択は引き続き sticky に保存する。
	if resolvedModel != "" && effectiveRoute != RouteOllama {
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
	writeJSON(w, map[string]bool{"ok": true})
}

// handleSpawnGrid は複数 session を一括起動して Detached Grid 用の session_ids を返す。
// request: { preset, layout, count, cwd, label_prefix }
// response: { ok, layout, session_ids }
//
// 対応 preset:
//   - "shell"     : count 枚の Shell session を起動
//   - "ai+shell"  : AI session 1 枚 + Shell session (count-1) 枚を起動
//                   provider フィールドで AI provider を指定（省略時 "claude"）
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

	// AI provider バリデーション（ai+shell プリセット時のみ使用）
	aiProvider := body.Provider
	if aiProvider == "" {
		aiProvider = "claude"
	}
	validAIProviders := map[string]bool{
		"claude": true, "codex": true, "copilot": true, "cursor-agent": true,
	}
	if body.Preset == "ai+shell" && !validAIProviders[aiProvider] {
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
		cmd.Env = append(sanitizeEnv(os.Environ()), "MANY_AI_CLI=1",
			fmt.Sprintf("MANY_AI_CLI_HUB_PORT=%d", hubPort))
		if s.parentShell != "" {
			cmd.Env = append(cmd.Env, "MANY_AI_CLI_PARENT_SHELL="+s.parentShell)
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
