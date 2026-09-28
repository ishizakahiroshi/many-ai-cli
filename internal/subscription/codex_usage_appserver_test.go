package subscription

import (
	"bufio"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestCodexAppServerUsageHelperProcess(t *testing.T) {
	if os.Getenv("MANY_AI_CLI_TEST_CODEX_USAGE_HELPER") != "1" {
		return
	}
	reader := bufio.NewScanner(os.Stdin)
	for reader.Scan() {
		var request struct {
			ID     *int   `json:"id"`
			Method string `json:"method"`
		}
		if json.Unmarshal(reader.Bytes(), &request) != nil || request.ID == nil {
			continue
		}
		root := os.Getenv(CodexHomeEnv)
		var result string
		switch request.Method {
		case "initialize":
			result = `{}`
		case "account/read":
			if strings.HasSuffix(root, "api-key") {
				result = `{"account":{"type":"apiKey"}}`
			} else {
				result = `{"account":{"type":"chatgpt","planType":"plus","email":"user@example.com"}}`
			}
		case "account/rateLimits/read":
			if strings.HasSuffix(root, "hang") {
				time.Sleep(30 * time.Second)
			}
			used := 25
			if strings.HasSuffix(root, "profile-b") {
				used = 71
			}
			result = `{"rateLimits":{"primary":{"usedPercent":` + stringNumber(used) + `,"windowDurationMins":300,"resetsAt":1800000000},"credits":{"hasCredits":true,"unlimited":false,"balance":"12"}}}`
		default:
			os.Exit(4)
		}
		_, _ = os.Stdout.WriteString(`{"id":` + stringNumber(*request.ID) + `,"result":` + result + `}` + "\n")
	}
	os.Exit(0)
}

func stringNumber(value int) string {
	encoded, _ := json.Marshal(value)
	return string(encoded)
}

func TestReadCodexAppServerUsageKeepsProfilesSeparate(t *testing.T) {
	previousLookPath, previousCommand := lookPath, codexAppServerCommand
	t.Cleanup(func() { lookPath, codexAppServerCommand = previousLookPath, previousCommand })
	lookPath = func(string) (string, error) { return os.Args[0], nil }
	codexAppServerCommand = func(ctx context.Context, path string) *exec.Cmd {
		return exec.CommandContext(ctx, path, "-test.run=^TestCodexAppServerUsageHelperProcess$")
	}
	t.Setenv("MANY_AI_CLI_TEST_CODEX_USAGE_HELPER", "1")
	base := t.TempDir()
	for _, test := range []struct {
		name string
		used float64
	}{
		{"profile-a", 25}, {"profile-b", 71},
	} {
		root := filepath.Join(base, test.name)
		if err := os.Mkdir(root, 0o700); err != nil {
			t.Fatal(err)
		}
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		usage, err := ReadCodexAppServerUsage(ctx, root)
		cancel()
		if err != nil || usage.Primary == nil || usage.Primary.UsedPercent != test.used || usage.PlanType != "plus" || usage.Credits == nil || usage.Credits.Balance != "12" {
			t.Fatalf("%s: usage=%+v err=%v", test.name, usage, err)
		}
	}
	apiKeyRoot := filepath.Join(base, "api-key")
	if err := os.Mkdir(apiKeyRoot, 0o700); err != nil {
		t.Fatal(err)
	}
	if _, err := ReadCodexAppServerUsage(context.Background(), apiKeyRoot); err == nil {
		t.Fatal("API key login was accepted as ChatGPT subscription")
	}
	if _, err := ReadCodexAppServerUsage(context.Background(), filepath.Join(base, "missing")); err == nil {
		t.Fatal("missing profile fell back to another login")
	}
	hangRoot := filepath.Join(base, "hang")
	if err := os.Mkdir(hangRoot, 0o700); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	if _, err := ReadCodexAppServerUsage(ctx, hangRoot); err == nil {
		t.Fatal("timed-out App Server read was accepted")
	}
}
