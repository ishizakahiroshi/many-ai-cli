// commandcode.go implements the Command Code diagnostic row.
//
// 「未導入」は既存の providers() 検査（検出できた provider だけを並べる）で
// 十分に分かるので、ここで重ねて出すと該当しない利用者の診断出力が伸びる。
// このファイルは command-code が PATH にあるときだけ、認証状態と Node バージョンを
// 追加で 1〜2 行出す（residue / subscriptions と同じ「該当があるときにしか出さない」方針）。
package doctor

import (
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"strconv"
	"strings"
)

// commandCodeStatusOutput は `command-code status --json` の実行を差し替え可能にする。
// 未認証時は終了コード 1 を返すが stdout の JSON は読めるので、呼び出し側は
// エラーの有無ではなく out の中身で判定する。
var commandCodeStatusOutput = func(ctx context.Context, path string) ([]byte, error) {
	return exec.CommandContext(ctx, path, "status", "--json").Output()
}

// commandCode は Command Code CLI の認証状態と Node バージョンを確認する。
//
// command-code が PATH に無い環境では 1 行も返さない。
func commandCode(ctx context.Context) []Check {
	path, err := providerLookPath("command-code")
	if err != nil {
		return nil
	}

	var checks []Check

	statusCtx, cancel := context.WithTimeout(ctx, providerVersionTimeout)
	out, _ := commandCodeStatusOutput(statusCtx, path)
	cancel()

	var status struct {
		Authenticated bool   `json:"authenticated"`
		Version       string `json:"version"`
	}
	if err := json.Unmarshal(out, &status); err != nil {
		checks = append(checks, Check{"Command Code", Warn, "command-code の状態を読めませんでした",
			"command-code status --json を実行して出力を確認してください"})
	} else if status.Authenticated {
		checks = append(checks, Check{"Command Code", OK, fmt.Sprintf("command-code %s はログイン済みです", status.Version), ""})
	} else {
		checks = append(checks, Check{"Command Code", Warn, fmt.Sprintf("command-code %s は未ログインです", status.Version),
			"command-code login を実行してください"})
	}

	if nodePath, err := providerLookPath("node"); err == nil {
		nodeCtx, cancel := context.WithTimeout(ctx, providerVersionTimeout)
		nodeOut, nodeErr := providerVersionOutput(nodeCtx, nodePath)
		cancel()
		// node が無い、または版を読めない場合は何も出さない。command-code が
		// 動いている以上 Node はある以上、ここで二重に警告する必要はない。
		if nodeErr == nil {
			if major, ok := parseNodeMajor(string(nodeOut)); ok && major < 22 {
				checks = append(checks, Check{"Command Code", Warn,
					fmt.Sprintf("Node.js %s は Command Code の要件（22 以上）を満たしません", firstLine(string(nodeOut))),
					"Node.js 22 以上へ更新してください"})
			}
		}
	}

	return checks
}

// parseNodeMajor は `node --version` の出力（例: "v24.15.0"）からメジャー番号を取り出す。
func parseNodeMajor(s string) (int, bool) {
	s = strings.TrimSpace(s)
	s = strings.TrimPrefix(s, "v")
	if i := strings.IndexByte(s, '.'); i >= 0 {
		s = s[:i]
	}
	n, err := strconv.Atoi(s)
	if err != nil {
		return 0, false
	}
	return n, true
}
