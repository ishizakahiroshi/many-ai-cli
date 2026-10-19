package clitrust

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
	"time"
)

// writeAgedFile writes a synthetic file and backdates it by age.
func writeAgedFile(t *testing.T, path string, age time.Duration) string {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(`{"synthetic":true}`), 0o600); err != nil {
		t.Fatal(err)
	}
	when := time.Now().Add(-age)
	if err := os.Chtimes(path, when, when); err != nil {
		t.Fatal(err)
	}
	return path
}

func assertGone(t *testing.T, path string) {
	t.Helper()
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Errorf("%s survived (stat err = %v)", filepath.Base(path), err)
	}
}

func assertKept(t *testing.T, path string) {
	t.Helper()
	if _, err := os.Stat(path); err != nil {
		t.Errorf("%s was removed: %v", filepath.Base(path), err)
	}
}

// v0.9 release review C10-C3: the leftover of a Grant killed between writing
// its temp file and renaming it holds a copy of the whole .claude.json
// (oauthAccount included). A folder whose name carries a glob metacharacter —
// "[" is legal in a Windows or Unix folder name, and a hand-written
// profile_dir can hold one — made the old filepath.Glob lookup fail or match
// nothing, so the copy was never reclaimed.
func TestGrantReclaimsAnOldTempFileBesideAConfigWhosePathHasGlobCharacters(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "profile[1")
	configPath := filepath.Join(dir, ".claude.json")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(configPath, []byte(`{}`), 0o600); err != nil {
		t.Fatal(err)
	}
	leftover := writeAgedFile(t, configPath+".many-ai-cli-4242-7.tmp", time.Hour)

	if _, err := claudeGrant(configPath, claudeKeyPlan("D:/tmp/sample-repo")); err != nil {
		t.Fatalf("claudeGrant: %v", err)
	}
	assertGone(t, leftover)
}

// The Hub-start reclaim (C10-C3): beside every configuration file the targets
// select, only this package's own old leftovers go. A temp file younger than
// claudeStaleTempFileAge may belong to a Grant still running (in this Hub or
// another), and anything not named the way writeClaudeRootAtomically names its
// temp file is somebody else's.
func TestReclaimStaleTempFilesRemovesOnlyOldLeftoversBesideEachConfig(t *testing.T) {
	base := t.TempDir()
	defaultDir := filepath.Join(base, "default")
	profileDir := filepath.Join(base, "profile")
	codexDir := filepath.Join(base, "codex")
	var gone, kept []string
	for _, dir := range []string{defaultDir, profileDir} {
		configPath := filepath.Join(dir, ".claude.json")
		kept = append(kept, writeAgedFile(t, configPath, time.Hour))
		gone = append(gone, writeAgedFile(t, configPath+".many-ai-cli-4242-7.tmp", time.Hour))
		kept = append(kept,
			writeAgedFile(t, configPath+".many-ai-cli-4242-8.tmp", 0), // a Grant still writing
			writeAgedFile(t, configPath+".backup", time.Hour),
			writeAgedFile(t, filepath.Join(dir, "settings.json.many-ai-cli-1-2.tmp"), time.Hour),
		)
	}
	// A directory under the leftover's name is not a file this package wrote.
	oddDir := filepath.Join(defaultDir, ".claude.json.many-ai-cli-9-9.tmp")
	if err := os.Mkdir(oddDir, 0o700); err != nil {
		t.Fatal(err)
	}
	old := time.Now().Add(-time.Hour)
	if err := os.Chtimes(oddDir, old, old); err != nil {
		t.Fatal(err)
	}
	kept = append(kept, oddDir)
	// codex appends to config.toml in place and writes no temp file, so a
	// lookalike beside it is not touched.
	kept = append(kept, writeAgedFile(t, filepath.Join(codexDir, "config.toml.many-ai-cli-1-2.tmp"), time.Hour))

	got := ReclaimStaleTempFiles([]Target{
		{Provider: "claude", Env: []string{"CLAUDE_CONFIG_DIR=" + defaultDir}},
		{Provider: "claude", Env: []string{"CLAUDE_CONFIG_DIR=" + profileDir}},
		{Provider: "claude", Env: []string{"CLAUDE_CONFIG_DIR=" + profileDir}}, // the same file twice
		{Provider: "claude", Env: []string{"CLAUDE_CONFIG_DIR=" + filepath.Join(base, "never-used")}},
		{Provider: "codex", Env: []string{"CODEX_HOME=" + codexDir}},
		{Provider: "someday-cli"},
	})

	if want := (ReclaimResult{Removed: 2}); got != want {
		t.Errorf("ReclaimStaleTempFiles = %+v, want %+v", got, want)
	}
	for _, path := range gone {
		assertGone(t, path)
	}
	for _, path := range kept {
		assertKept(t, path)
	}
}

// writeClaudeRootAtomically puts its temp file beside the symlink's target, so
// the reclaim has to look there too — not beside the link.
func TestReclaimStaleTempFilesLooksBesideASymlinkedConfigsTarget(t *testing.T) {
	base := t.TempDir()
	realDir := filepath.Join(base, "dotfiles")
	linkDir := filepath.Join(base, "home")
	target := writeAgedFile(t, filepath.Join(realDir, "claude.json"), time.Hour)
	if err := os.MkdirAll(linkDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(target, filepath.Join(linkDir, ".claude.json")); err != nil {
		t.Skipf("symlinks are not available here: %v", err)
	}
	leftover := writeAgedFile(t, target+".many-ai-cli-4242-7.tmp", time.Hour)

	got := ReclaimStaleTempFiles([]Target{{Provider: "claude", Env: []string{"CLAUDE_CONFIG_DIR=" + linkDir}}})

	if want := (ReclaimResult{Removed: 1}); got != want {
		t.Errorf("ReclaimStaleTempFiles = %+v, want %+v", got, want)
	}
	assertGone(t, leftover)
	assertKept(t, target)
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

// A leftover that cannot be deleted is counted, not fatal, and does not stop
// the other configuration files from being reclaimed (the Hub start must not
// stop on this — C10-C3 完了条件).
func TestReclaimStaleTempFilesCountsAFailureAndCarriesOn(t *testing.T) {
	base := t.TempDir()
	stuckDir := filepath.Join(base, "stuck")
	stuck := writeAgedFile(t, filepath.Join(stuckDir, ".claude.json.many-ai-cli-1-2.tmp"), time.Hour)
	holdUndeletable(t, stuck)
	goodDir := filepath.Join(base, "good")
	leftover := writeAgedFile(t, filepath.Join(goodDir, ".claude.json.many-ai-cli-1-2.tmp"), time.Hour)

	got := ReclaimStaleTempFiles([]Target{
		{Provider: "claude", Env: []string{"CLAUDE_CONFIG_DIR=" + stuckDir}},
		{Provider: "claude", Env: []string{"CLAUDE_CONFIG_DIR=" + goodDir}},
	})

	if want := (ReclaimResult{Removed: 1, Failed: 1}); got != want {
		t.Errorf("ReclaimStaleTempFiles = %+v, want %+v", got, want)
	}
	assertGone(t, leftover)
}
