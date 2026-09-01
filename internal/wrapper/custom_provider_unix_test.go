//go:build !windows

package wrapper

import "testing"

// TestResolveCmdCustomArgvOverridesProvider は customArgv 指定時、resolveCmd が
// provider 文字列（config.yaml の id）ではなく customArgv[0] を実行ファイル解決の
// 対象にすることを確認する。args は customArgv[1:] の後ろに続く
// （plan_custom-provider-spawn-execution.md C3）。
func TestResolveCmdCustomArgvOverridesProvider(t *testing.T) {
	cmd, cmdArgs := resolveCmd("my-cli", []string{"nonexistent-cli-binary-xyz", "--agent"}, []string{"extra"})
	if cmd != "nonexistent-cli-binary-xyz" {
		t.Errorf("cmd = %q, want the unresolved customArgv[0] (not found on PATH, so returned as-is)", cmd)
	}
	if len(cmdArgs) != 2 || cmdArgs[0] != "--agent" || cmdArgs[1] != "extra" {
		t.Errorf("cmdArgs = %#v, want [--agent extra] (customArgv[1:] followed by args)", cmdArgs)
	}
}

// TestResolveCmdCustomArgvSingleElement は customArgv が実行ファイルのみ（引数無し）の
// ときに args だけが後ろに付くことを確認する。
func TestResolveCmdCustomArgvSingleElement(t *testing.T) {
	cmd, cmdArgs := resolveCmd("my-cli", []string{"nonexistent-cli-binary-xyz"}, nil)
	if cmd != "nonexistent-cli-binary-xyz" {
		t.Errorf("cmd = %q, want nonexistent-cli-binary-xyz", cmd)
	}
	if len(cmdArgs) != 0 {
		t.Errorf("cmdArgs = %#v, want empty", cmdArgs)
	}
}

// TestResolveCmdNilCustomArgvUnchangedForBuiltins は customArgv が nil のとき、
// 既存 provider（shell/copilot/その他）の解決結果が従来と同じであることを固定する
// （resolveCmd のシグネチャ変更が built-in の挙動を変えていないことの回帰）。
func TestResolveCmdNilCustomArgvUnchangedForBuiltins(t *testing.T) {
	wantShell := resolveDefaultShell()
	gotShell, gotShellArgs := resolveCmd("shell", nil, []string{"-c", "echo hi"})
	if gotShell != wantShell {
		t.Errorf("resolveCmd(shell) = %q, want %q", gotShell, wantShell)
	}
	if len(gotShellArgs) != 2 || gotShellArgs[0] != "-c" || gotShellArgs[1] != "echo hi" {
		t.Errorf("resolveCmd(shell) args = %#v, want passthrough", gotShellArgs)
	}

	// 未知 provider（PATH に無い想定の名前）は customArgv=nil のとき、従来どおり
	// provider 自身をそのまま実行ファイル名として返す（LookPath 失敗フォールバック）。
	got, gotArgs := resolveCmd("nonexistent-provider-xyz", nil, []string{"--flag"})
	if got != "nonexistent-provider-xyz" {
		t.Errorf("resolveCmd(unknown, nil, ...) cmd = %q, want the provider string unresolved", got)
	}
	if len(gotArgs) != 1 || gotArgs[0] != "--flag" {
		t.Errorf("resolveCmd(unknown, nil, ...) args = %#v, want passthrough", gotArgs)
	}
}
