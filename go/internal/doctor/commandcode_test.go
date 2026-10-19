package doctor

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"testing"
)

// TestCommandCodeFakeCLIHelper は「command-code status --json」や「node --version」の
// 代わりに起動される偽プロセス。指定された stdout をそのまま出力し、指定された
// 終了コードで終わる。通常のテスト実行では即 return し、親テストから env 付きで
// 起動されたときだけ働く（手本: internal/hub/subscription_integration_test.go）。
func TestCommandCodeFakeCLIHelper(t *testing.T) {
	if os.Getenv("MANY_AI_CLI_FAKE_CLI") != "1" {
		return
	}
	fmt.Fprint(os.Stdout, os.Getenv("MANY_AI_CLI_FAKE_STDOUT"))
	code, _ := strconv.Atoi(os.Getenv("MANY_AI_CLI_FAKE_EXIT"))
	os.Exit(code)
}

// fakeCLIOutput は commandCodeStatusOutput / providerVersionOutput と同じ形の関数を
// 返す。実プロセスを起動するので、終了コードが非 0 でも Output() が stdout を
// 返すという os/exec の実際の挙動ごと確認できる（モックのままだと見落とす）。
func fakeCLIOutput(stdout string, exitCode int) func(ctx context.Context, path string) ([]byte, error) {
	return func(ctx context.Context, path string) ([]byte, error) {
		cmd := exec.CommandContext(ctx, os.Args[0], "-test.run=^TestCommandCodeFakeCLIHelper$")
		cmd.Env = append(os.Environ(),
			"MANY_AI_CLI_FAKE_CLI=1",
			"MANY_AI_CLI_FAKE_STDOUT="+stdout,
			fmt.Sprintf("MANY_AI_CLI_FAKE_EXIT=%d", exitCode))
		return cmd.Output()
	}
}

// withCommandCodeFakes は providerLookPath / commandCodeStatusOutput / providerVersionOutput
// を差し替え、テスト終了時に元へ戻す。node を検出させたくない場合は nodeStdout を空にする。
func withCommandCodeFakes(t *testing.T, statusStdout string, statusExit int, nodeStdout string) {
	t.Helper()
	oldLookPath := providerLookPath
	oldStatusOutput := commandCodeStatusOutput
	oldVersionOutput := providerVersionOutput
	t.Cleanup(func() {
		providerLookPath = oldLookPath
		commandCodeStatusOutput = oldStatusOutput
		providerVersionOutput = oldVersionOutput
	})

	providerLookPath = func(name string) (string, error) {
		switch name {
		case "command-code":
			return "fake-command-code", nil
		case "node":
			if nodeStdout == "" {
				return "", errors.New("not found")
			}
			return "fake-node", nil
		default:
			return "", errors.New("not found")
		}
	}
	commandCodeStatusOutput = fakeCLIOutput(statusStdout, statusExit)
	providerVersionOutput = fakeCLIOutput(nodeStdout, 0)
}

func TestCommandCodeAbsentFromPATH(t *testing.T) {
	old := providerLookPath
	t.Cleanup(func() { providerLookPath = old })
	providerLookPath = func(name string) (string, error) { return "", errors.New("not found") }

	if got := commandCode(context.Background()); got != nil {
		t.Fatalf("commandCode() = %#v, want nil when command-code is not on PATH", got)
	}
}

func TestCommandCodeAuthenticated(t *testing.T) {
	withCommandCodeFakes(t, `{"authenticated":true,"version":"1.37.0"}`, 0, "v24.15.0\n")

	checks := commandCode(context.Background())
	if len(checks) != 1 {
		t.Fatalf("checks = %#v, want exactly 1 row (Node 24 満たすので警告なし)", checks)
	}
	if checks[0].Level != OK || !strings.Contains(checks[0].Message, "1.37.0") || !strings.Contains(checks[0].Message, "ログイン済み") {
		t.Fatalf("check = %+v, want OK naming the version as logged in", checks[0])
	}
}

// TestCommandCodeUnauthenticatedDespiteExitCode1 は、未認証時に command-code が
// 終了コード 1 を返しても JSON を読んで Warn を出すことを確認する（前提節の実測どおり）。
func TestCommandCodeUnauthenticatedDespiteExitCode1(t *testing.T) {
	withCommandCodeFakes(t, `{"authenticated":false,"version":"1.37.0"}`, 1, "v24.15.0\n")

	checks := commandCode(context.Background())
	if len(checks) != 1 {
		t.Fatalf("checks = %#v, want exactly 1 row", checks)
	}
	if checks[0].Level != Warn || !strings.Contains(checks[0].Message, "1.37.0") || !strings.Contains(checks[0].Message, "未ログイン") {
		t.Fatalf("check = %+v, want Warn naming the version as not logged in", checks[0])
	}
	if !strings.Contains(checks[0].Fix, "login") {
		t.Fatalf("check.Fix = %q, want a login hint", checks[0].Fix)
	}
}

func TestCommandCodeMalformedStatusJSON(t *testing.T) {
	withCommandCodeFakes(t, "not json", 0, "v24.15.0\n")

	checks := commandCode(context.Background())
	if len(checks) != 1 {
		t.Fatalf("checks = %#v, want exactly 1 row", checks)
	}
	if checks[0].Level != Warn || !strings.Contains(checks[0].Message, "読めませんでした") {
		t.Fatalf("check = %+v, want a Warn about being unable to read the status", checks[0])
	}
}

func TestCommandCodeNodeTooOldWarns(t *testing.T) {
	withCommandCodeFakes(t, `{"authenticated":true,"version":"1.37.0"}`, 0, "v20.11.1\n")

	checks := commandCode(context.Background())
	if len(checks) != 2 {
		t.Fatalf("checks = %#v, want 2 rows (auth + Node warning)", checks)
	}
	nodeCheck := checks[1]
	if nodeCheck.Level != Warn || !strings.Contains(nodeCheck.Message, "v20.11.1") || !strings.Contains(nodeCheck.Message, "22") {
		t.Fatalf("node check = %+v, want a Warn naming the version and the 22 requirement", nodeCheck)
	}
}

// TestCommandCodeNodeMissingOrUnparseableIsSilent は node が無い場合と、
// バージョン文字列を解釈できない場合の両方で、Node に関する行が増えないことを
// 確認する（command-code が動いている以上 Node はあるので、二重に警告しない）。
func TestCommandCodeNodeMissingOrUnparseableIsSilent(t *testing.T) {
	t.Run("node not on PATH", func(t *testing.T) {
		withCommandCodeFakes(t, `{"authenticated":true,"version":"1.37.0"}`, 0, "")
		checks := commandCode(context.Background())
		if len(checks) != 1 {
			t.Fatalf("checks = %#v, want exactly 1 row (node 不在なので警告なし)", checks)
		}
	})

	t.Run("node version unparseable", func(t *testing.T) {
		withCommandCodeFakes(t, `{"authenticated":true,"version":"1.37.0"}`, 0, "not-a-version\n")
		checks := commandCode(context.Background())
		if len(checks) != 1 {
			t.Fatalf("checks = %#v, want exactly 1 row (解釈不能なので警告なし)", checks)
		}
	})
}

func TestParseNodeMajor(t *testing.T) {
	for _, tc := range []struct {
		in      string
		want    int
		wantOK  bool
		comment string
	}{
		{in: "v24.15.0\n", want: 24, wantOK: true},
		{in: "v20.11.1", want: 20, wantOK: true},
		{in: "v9.2.0", want: 9, wantOK: true},
		{in: "not-a-version", wantOK: false},
		{in: "", wantOK: false},
	} {
		got, ok := parseNodeMajor(tc.in)
		if ok != tc.wantOK || (ok && got != tc.want) {
			t.Fatalf("parseNodeMajor(%q) = (%d, %v), want (%d, %v)", tc.in, got, ok, tc.want, tc.wantOK)
		}
	}
}
