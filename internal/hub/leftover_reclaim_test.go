package hub

import (
	"bytes"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"log/slog"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/config"
)

func writeAgedLeftover(t *testing.T, path string, age time.Duration) string {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("synthetic"), 0o600); err != nil {
		t.Fatal(err)
	}
	when := time.Now().Add(-age)
	if err := os.Chtimes(path, when, when); err != nil {
		t.Fatal(err)
	}
	return path
}

// leftoverReclaimHome points every location the reclaim resolves at temp
// folders: the home folder (~/.claude.json, ~/.many-ai-cli/tmp, the default
// profile tree) and the Hub's own CLAUDE_CONFIG_DIR / CODEX_HOME. The real
// user's files are never listed, let alone removed — this matters doubly when
// the tests themselves run under a profile's CLAUDE_CONFIG_DIR.
func leftoverReclaimHome(t *testing.T) (home, hubEnvClaudeDir string) {
	t.Helper()
	home = t.TempDir()
	t.Setenv("USERPROFILE", home)
	t.Setenv("HOME", home)
	hubEnvClaudeDir = filepath.Join(t.TempDir(), "hub-env-claude")
	t.Setenv("CLAUDE_CONFIG_DIR", hubEnvClaudeDir)
	t.Setenv("CODEX_HOME", filepath.Join(t.TempDir(), "hub-env-codex"))
	return home, hubEnvClaudeDir
}

// Q6: at Hub start, old folder-trust temp copies of .claude.json (the default
// login's, the Hub environment's, and every Claude profile's) and old
// instruction files in ~/.many-ai-cli/tmp are removed; fresh ones, a running
// wrapper's pointer file and unrelated files stay. The log names kinds and
// counts only — no path (it carries the user name), no file name.
func TestReclaimLeftoverFilesAtHubStart(t *testing.T) {
	home, hubEnvClaudeDir := leftoverReclaimHome(t)
	customProfile := filepath.Join(t.TempDir(), "custom-profile")
	s := newTestServer()
	s.cfg.Subscriptions = config.SubscriptionProfiles{
		"claude": {{ID: "work"}, {ID: "side", ProfileDir: customProfile}},
	}
	var logs bytes.Buffer
	s.logger = slog.New(slog.NewTextHandler(&logs, nil))

	claudeDirs := []string{
		"",
		hubEnvClaudeDir,
		filepath.Join(home, ".many-ai-cli", "subscriptions", "claude", "work"),
		customProfile,
	}
	var gone, kept []string
	for _, dir := range claudeDirs {
		configPath := filepath.Join(home, ".claude.json")
		if dir != "" {
			configPath = filepath.Join(dir, ".claude.json")
		}
		kept = append(kept, writeAgedLeftover(t, configPath, time.Hour))
		gone = append(gone, writeAgedLeftover(t, configPath+".many-ai-cli-4242-7.tmp", time.Hour))
		kept = append(kept, writeAgedLeftover(t, configPath+".many-ai-cli-4242-8.tmp", 0))
	}
	tmp := filepath.Join(home, ".many-ai-cli", "tmp")
	gone = append(gone,
		writeAgedLeftover(t, filepath.Join(tmp, "prompt-aaaa.md"), 3*time.Hour),
		// 実在しないはずの PID（TestSweepStaleHeadlessPromptsKeepsALiveWrappersLaunchFile と同じ）。
		writeAgedLeftover(t, filepath.Join(tmp, "launch-2147483001-bbbb.md"), 3*time.Hour),
	)
	kept = append(kept,
		writeAgedLeftover(t, filepath.Join(tmp, "prompt-cccc.md"), time.Minute),
		// A wrapper that outlived the previous Hub reattaches to this one; its
		// CLI may still be waiting on a trust prompt without having read this.
		writeAgedLeftover(t, filepath.Join(tmp, fmt.Sprintf("launch-%d-dddd.md", os.Getpid())), 3*time.Hour),
		writeAgedLeftover(t, filepath.Join(tmp, "notes.md"), 3*time.Hour),
	)

	s.reclaimLeftoverFiles()

	for _, path := range gone {
		if _, err := os.Stat(path); !os.IsNotExist(err) {
			t.Errorf("%s survived the Hub-start reclaim (stat err = %v)", filepath.Base(path), err)
		}
	}
	for _, path := range kept {
		if _, err := os.Stat(path); err != nil {
			t.Errorf("%s was removed by the Hub-start reclaim: %v", filepath.Base(path), err)
		}
	}
	out := logs.String()
	for _, want := range []string{"folder_trust_temp_removed=4", "instruction_files_removed=2"} {
		if !strings.Contains(out, want) {
			t.Errorf("log lacks %q:\n%s", want, out)
		}
	}
	assertLeftoverLogCarriesNoPath(t, out, home, hubEnvClaudeDir, customProfile)
}

// holdUndeletable makes path impossible to delete until the test ends: on
// Windows an open handle (os.Open does not share delete access), elsewhere a
// read-only parent folder.
func holdUndeletable(t *testing.T, path string) {
	t.Helper()
	if runtime.GOOS == "windows" {
		f, err := os.Open(path)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { _ = f.Close() })
		return
	}
	if os.Geteuid() == 0 {
		t.Skip("root deletes from a read-only folder; the failure cannot be staged")
	}
	dir := filepath.Dir(path)
	if err := os.Chmod(dir, 0o500); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chmod(dir, 0o700) })
}

// A reclaim that cannot remove a file logs the kind and the count and returns
// (it has no error to hand back to Run); the rest is still reclaimed.
func TestReclaimLeftoverFilesCarriesOnPastFailures(t *testing.T) {
	home, hubEnvClaudeDir := leftoverReclaimHome(t)
	stuckTrust := writeAgedLeftover(t, filepath.Join(hubEnvClaudeDir, ".claude.json.many-ai-cli-4242-7.tmp"), time.Hour)
	holdUndeletable(t, stuckTrust)
	stuckPrompt := writeAgedLeftover(t, filepath.Join(home, ".many-ai-cli", "tmp", "prompt-aaaa.md"), 3*time.Hour)
	holdUndeletable(t, stuckPrompt)
	leftover := writeAgedLeftover(t, filepath.Join(home, ".claude.json.many-ai-cli-4242-7.tmp"), time.Hour)
	s := newTestServer()
	var logs bytes.Buffer
	s.logger = slog.New(slog.NewTextHandler(&logs, nil))

	s.reclaimLeftoverFiles()

	if _, err := os.Stat(leftover); !os.IsNotExist(err) {
		t.Errorf("the default login's leftover survived next to a failing one (stat err = %v)", err)
	}
	out := logs.String()
	for _, want := range []string{"folder_trust_temp_removed=1", "folder_trust_temp_failed=1", "instruction_files_failed=1"} {
		if !strings.Contains(out, want) {
			t.Errorf("log lacks %q:\n%s", want, out)
		}
	}
	assertLeftoverLogCarriesNoPath(t, out, home, hubEnvClaudeDir)
}

func assertLeftoverLogCarriesNoPath(t *testing.T, out string, dirs ...string) {
	t.Helper()
	forbidden := []string{".claude.json", "prompt-", "launch-", ".many-ai-cli"}
	for _, dir := range dirs {
		forbidden = append(forbidden, dir, filepath.ToSlash(dir), strings.ReplaceAll(dir, `\`, `\\`))
	}
	for _, word := range forbidden {
		if strings.Contains(out, word) {
			t.Errorf("the reclaim log carries %q; it must name kinds and counts only:\n%s", word, out)
		}
	}
}

// v0.9 release review C10-C3 (Q6): the leftovers a kill can strand — the
// folder-trust temp copy of .claude.json and the instruction files in
// ~/.many-ai-cli/tmp — are reclaimed when the Hub starts, not only the next
// time a feature happens to write the same kind of file (the rule in
// internal/doctor/residue.go). Run is not exercised by any test (it binds a
// port and writes the pid / runtime files), so the wiring is fixed from the
// source: Run hands reclaimLeftoverFiles to safeGo, which recovers a panic and
// has no way to return an error to Run — a failed reclaim cannot stop the
// start.
func TestHubRunReclaimsLeftoverFilesAtStart(t *testing.T) {
	fset := token.NewFileSet()
	file, err := parser.ParseFile(fset, "server.go", nil, 0)
	if err != nil {
		t.Fatal(err)
	}
	var run *ast.FuncDecl
	for _, decl := range file.Decls {
		fn, ok := decl.(*ast.FuncDecl)
		if ok && fn.Name.Name == "Run" && fn.Recv != nil {
			run = fn
		}
	}
	if run == nil {
		t.Fatal("func (s *Server) Run not found in server.go")
	}
	found := false
	ast.Inspect(run.Body, func(n ast.Node) bool {
		call, ok := n.(*ast.CallExpr)
		if !ok || len(call.Args) != 2 {
			return true
		}
		if fun, ok := call.Fun.(*ast.SelectorExpr); !ok || fun.Sel.Name != "safeGo" {
			return true
		}
		if arg, ok := call.Args[1].(*ast.SelectorExpr); ok && arg.Sel.Name == "reclaimLeftoverFiles" {
			found = true
		}
		return true
	})
	if !found {
		t.Fatal("Run does not start s.reclaimLeftoverFiles through safeGo; leftovers a kill strands are never reclaimed at Hub start")
	}
}
