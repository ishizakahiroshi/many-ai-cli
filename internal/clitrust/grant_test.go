package clitrust

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"many-ai-cli/internal/subscription"
)

// These tests drive the public Grant with a child env that points
// CODEX_HOME / CLAUDE_CONFIG_DIR at a temp folder, so the user's real
// ~/.codex/config.toml and ~/.claude.json are never read or written. Every
// fixture is synthetic. Folders come from realTempDir so that the spellings
// git records (worktree metadata) and the ones the test builds agree on every
// runner (8.3 short names on Windows, /var → /private/var on macOS).

func realTempDir(t *testing.T) string {
	t.Helper()
	dir, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	return dir
}

func runGit(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", append([]string{"-C", dir}, args...)...)
	cmd.Env = append(os.Environ(), "GIT_CONFIG_GLOBAL="+os.DevNull, "GIT_CONFIG_NOSYSTEM=1")
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
}

// newGitRepo makes a repository with one empty commit, so worktrees can be
// added from it.
func newGitRepo(t *testing.T) string {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git is not installed")
	}
	repo := filepath.Join(realTempDir(t), "repo")
	if err := os.MkdirAll(repo, 0o700); err != nil {
		t.Fatal(err)
	}
	runGit(t, repo, "init", "-q")
	runGit(t, repo, "-c", "user.name=clitrust-test", "-c", "user.email=clitrust-test@example.invalid",
		"commit", "-q", "--allow-empty", "-m", "init")
	return repo
}

// addWorktree adds a linked worktree the way the Hub does for an
// orchestration child (<repo>/.many-ai-cli/worktrees/<orchestration>/<role>).
func addWorktree(t *testing.T, repo, role string) string {
	t.Helper()
	path := filepath.Join(repo, ".many-ai-cli", "worktrees", "orch-1", role)
	runGit(t, repo, "worktree", "add", "-q", "-b", "orch/orch-1/"+role, path)
	return path
}

func codexTargetIn(t *testing.T, dir string) (Target, string) {
	t.Helper()
	home := t.TempDir()
	return Target{Provider: "codex", Env: []string{subscription.CodexHomeEnv + "=" + home}, Dir: dir},
		filepath.Join(home, "config.toml")
}

func claudeTargetIn(t *testing.T, dir string) (Target, string) {
	t.Helper()
	configDir := t.TempDir()
	return Target{Provider: "claude", Env: []string{subscription.ClaudeConfigDirEnv + "=" + configDir}, Dir: dir},
		filepath.Join(configDir, ".claude.json")
}

// wantCodexKey spells path the way codex writes it: on Windows lowercased
// (ASCII only), elsewhere unchanged. path must already be real (no links, no
// 8.3 names).
func wantCodexKey(path string) string {
	if runtime.GOOS != "windows" {
		return path
	}
	b := []byte(path)
	for i, c := range b {
		if c >= 'A' && c <= 'Z' {
			b[i] = c + ('a' - 'A')
		}
	}
	return string(b)
}

// wantClaudeKey spells path the way Claude writes it: forward slashes on
// Windows, nothing else changed.
func wantClaudeKey(path string) string {
	if runtime.GOOS == "windows" {
		return strings.ReplaceAll(path, `\`, "/")
	}
	return path
}

func writeFileT(t *testing.T, path, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

func readFileT(t *testing.T, path string) []byte {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func codexUntrustedEntry(key string) string {
	return "[projects." + tomlQuoteKey(key) + "]\ntrust_level = \"untrusted\"\n"
}

func assertCodexFileUnchanged(t *testing.T, configPath, before string, result Result, err error) {
	t.Helper()
	if err != nil {
		t.Fatalf("Grant: %v", err)
	}
	if result.Written {
		t.Fatalf("Grant wrote %q; an untrusted entry at or above the folder must stop it", result.Key)
	}
	if result.Existing != "untrusted" {
		t.Errorf("Existing = %q, want untrusted", result.Existing)
	}
	if got := string(readFileT(t, configPath)); got != before {
		t.Errorf("config.toml changed:\nbefore=%q\nafter =%q", before, got)
	}
}

// C1 (a): a folder the user refused in another letter case is not trusted
// through the approval screen (v0.9 review C4-A F4; Q4a).
func TestGrantCodexLeavesAFolderUntrustedInAnotherCase(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("letter case only folds on Windows")
	}
	dir := realTempDir(t)
	target, configPath := codexTargetIn(t, dir)
	before := codexUntrustedEntry(strings.ToUpper(dir))
	writeFileT(t, configPath, before)

	result, err := Grant(target)
	assertCodexFileUnchanged(t, configPath, before, result, err)
}

// C1 (a): an untrusted parent folder also stops the write, even though codex
// itself only looks at the folder and its repository root.
func TestGrantCodexLeavesAFolderUnderAnUntrustedParent(t *testing.T) {
	parent := realTempDir(t)
	dir := filepath.Join(parent, "child")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	target, configPath := codexTargetIn(t, dir)
	before := codexUntrustedEntry(wantCodexKey(parent) + string(filepath.Separator))
	writeFileT(t, configPath, before)

	result, err := Grant(target)
	assertCodexFileUnchanged(t, configPath, before, result, err)
}

// C1 (a): a worktree child whose main repository is untrusted is not trusted
// through its own worktree key.
func TestGrantCodexLeavesAWorktreeOfAnUntrustedRepository(t *testing.T) {
	repo := newGitRepo(t)
	worktree := addWorktree(t, repo, "impl")
	target, configPath := codexTargetIn(t, worktree)
	before := codexUntrustedEntry(wantCodexKey(repo))
	writeFileT(t, configPath, before)

	result, err := Grant(target)
	assertCodexFileUnchanged(t, configPath, before, result, err)
}

// C1 (b)(d): codex writes its own trust target — the main repository for a
// worktree child — so the second worktree of the same repository adds
// nothing (v0.9 review C4-A F5: one table per worktree piled up before).
func TestGrantCodexWritesTheMainRepositoryOnceForWorktreeChildren(t *testing.T) {
	repo := newGitRepo(t)
	first := addWorktree(t, repo, "impl")
	second := addWorktree(t, repo, "review")
	target, configPath := codexTargetIn(t, first)

	result, err := Grant(target)
	if err != nil {
		t.Fatalf("Grant(first worktree): %v", err)
	}
	if !result.Written || result.Key != wantCodexKey(repo) {
		t.Fatalf("result = %+v, want Written with key %q (the main repository)", result, wantCodexKey(repo))
	}

	target.Dir = second
	result, err = Grant(target)
	if err != nil {
		t.Fatalf("Grant(second worktree): %v", err)
	}
	if result.Written || result.Existing != "trusted" {
		t.Errorf("second worktree: result = %+v, want the main repository's entry found trusted", result)
	}
	if n := bytes.Count(readFileT(t, configPath), []byte("[projects.")); n != 1 {
		t.Errorf("config.toml has %d project tables, want 1:\n%s", n, readFileT(t, configPath))
	}
}

// C1 (b): a folder inside a repository is recorded under the repository root,
// as codex's own "Trust and continue" does.
func TestGrantCodexUsesTheRepositoryRootForASubfolder(t *testing.T) {
	repo := newGitRepo(t)
	sub := filepath.Join(repo, "cases", "a")
	if err := os.MkdirAll(sub, 0o700); err != nil {
		t.Fatal(err)
	}
	target, _ := codexTargetIn(t, sub)

	result, err := Grant(target)
	if err != nil {
		t.Fatalf("Grant: %v", err)
	}
	if !result.Written || result.Key != wantCodexKey(repo) {
		t.Errorf("result = %+v, want Written with key %q", result, wantCodexKey(repo))
	}
}

// C1 (b): the key is codex's canonical spelling — backslashes, no trailing
// separator, ".." resolved, a junction resolved — lowercased for ASCII
// letters only.
func TestGrantCodexKeyIsTheCanonicalSpelling(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("the separators, the junction and the lowercasing are Windows-only")
	}
	base := realTempDir(t)
	real := filepath.Join(base, "Ärger-Dir", "Sub")
	if err := os.MkdirAll(real, 0o700); err != nil {
		t.Fatal(err)
	}
	junction := filepath.Join(base, "Link")
	if out, err := exec.Command("cmd", "/c", "mklink", "/J", junction, filepath.Dir(real)).CombinedOutput(); err != nil {
		t.Skipf("cannot create a junction here: %v %s", err, out)
	}
	want := wantCodexKey(real)
	for name, dir := range map[string]string{
		"forward slashes and trailing separator": strings.ReplaceAll(real, `\`, "/") + "/",
		"dot dot":                                filepath.Join(real, "..", "Sub") + `\..\Sub`,
		"junction":                               filepath.Join(junction, "Sub"),
	} {
		t.Run(name, func(t *testing.T) {
			target, _ := codexTargetIn(t, dir)
			result, err := Grant(target)
			if err != nil {
				t.Fatalf("Grant: %v", err)
			}
			if result.Key != want {
				t.Errorf("Key = %q, want %q", result.Key, want)
			}
		})
	}
}

// C1 (c): hasTrustDialogAccepted:false is the default of a new Claude entry,
// never an answer ("No, exit" writes nothing), so Grant turns it into true the
// way Claude's own "Yes" does, keeping every other field.
func TestGrantClaudeAcceptsAnEntryLeftAtTheDefaultFalse(t *testing.T) {
	repo := newGitRepo(t)
	target, configPath := claudeTargetIn(t, repo)
	key := wantClaudeKey(repo)
	entry := `{"allowedTools":["Bash(ls)"],"hasTrustDialogAccepted":false,"lastCost":1.25,"big":12345678901234567890,"nested":{"a":[1,{"b":null}]}}`
	body := `{"oauthAccount":{"id":"synthetic-account"},"projects":{` + mustJSON(t, key) + `:` + entry + `}}`
	writeFileT(t, configPath, body)

	result, err := Grant(target)
	if err != nil {
		t.Fatalf("Grant: %v", err)
	}
	if !result.Written {
		t.Fatalf("result = %+v, want Written (false is not an answer)", result)
	}
	after := decodeWithNumbers(t, readFileT(t, configPath))
	got := after["projects"].(map[string]any)[key].(map[string]any)
	want := decodeWithNumbers(t, []byte(entry))
	want["hasTrustDialogAccepted"] = true
	if !jsonEqual(t, got, want) {
		t.Errorf("entry = %#v, want %#v", got, want)
	}
	if !jsonEqual(t, after["oauthAccount"], decodeWithNumbers(t, []byte(body))["oauthAccount"]) {
		t.Error("oauthAccount changed")
	}
}

// C1 (c): an entry Grant does not understand (no hasTrustDialogAccepted) is
// left exactly as it is.
func TestGrantClaudeLeavesAnEntryWithoutTheFlag(t *testing.T) {
	repo := newGitRepo(t)
	target, configPath := claudeTargetIn(t, repo)
	body := `{"projects":{` + mustJSON(t, wantClaudeKey(repo)) + `:{"allowedTools":[]}}}`
	writeFileT(t, configPath, body)

	result, err := Grant(target)
	if err != nil {
		t.Fatalf("Grant: %v", err)
	}
	if result.Written || result.Existing != "other" {
		t.Errorf("result = %+v, want the entry left as other", result)
	}
	if got := string(readFileT(t, configPath)); got != body {
		t.Errorf("file changed:\n%s", got)
	}
}

// C1 (b)(d): Claude's own key for a worktree child is the main repository's
// root (its canonical git root), so every worktree child shares one entry.
func TestGrantClaudeWritesTheMainRepositoryForWorktreeChildren(t *testing.T) {
	repo := newGitRepo(t)
	first := addWorktree(t, repo, "impl")
	second := addWorktree(t, repo, "review")
	target, configPath := claudeTargetIn(t, first)

	result, err := Grant(target)
	if err != nil {
		t.Fatalf("Grant(first worktree): %v", err)
	}
	if !result.Written || result.Key != wantClaudeKey(repo) {
		t.Fatalf("result = %+v, want Written with key %q (the main repository)", result, wantClaudeKey(repo))
	}
	target.Dir = second
	result, err = Grant(target)
	if err != nil {
		t.Fatalf("Grant(second worktree): %v", err)
	}
	if result.Written || result.Existing != "trusted" {
		t.Errorf("second worktree: result = %+v, want the main repository found trusted", result)
	}
	var root struct {
		Projects map[string]json.RawMessage `json:"projects"`
	}
	if err := json.Unmarshal(readFileT(t, configPath), &root); err != nil {
		t.Fatal(err)
	}
	if len(root.Projects) != 1 {
		t.Errorf("projects has %d entries, want 1", len(root.Projects))
	}
}

// C1 (b): Claude keeps the drive letter's case (its key comes from the
// case-preserving realpath of the current folder), so a folder passed with a
// lowercase drive letter is recorded with one.
func TestGrantClaudeKeepsALowercaseDriveLetter(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("drive letters are Windows-only")
	}
	repo := newGitRepo(t)
	lower := strings.ToLower(repo[:1]) + repo[1:]
	target, _ := claudeTargetIn(t, lower)

	result, err := Grant(target)
	if err != nil {
		t.Fatalf("Grant: %v", err)
	}
	if want := wantClaudeKey(lower); result.Key != want {
		t.Errorf("Key = %q, want %q", result.Key, want)
	}
}

// C1 (b): a network (UNC) folder or one with a \\?\ prefix is not written —
// the codex shim cannot start there, and neither CLI's key for it is known.
func TestGrantRefusesANetworkOrVerbatimFolder(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("UNC and \\\\?\\ paths are Windows-only")
	}
	for _, provider := range []string{"codex", "claude"} {
		for _, dir := range []string{`\\server\share\work`, `\\?\` + realTempDir(t)} {
			t.Run(provider+" "+dir[:4], func(t *testing.T) {
				var target Target
				var configPath string
				if provider == "codex" {
					target, configPath = codexTargetIn(t, dir)
				} else {
					target, configPath = claudeTargetIn(t, dir)
				}
				result, err := Grant(target)
				if err == nil || result.Written {
					t.Fatalf("Grant = (%+v, %v), want an error and nothing written", result, err)
				}
				if _, statErr := os.Stat(configPath); !os.IsNotExist(statErr) {
					t.Errorf("the configuration file was created: %v", statErr)
				}
			})
		}
	}
}

// A folder inside a repository whose root is the home folder would make the
// CLI's own trust target the whole home folder. Grant does not write that.
func TestGrantRefusesTheHomeFolderAsTheTrustTarget(t *testing.T) {
	repo := newGitRepo(t)
	t.Setenv("HOME", repo)
	t.Setenv("USERPROFILE", repo)
	sub := filepath.Join(repo, "project")
	if err := os.MkdirAll(sub, 0o700); err != nil {
		t.Fatal(err)
	}
	for _, provider := range []string{"codex", "claude"} {
		t.Run(provider, func(t *testing.T) {
			var target Target
			var configPath string
			if provider == "codex" {
				target, configPath = codexTargetIn(t, sub)
			} else {
				target, configPath = claudeTargetIn(t, sub)
			}
			result, err := Grant(target)
			if err == nil || result.Written {
				t.Fatalf("Grant = (%+v, %v), want an error and nothing written", result, err)
			}
			if _, statErr := os.Stat(configPath); !os.IsNotExist(statErr) {
				t.Errorf("the configuration file was created: %v", statErr)
			}
		})
	}
}

func mustJSON(t *testing.T, v any) string {
	t.Helper()
	data, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

func jsonEqual(t *testing.T, a, b any) bool {
	t.Helper()
	return mustJSON(t, a) == mustJSON(t, b)
}
