// Package clitrust writes a folder's trust decision into a CLI's own
// configuration file, in the same shape the CLI itself writes when its trust
// prompt is answered "yes". It exists so that granting trust on the user's
// behalf — done only when the user approves a child's launch on the spawn
// confirmation screen (parent plan's C3) — never diverges from what the CLI
// would have written itself: same file, same key, same fields.
//
// Nothing in this package drives the CLI's own binary. It reads and writes
// the configuration file directly, because the whole point is to run before
// the CLI's process exists (Trusted, checked ahead of spawn) or instead of
// answering its trust prompt (Grant, in place of the keystroke Hub used to
// send blindly — see the parent plan's "現状と問題" section for why that
// keystroke approach was abandoned).
//
// Provider differences — which file, which key shape, which write protocol —
// live in the providers table below, not in a switch on the provider name.
// Adding a provider later means adding one map entry, per the design
// principles table in CLAUDE.md ("残量ソースは... 1 本の表で持つ" applies to
// this table too, for the same reason).
package clitrust

import (
	"fmt"
	"runtime"
	"strings"

	"many-ai-cli/internal/subscription"
)

// Target names "which CLI, which folder, using which child's environment".
// Env matters because CLAUDE_CONFIG_DIR / CODEX_HOME can move the
// configuration file, and the file that matters is the child's, not the Hub
// process's own (see subscription.ClaudeStateFileFromEnv's doc comment).
type Target struct {
	// Provider is "claude" or "codex". Supported(Provider) reports whether
	// Grant/Trusted know it; anything else makes both return an error.
	Provider string
	// Env is the environment the child will be spawned with — the same slice
	// (KEY=VALUE entries) passed as spawnWrappedSpec's env, not os.Environ().
	Env []string
	// Dir is the working folder, in the same string form as
	// spawnWrappedSpec.CWD (already through go-pty's drive-letter
	// normalization on Windows).
	Dir string
}

// Result reports what Grant did, so the caller can tell the user and record
// it on the board without re-deriving the file path or key itself.
type Result struct {
	// Written is true only when this call added the key. An existing key —
	// whatever its value — always leaves Written false.
	Written bool
	// ConfigPath is the file Grant wrote to, or attempted to write to. Set
	// even when Grant returns an error, so a failure can still be reported
	// with "which file".
	ConfigPath string
	// Key is the key Grant used (or would have used) in that file.
	Key string
	// Existing describes the key's value when Written is false: "trusted",
	// "untrusted", or "other" (present but not a plain yes/no). Empty when
	// Written is true or the key was absent and Grant failed before writing.
	Existing string
}

// providerOps is the one seam between claude and codex. Everything above and
// below this type is provider-agnostic; everything that differs is one of
// these four functions.
type providerOps struct {
	configPath func(env []string) string
	key        func(dir string) string
	trusted    func(configPath, key string) (bool, error)
	grant      func(configPath, key string) (Result, error)
}

// providers holds the complete set of CLIs this package knows how to grant
// trust for. A provider not in this table is simply unsupported — Grant and
// Trusted report that as an error rather than guessing at a file format they
// have never confirmed against a real trust prompt.
var providers = map[string]providerOps{
	"claude": {
		configPath: subscription.ClaudeStateFileFromEnv,
		key:        claudeKey,
		trusted:    claudeTrusted,
		grant:      claudeGrant,
	},
	"codex": {
		configPath: subscription.CodexConfigFileFromEnv,
		key:        codexKey,
		trusted:    codexTrusted,
		grant:      codexGrant,
	},
}

// Supported reports whether provider is one Grant/Trusted know how to write
// trust for. The parent plan's C3 uses this to decide whether the spawn
// confirmation screen offers the "このフォルダを信頼済みに登録する" checkbox
// at all.
func Supported(provider string) bool {
	_, ok := providers[provider]
	return ok
}

// Trusted reports whether t.Dir's own key — no walking up to parent folders,
// unlike Claude Code's own lookup (whose ancestor-walk limit is unconfirmed;
// see the parent plan's "Claude Code 2.1.281 本体から読み取った仕様") — is
// already recorded as trusted. A missing configuration file, or a missing
// key, both mean "not trusted, no error": a CLI that has simply never seen
// this folder is the ordinary case, not a failure.
func Trusted(t Target) (bool, error) {
	ops, configPath, key, err := resolve(t)
	if err != nil {
		return false, err
	}
	return ops.trusted(configPath, key)
}

// Grant records t.Dir as trusted, but only when the key is not present at
// all. An existing key — trusted, untrusted, or anything else — is left
// exactly as it is; that is the user's own prior decision (invariant 3 in the
// parent plan), and Grant is never the thing that overrides it.
func Grant(t Target) (Result, error) {
	ops, configPath, key, err := resolve(t)
	if err != nil {
		return Result{}, err
	}
	result, err := ops.grant(configPath, key)
	result.ConfigPath = configPath
	result.Key = key
	return result, err
}

func resolve(t Target) (providerOps, string, string, error) {
	ops, ok := providers[t.Provider]
	if !ok {
		return providerOps{}, "", "", fmt.Errorf("clitrust: unsupported provider %q", t.Provider)
	}
	configPath := ops.configPath(t.Env)
	if configPath == "" {
		return providerOps{}, "", "", fmt.Errorf("clitrust: could not resolve the %s configuration file", t.Provider)
	}
	return ops, configPath, ops.key(t.Dir), nil
}

// claudeKey mirrors the project key Claude Code itself computes for a working
// directory: forward slashes, and the drive letter (if any) forced to upper
// case. Everything else keeps whatever case it already had — Claude's own
// keys do too. A real ~/.claude.json was seen holding two projects entries
// that differ only in case (the shape of "C:/work/sampleApp" next to
// "C:/work/SampleApp"), so this function must not normalize case beyond the
// drive letter.
//
// This is Claude's own key format, not this OS's path separator, so the
// forward-slash rewrite happens unconditionally rather than through
// filepath.ToSlash. filepath.ToSlash is a no-op wherever '/' is already the
// OS separator, which would leave a Windows-shaped input's backslashes
// untouched when this runs — or is tested — on Linux.
func claudeKey(dir string) string {
	key := strings.ReplaceAll(dir, `\`, "/")
	if len(key) >= 2 && key[1] == ':' && key[0] >= 'a' && key[0] <= 'z' {
		key = string(key[0]-'a'+'A') + key[1:]
	}
	return key
}

// codexKey mirrors the key codex itself writes into config.toml's
// [projects.'<key>'] table. Confirmed on Windows only — the parent plan's T3
// (codex's own write) and T5 (this same rule, applied by hand) matched: the
// absolute path lowercased, backslashes kept as-is. The macOS/Linux shape is
// unconfirmed (this child plan's 判断ログ), so the path is left unchanged
// there rather than guessing at a transformation that might not match what
// codex actually writes. runtime.GOOS is the right switch here, not a
// property of dir: the Hub only ever spawns children on the OS it itself runs
// on, so "is this a Windows path" and "is this process on Windows" agree.
func codexKey(dir string) string {
	if runtime.GOOS != "windows" {
		return dir
	}
	return strings.ToLower(dir)
}
