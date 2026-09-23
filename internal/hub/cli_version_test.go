package hub

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/provider"
)

// TestCliVersionFakeCLIHelper is "the CLI whose --version many-ai-cli would
// run", the same technique CLAUDE/coding.md's 子プロセスへ渡す env
// を足したら実測する section and internal/hub/subscription_integration_test.go
// use: the test binary re-execs itself with a mode env var and an
// -test.run=... argv selecting only this test (no regexp anchors: "^" is
// blocked by provider.validateArg as shell-expansion syntax, and an
// unanchored substring match is precise enough within this one package's
// test set). It exits before printing any
// go test framework output (PASS lines etc.) so captured version_text is
// exactly what the mode below writes — nothing else in this file may run
// before this early return in a normal `go test` invocation.
func TestCliVersionFakeCLIHelper(t *testing.T) {
	mode := os.Getenv("MANY_AI_CLI_FAKE_VERSION_MODE")
	if mode == "" {
		return
	}
	switch mode {
	case "normal":
		fmt.Println("fakecli version 1.2.3")
	case "multiline":
		fmt.Println("")
		fmt.Println("fakecli version 1.2.3")
		fmt.Println("build abc123")
	case "exit1":
		fmt.Fprintln(os.Stderr, "fakecli: boom")
		os.Exit(1)
	case "empty":
		// no output
	case "hang":
		time.Sleep(30 * time.Second)
		if marker := os.Getenv("MANY_AI_CLI_FAKE_VERSION_MARKER"); marker != "" {
			_ = os.WriteFile(marker, []byte("survived"), 0o600)
		}
	}
	os.Exit(0)
}

// withFakeCLILookup makes providerCommandLookPath resolve exactly one
// candidate name to this test binary (so exec.Command actually runs
// TestCliVersionFakeCLIHelper above) and everything else to "not found".
func withFakeCLILookup(t *testing.T, candidateName string) {
	t.Helper()
	original := providerCommandLookPath
	t.Cleanup(func() { providerCommandLookPath = original })
	exe := os.Args[0]
	providerCommandLookPath = func(file string) (string, error) {
		if file == candidateName {
			return exe, nil
		}
		return "", fmt.Errorf("not found: %s", file)
	}
}

func fakeCLIDefinition(id, mode string) provider.EffectiveDefinition {
	return provider.EffectiveDefinition{
		Definition: provider.Definition{
			SchemaVersion: provider.CurrentSchemaVersion,
			ID:            id,
			DisplayName:   "Fake CLI",
			Launch:        &provider.LaunchDefinition{Executable: id},
			Update:        &provider.UpdateDefinition{VersionArgs: []string{"-test.run=TestCliVersionFakeCLIHelper"}},
		},
	}
}

func TestCheckOneCliVersionNormalOutputIsVersionLine(t *testing.T) {
	withFakeCLILookup(t, "fakecli")
	t.Setenv("MANY_AI_CLI_FAKE_VERSION_MODE", "normal")

	result := checkOneCLIVersion(context.Background(), "fakecli", fakeCLIDefinition("fakecli", "normal"))
	if result.Error != "" {
		t.Fatalf("Error = %q, want empty", result.Error)
	}
	if result.VersionLine != "fakecli version 1.2.3" {
		t.Fatalf("VersionLine = %q", result.VersionLine)
	}
	if result.ExitCode != 0 {
		t.Fatalf("ExitCode = %d, want 0", result.ExitCode)
	}
}

func TestCheckOneCliVersionMultilineOutputTakesFirstNonEmptyLine(t *testing.T) {
	withFakeCLILookup(t, "fakecli")
	t.Setenv("MANY_AI_CLI_FAKE_VERSION_MODE", "multiline")

	result := checkOneCLIVersion(context.Background(), "fakecli", fakeCLIDefinition("fakecli", "multiline"))
	if result.Error != "" {
		t.Fatalf("Error = %q, want empty", result.Error)
	}
	if result.VersionLine != "fakecli version 1.2.3" {
		t.Fatalf("VersionLine = %q, want first non-empty line (leading blank line skipped)", result.VersionLine)
	}
}

func TestCheckOneCliVersionNonZeroExit(t *testing.T) {
	withFakeCLILookup(t, "fakecli")
	t.Setenv("MANY_AI_CLI_FAKE_VERSION_MODE", "exit1")

	result := checkOneCLIVersion(context.Background(), "fakecli", fakeCLIDefinition("fakecli", "exit1"))
	if result.Error != "終了コード 1" {
		t.Fatalf("Error = %q, want 終了コード 1", result.Error)
	}
	if result.ExitCode != 1 {
		t.Fatalf("ExitCode = %d, want 1", result.ExitCode)
	}
}

func TestCheckOneCliVersionEmptyOutput(t *testing.T) {
	withFakeCLILookup(t, "fakecli")
	t.Setenv("MANY_AI_CLI_FAKE_VERSION_MODE", "empty")

	result := checkOneCLIVersion(context.Background(), "fakecli", fakeCLIDefinition("fakecli", "empty"))
	if result.Error != "出力が空" {
		t.Fatalf("Error = %q, want 出力が空", result.Error)
	}
	if result.ExitCode != 0 {
		t.Fatalf("ExitCode = %d, want 0", result.ExitCode)
	}
}

func TestCheckOneCliVersionNotFound(t *testing.T) {
	withFakeCLILookup(t, "fakecli")
	// "other" is not the resolved candidate, so providerCommandLookPath misses it.
	result := checkOneCLIVersion(context.Background(), "other", fakeCLIDefinition("other", ""))
	if result.Error != "見つからない" {
		t.Fatalf("Error = %q, want 見つからない", result.Error)
	}
}

// TestCheckOneCliVersionTimeoutKillsProcess pins the C1 completion condition:
// the timeout is exercised without waiting out the real 10s default (via the
// cliVersionCheckTimeout test seam), and the fake CLI process must actually
// be gone afterward, not merely abandoned by the Go side. The fake CLI only
// writes its marker file after a 30s sleep; if the marker exists once we give
// it a grace period past our short timeout, the process was not killed.
func TestCheckOneCliVersionTimeoutKillsProcess(t *testing.T) {
	withFakeCLILookup(t, "fakecli")
	t.Setenv("MANY_AI_CLI_FAKE_VERSION_MODE", "hang")
	marker := filepath.Join(t.TempDir(), "survived.marker")
	t.Setenv("MANY_AI_CLI_FAKE_VERSION_MARKER", marker)

	originalTimeout := cliVersionCheckTimeout
	cliVersionCheckTimeout = 200 * time.Millisecond
	t.Cleanup(func() { cliVersionCheckTimeout = originalTimeout })

	start := time.Now()
	result := checkOneCLIVersion(context.Background(), "fakecli", fakeCLIDefinition("fakecli", "hang"))
	if result.Error != "打ち切り" {
		t.Fatalf("Error = %q, want 打ち切り", result.Error)
	}
	if elapsed := time.Since(start); elapsed > 5*time.Second {
		t.Fatalf("checkOneCLIVersion took %v, want well under the 30s hang duration", elapsed)
	}

	// Give a killed-but-slow-to-reap process a moment, then confirm it never
	// reached the point of writing the marker.
	time.Sleep(1 * time.Second)
	if _, err := os.Stat(marker); err == nil {
		t.Fatal("fake CLI process was not killed: marker file was written after the timeout")
	}
}

func newCLIVersionTestServer(t *testing.T) *Server {
	t.Helper()
	s := newTestServer()
	s.cfg.Token = "test-token"
	s.cliVersions = newCLIVersionState()
	fakeDef := provider.Definition{
		SchemaVersion: provider.CurrentSchemaVersion,
		ID:            "fakecli",
		DisplayName:   "Fake CLI",
		Launch:        &provider.LaunchDefinition{Executable: "fakecli"},
		Update:        &provider.UpdateDefinition{VersionArgs: []string{"-test.run=TestCliVersionFakeCLIHelper"}},
	}
	registry, diagnostics, err := buildProviderRegistry(s.cfg, []provider.Definition{fakeDef})
	if err != nil {
		t.Fatalf("build provider registry: %v", err)
	}
	for _, d := range diagnostics {
		if d.IsError() {
			t.Fatalf("unexpected registry diagnostic: %#v", d)
		}
	}
	s.providers = registry
	withFakeCLILookup(t, "fakecli")
	t.Setenv("MANY_AI_CLI_FAKE_VERSION_MODE", "normal")
	return s
}

func TestHandleCliVersionsRequiresToken(t *testing.T) {
	s := newCLIVersionTestServer(t)
	req := httptest.NewRequest(http.MethodPost, "/api/cli-versions", nil)
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleCLIVersions(resp, req)
	if resp.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want 401", resp.Code)
	}
}

func TestHandleCliVersionsPostThenGetReturnsSameResult(t *testing.T) {
	s := newCLIVersionTestServer(t)

	postReq := httptest.NewRequest(http.MethodPost, "/api/cli-versions?token=test-token", nil)
	postReq.Header.Set("Origin", "http://127.0.0.1:47777")
	postReq.Host = "127.0.0.1:47777"
	postResp := httptest.NewRecorder()
	s.handleCLIVersions(postResp, postReq)
	if postResp.Code != http.StatusOK {
		t.Fatalf("POST status = %d, body=%s", postResp.Code, postResp.Body.String())
	}
	var posted cliVersionResponse
	if err := json.Unmarshal(postResp.Body.Bytes(), &posted); err != nil {
		t.Fatalf("decode POST body: %v", err)
	}

	getReq := httptest.NewRequest(http.MethodGet, "/api/cli-versions?token=test-token", nil)
	getReq.Host = "127.0.0.1:47777"
	getResp := httptest.NewRecorder()
	s.handleCLIVersions(getResp, getReq)
	if getResp.Code != http.StatusOK {
		t.Fatalf("GET status = %d, body=%s", getResp.Code, getResp.Body.String())
	}
	var got cliVersionResponse
	if err := json.Unmarshal(getResp.Body.Bytes(), &got); err != nil {
		t.Fatalf("decode GET body: %v", err)
	}
	if got.CheckedAt != posted.CheckedAt {
		t.Fatalf("GET checked_at = %q, want the POST result's %q", got.CheckedAt, posted.CheckedAt)
	}
	if len(got.Results) != len(posted.Results) {
		t.Fatalf("GET results = %#v, want the same as POST %#v", got.Results, posted.Results)
	}
}

func TestHandleCliVersionsPostFiltersByProvidersField(t *testing.T) {
	s := newCLIVersionTestServer(t)
	body := `{"providers":["fakecli"]}`
	req := httptest.NewRequest(http.MethodPost, "/api/cli-versions?token=test-token", strings.NewReader(body))
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Header.Set("Content-Type", "application/json")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleCLIVersions(resp, req)
	if resp.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", resp.Code, resp.Body.String())
	}
	var got cliVersionResponse
	if err := json.Unmarshal(resp.Body.Bytes(), &got); err != nil {
		t.Fatalf("decode body: %v", err)
	}
	if len(got.Results) != 1 || got.Results[0].Provider != "fakecli" {
		t.Fatalf("Results = %#v, want exactly one fakecli result", got.Results)
	}
	if got.Results[0].VersionLine != "fakecli version 1.2.3" {
		t.Fatalf("Results[0].VersionLine = %q", got.Results[0].VersionLine)
	}
}
