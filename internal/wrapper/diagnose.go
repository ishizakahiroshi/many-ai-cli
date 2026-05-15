package wrapper

import (
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strings"
)

// classifyStartFailure は startProcess のエラーを Hub UI に渡すための
// 短い reason コードに変換する。i18n キー `end_reason_<code>` に対応する。
// 該当パターンが無い場合は空文字を返し、UI 側は通常の "disconnected" 表示のみとする。
func classifyStartFailure(err error) string {
	if isExecNotFound(err) {
		return "exec_not_found"
	}
	return ""
}

// isExecNotFound は `exec.LookPath` 由来の「ファイルが PATH に無い」エラーを判定する。
// Go ランタイムは Windows/Unix で文言が微妙に異なるため、errors.Is と文字列の両方で見る。
func isExecNotFound(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, exec.ErrNotFound) {
		return true
	}
	msg := err.Error()
	return strings.Contains(msg, "executable file not found") ||
		strings.Contains(msg, "file not found in %PATH%")
}

// diagnoseStartFailure は provider CLI の起動失敗時に、Hub の spawn ログ
// （wrapper の stderr を引き継ぐファイル）へ人間可読な診断ブロックを追記する。
// 「対処ヒント」は executable-not-found パターンを検知したときだけ出し、
// 別種のエラー（権限不足・ConPTY 内部障害等）でノイズにしない。
func diagnoseStartFailure(w io.Writer, provider string, providerArgs []string, startErr error) {
	fmt.Fprintln(w, "")
	fmt.Fprintln(w, "==== any-ai-cli spawn diagnostic ====")
	fmt.Fprintf(w, "provider: %s\n", provider)
	fmt.Fprintf(w, "lookup target: argv[0]=%q args=%v\n", provider, providerArgs)
	fmt.Fprintf(w, "error: %v\n", startErr)

	pathRaw := os.Getenv("PATH")
	entries := strings.Split(pathRaw, string(os.PathListSeparator))
	fmt.Fprintf(w, "PATH entries: %d (raw bytes: %d)\n", len(entries), len(pathRaw))

	for _, tool := range []string{"pnpm", "npm", "node", "scoop", "winget", "brew"} {
		if path, err := exec.LookPath(tool); err == nil {
			fmt.Fprintf(w, "  found %s: %s\n", tool, path)
		} else {
			fmt.Fprintf(w, "  missing %s\n", tool)
		}
	}

	if isExecNotFound(startErr) {
		fmt.Fprintln(w, "")
		fmt.Fprintf(w, "Hint: %q was not found on PATH at Hub start time.\n", provider)
		fmt.Fprintln(w, "  - If installed via pnpm: ensure $PNPM_HOME (or %PNPM_HOME%) is exported")
		fmt.Fprintln(w, "    in the shell that starts the Hub, then run:")
		fmt.Fprintf(w, "      any-ai-cli stop && any-ai-cli %s\n", provider)
		fmt.Fprintln(w, "    to refresh the Hub's PATH snapshot.")
		fmt.Fprintln(w, "  - Otherwise verify the provider CLI is installed and on PATH for the")
		fmt.Fprintln(w, "    parent shell (the env Hub inherited at start time).")
	}
	fmt.Fprintln(w, "==== end diagnostic ====")
}
