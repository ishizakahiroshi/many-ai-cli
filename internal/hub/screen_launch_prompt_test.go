package hub

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

// 画面から起動するセッション（/api/spawn）の最初の指示の渡し方を固定する（子 plan:
// docs/local/plan_child-launch-prompt-and-trust_c5_ui-launched.md 内部 C1）。
//
// 起動は wrapper のプロセスを作る直前（startSpawnCmd）で止める。そこで Hub が組んだ
// 引数と --prompt-file の中身を読み取り、起動は失敗させる。テストから wrapper も
// CLI も立たない。登録後に打ち込むかどうかは wrapperLoop が registrationInjectPrompt
// と initialInjectGateNeeded だけで決めるので、pending に残った印をその 2 つへ通して
// 確かめる。

// screenSpawnCapture は止めた起動 1 回ぶんの記録。
type screenSpawnCapture struct {
	args        []string
	promptFiles int
	promptPath  string
	prompt      string
}

func screenSpawnServer(t *testing.T, usable bool) (*Server, *screenSpawnCapture) {
	t.Helper()
	s, _ := subsTestServer(t) // HOME / USERPROFILE を一時フォルダにする（--prompt-file の置き場所）
	s.hubCWD = t.TempDir()
	s.cfg.Hub.LogDir = t.TempDir() // spawn ログの置き場所。空のままだとパッケージのフォルダへ書く
	s.launchArgUsable = func(string) (bool, string) {
		if usable {
			return true, ""
		}
		return false, "synthetic: the launch goes through cmd.exe from a shim path with whitespace"
	}
	got := &screenSpawnCapture{}
	s.startSpawnCmd = func(cmd *exec.Cmd) error {
		got.args = append([]string(nil), cmd.Args...)
		for i, arg := range cmd.Args {
			if arg != "--prompt-file" || i+1 >= len(cmd.Args) {
				continue
			}
			got.promptFiles++
			got.promptPath = cmd.Args[i+1]
			data, err := os.ReadFile(got.promptPath)
			if err != nil {
				t.Errorf("read the prompt file the Hub pointed the wrapper at: %v", err)
			}
			got.prompt = string(data)
		}
		return errors.New("synthetic: the wrapper is not started in tests")
	}
	return s, got
}

func postScreenSpawn(t *testing.T, s *Server, body map[string]any) {
	t.Helper()
	req := map[string]any{
		"cwd":              s.hubCWD,
		"isolate_worktree": false,
		"risk_confirmed":   true,
	}
	for k, v := range body {
		req[k] = v
	}
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", req))
	if w.Code != http.StatusInternalServerError || !strings.Contains(w.Body.String(), "synthetic: the wrapper is not started") {
		t.Fatalf("code = %d, body = %s, want the stubbed start failure (the launch must reach the wrapper start)", w.Code, w.Body.String())
	}
}

func screenSpawnPending(t *testing.T, s *Server, label string) pendingChild {
	t.Helper()
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	meta, ok := s.orchestration.pending[label]
	if !ok {
		t.Fatalf("no pending entry for label %q", label)
	}
	return meta
}

// 起動時に渡したセッションは、登録後に 1 バイトも打ち込まれず、入力保留も掛からない。
func assertNothingTypedAfterRegistration(t *testing.T, s *Server, meta pendingChild) {
	t.Helper()
	if !meta.PromptAtLaunch {
		t.Error("PromptAtLaunch = false, want true")
	}
	if meta.InitialPrompt != "" {
		t.Errorf("InitialPrompt = %q, want empty (the instruction would arrive twice)", meta.InitialPrompt)
	}
	if prompt, _ := s.registrationInjectPrompt(meta); prompt != "" {
		t.Errorf("registration would type %q, want nothing", prompt)
	}
	if initialInjectGateNeeded(meta) {
		t.Error("registration would hold the user's input, want no gate")
	}
}

// 起動に失敗したら、書いた --prompt-file は Hub が引き取る。
func assertPromptFileTakenBack(t *testing.T, path string) {
	t.Helper()
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Errorf("prompt file %s is still there after the failed start (stat err = %v)", path, err)
	}
}

// 完了条件 1: claude の conductor は、案内を --prompt-file で起動時に受け取り、
// ファイルの中身は buildConductorInitialPrompt の結果と同じ。登録後は打ち込まれない。
func TestScreenSpawnHandsAClaudeConductorItsGuideAtLaunch(t *testing.T) {
	s, got := screenSpawnServer(t, true)
	postScreenSpawn(t, s, map[string]any{
		"provider":      "claude",
		"label":         "conductor-under-test",
		"orchestration": true,
		// 役割は 1 つだけにする。案内は map を回して書くので、2 つ以上だと並び順が
		// 呼ぶたびに変わり、中身の一致を比べられない。
		"orchestration_roles": map[string]any{"review": map[string]any{"provider": "codex", "model": "sample-model"}},
	})
	meta := screenSpawnPending(t, s, "conductor-under-test")
	want := buildConductorInitialPrompt(meta.OrchestrationID, s.orchestrationRolesFor(meta.OrchestrationID))
	if !strings.Contains(want, "review: provider=codex model=sample-model") {
		t.Fatalf("fixture: the guide does not carry the configured role:\n%s", want)
	}
	if got.promptFiles != 1 {
		t.Fatalf("--prompt-file appears %d times in %v, want exactly once", got.promptFiles, got.args)
	}
	if got.prompt != want {
		t.Errorf("prompt file =\n%s\nwant the conductor guide =\n%s", got.prompt, want)
	}
	assertNothingTypedAfterRegistration(t, s, meta)
	assertPromptFileTakenBack(t, got.promptPath)
}

// 完了条件 2: codex の指示付き起動は、sanitize 済みの文面を --prompt-file で受け取り、
// 登録後は打ち込まれない。
func TestScreenSpawnHandsACodexInstructionAtLaunch(t *testing.T) {
	s, got := screenSpawnServer(t, true)
	postScreenSpawn(t, s, map[string]any{
		"provider":       "codex",
		"label":          "prompted-under-test",
		"initial_prompt": "  review the diff\x07  ",
	})
	if got.promptFiles != 1 || got.prompt != "review the diff" {
		t.Fatalf("prompt files = %d, prompt = %q (args %v), want the sanitized instruction once", got.promptFiles, got.prompt, got.args)
	}
	assertNothingTypedAfterRegistration(t, s, screenSpawnPending(t, s, "prompted-under-test"))
	assertPromptFileTakenBack(t, got.promptPath)
}

// 完了条件 3: 表に無い provider（copilot）と、表にあってもこの端末で渡せない
// claude は、conductor も指示付き起動も今までどおり登録後に打ち込む。起動の引数は
// 1 つも増えない。
func TestScreenSpawnStillTypesWhenTheCLICannotTakeItAtLaunch(t *testing.T) {
	for _, tc := range []struct {
		name     string
		provider string
		usable   bool
	}{
		{"copilot", "copilot", true},
		{"claude on a machine that cannot pass it", "claude", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s, got := screenSpawnServer(t, tc.usable)
			postScreenSpawn(t, s, map[string]any{"provider": tc.provider, "label": "conductor-under-test", "orchestration": true})
			if got.promptFiles != 0 {
				t.Fatalf("conductor: args = %v, want no --prompt-file", got.args)
			}
			meta := screenSpawnPending(t, s, "conductor-under-test")
			guide := buildConductorInitialPrompt(meta.OrchestrationID, nil)
			if prompt, name := s.registrationInjectPrompt(meta); prompt != guide || name != "inject_initial_prompt_conductor" {
				t.Errorf("conductor: registration types %q (%s), want the guide", prompt, name)
			}
			if meta.PromptAtLaunch || !initialInjectGateNeeded(meta) {
				t.Errorf("conductor: PromptAtLaunch = %v gate = %v, want false / true", meta.PromptAtLaunch, initialInjectGateNeeded(meta))
			}

			s, got = screenSpawnServer(t, tc.usable)
			postScreenSpawn(t, s, map[string]any{"provider": tc.provider, "label": "prompted-under-test", "initial_prompt": "review the diff"})
			if got.promptFiles != 0 {
				t.Fatalf("prompted: args = %v, want no --prompt-file", got.args)
			}
			meta = screenSpawnPending(t, s, "prompted-under-test")
			if prompt, name := s.registrationInjectPrompt(meta); prompt != "review the diff" || name != "inject_initial_prompt_spawn" {
				t.Errorf("prompted: registration types %q (%s), want the instruction", prompt, name)
			}
			if meta.PromptAtLaunch || !initialInjectGateNeeded(meta) {
				t.Errorf("prompted: PromptAtLaunch = %v gate = %v, want false / true", meta.PromptAtLaunch, initialInjectGateNeeded(meta))
			}
		})
	}
}

// 完了条件 4: conductor に initial_prompt も付いているとき、届くのは conductor の
// 案内だけ（今の挙動と同じ）。起動時に渡す claude でも、打ち込む copilot でも同じ。
func TestScreenSpawnConductorWithAnInstructionGetsOnlyTheGuide(t *testing.T) {
	const instruction = "an instruction that must not reach a conductor"

	s, got := screenSpawnServer(t, true)
	postScreenSpawn(t, s, map[string]any{"provider": "claude", "label": "conductor-under-test", "orchestration": true, "initial_prompt": instruction})
	meta := screenSpawnPending(t, s, "conductor-under-test")
	if want := buildConductorInitialPrompt(meta.OrchestrationID, nil); got.prompt != want {
		t.Errorf("claude: prompt file =\n%s\nwant the conductor guide only", got.prompt)
	}
	assertNothingTypedAfterRegistration(t, s, meta)

	s, _ = screenSpawnServer(t, true)
	postScreenSpawn(t, s, map[string]any{"provider": "copilot", "label": "conductor-under-test", "orchestration": true, "initial_prompt": instruction})
	meta = screenSpawnPending(t, s, "conductor-under-test")
	if prompt, _ := s.registrationInjectPrompt(meta); prompt != buildConductorInitialPrompt(meta.OrchestrationID, nil) {
		t.Errorf("copilot: registration types %q, want the conductor guide only", prompt)
	}
}

// headless の指示付き起動は今までどおり: 画面の文面を --prompt-file で渡し、
// 起動引数の経路の印は立てない（打ち込まないのは headless だから）。
func TestScreenSpawnHeadlessLaunchIsUnchanged(t *testing.T) {
	s, got := screenSpawnServer(t, true)
	postScreenSpawn(t, s, map[string]any{
		"provider":       "claude",
		"label":          "headless-under-test",
		"execution_mode": config.ExecutionModeHeadless,
		"initial_prompt": "review the diff",
	})
	if got.promptFiles != 1 || got.prompt != "review the diff" || !containsArg(got.args, "--headless") {
		t.Fatalf("args = %v, prompt = %q, want --headless with the instruction as the prompt file", got.args, got.prompt)
	}
	meta := screenSpawnPending(t, s, "headless-under-test")
	if meta.PromptAtLaunch || meta.InitialPrompt != "" {
		t.Errorf("headless: PromptAtLaunch = %v InitialPrompt = %q, want false / empty", meta.PromptAtLaunch, meta.InitialPrompt)
	}
}

func containsArg(args []string, want string) bool {
	for _, arg := range args {
		if arg == want {
			return true
		}
	}
	return false
}

// 登録時に打ち込むものは、どのセッションでも registrationInjectPrompt の表どおり。
func TestRegistrationInjectPromptTypesOnlyWhatWasNotHandedAtLaunch(t *testing.T) {
	s := newTestServer()
	guide := buildConductorInitialPrompt("o1", nil)
	for _, tc := range []struct {
		name       string
		meta       pendingChild
		wantPrompt string
		wantName   string
	}{
		{"conductor", pendingChild{OrchestrationID: "o1"}, guide, "inject_initial_prompt_conductor"},
		{"conductor with an instruction", pendingChild{OrchestrationID: "o1", InitialPrompt: "hi"}, guide, "inject_initial_prompt_conductor"},
		{"conductor handed at launch", pendingChild{OrchestrationID: "o1", PromptAtLaunch: true}, "", ""},
		{"prompted /api/spawn", pendingChild{InitialPrompt: "hi"}, "hi", "inject_initial_prompt_spawn"},
		{"prompted /api/spawn handed at launch", pendingChild{PromptAtLaunch: true}, "", ""},
		{"spawn-child child", pendingChild{OrchestrationID: "o1", Auto: true}, "", ""},
		{"plain session", pendingChild{}, "", ""},
	} {
		prompt, name := s.registrationInjectPrompt(tc.meta)
		if prompt != tc.wantPrompt || name != tc.wantName {
			t.Errorf("%s: registration types %q (%s), want %q (%s)", tc.name, prompt, name, tc.wantPrompt, tc.wantName)
		}
	}
}
