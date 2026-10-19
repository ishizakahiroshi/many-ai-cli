package subscription

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"runtime"
	"strings"
)

// ErrCLINotFound は provider の公式 CLI が PATH に無い場合。
var ErrCLINotFound = errors.New("provider CLI not found in PATH")

// lookPath / commandOutput はテスト用の seam（internal/doctor と同じ作法）。
var (
	lookPath      = exec.LookPath
	commandOutput = defaultCommandOutput
)

// runVendorCLI は profile の env を重ねて公式 CLI を 1 回実行し、標準出力と
// 終了コードを返す。
//
// **標準出力はそのままエラーメッセージへ載せない。** 例えば `claude auth status`
// は成功時にアカウントのメールアドレスを含む JSON を返すので、呼び出し側は
// 必要なフィールドだけを取り出し、残りは捨てる責務を負う。
func runVendorCLI(ctx context.Context, bin string, args, envOverlay []string) (string, int, error) {
	path, err := lookPath(bin)
	if err != nil {
		return "", 0, fmt.Errorf("%w: %s", ErrCLINotFound, bin)
	}
	out, code, err := commandOutput(ctx, path, args, mergeEnv(os.Environ(), envOverlay))
	return out, code, err
}

func defaultCommandOutput(ctx context.Context, path string, args, env []string) (string, int, error) {
	name, argv := shellSafeCommand(path, args)
	cmd := exec.CommandContext(ctx, name, argv...)
	cmd.Env = env
	var stdout bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = nil
	cmd.Stdin = nil
	err := cmd.Run()
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) {
			// 非 0 終了は「未ログイン」の正常な表現なので、エラーではなく
			// 終了コードとして返す。
			return stdout.String(), exitErr.ExitCode(), nil
		}
		return stdout.String(), 0, err
	}
	return stdout.String(), 0, nil
}

// shellSafeCommand は Windows の .cmd / .bat シムを COMSPEC 経由で起動する形へ
// 直す（CreateProcess はバッチファイルを直接起動できない）。
// path は exec.LookPath の戻り値で、利用者入力ではない。
func shellSafeCommand(path string, args []string) (string, []string) {
	if runtime.GOOS != "windows" {
		return path, args
	}
	lower := strings.ToLower(path)
	if !strings.HasSuffix(lower, ".cmd") && !strings.HasSuffix(lower, ".bat") {
		return path, args
	}
	comspec := os.Getenv("COMSPEC")
	if comspec == "" {
		comspec = `C:\Windows\System32\cmd.exe`
	}
	return comspec, append([]string{"/c", path}, args...)
}

// mergeEnv overlays KEY=VALUE entries onto base, last one wins.
func mergeEnv(base, overlay []string) []string {
	if len(overlay) == 0 {
		return base
	}
	idx := make(map[string]int, len(base))
	out := make([]string, 0, len(base)+len(overlay))
	for _, kv := range base {
		eq := strings.IndexByte(kv, '=')
		if eq <= 0 {
			out = append(out, kv)
			continue
		}
		key := envKey(kv[:eq])
		idx[key] = len(out)
		out = append(out, kv)
	}
	for _, kv := range overlay {
		eq := strings.IndexByte(kv, '=')
		if eq <= 0 {
			out = append(out, kv)
			continue
		}
		key := envKey(kv[:eq])
		if i, ok := idx[key]; ok {
			out[i] = kv
			continue
		}
		idx[key] = len(out)
		out = append(out, kv)
	}
	return out
}

// envKey normalizes the case of environment variable names on Windows, where
// they are case-insensitive. Without this, overlaying "Path=..." onto "PATH=..."
// would append a second entry instead of replacing the first.
func envKey(name string) string {
	if runtime.GOOS == "windows" {
		return strings.ToUpper(name)
	}
	return name
}
