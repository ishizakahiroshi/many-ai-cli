// Package clitrust writes a folder's trust decision into a CLI's own
// configuration file, in the same shape the CLI itself writes when its trust
// prompt is answered "yes". It exists so that granting trust on the user's
// behalf — done only when the user approves a child's launch on the spawn
// confirmation screen (parent plan's C3) — never diverges from what the CLI
// would have written itself: same file, same key, same fields.
//
// "Same key" means the CLI's own trust target, not the child's working folder:
// both CLIs record trust for the repository a folder belongs to (the main
// repository for a git worktree), and only fall back to the folder itself
// outside git. Writing the folder instead left one entry per worktree child
// behind in the user's file, and for codex it could outvote a repository the
// user had marked untrusted (v0.9 release review, C4-A F2・F4・F5). Each
// provider file ports its CLI's own lookup: codex_project.go (codex-rs
// rust-v0.156.1) and claude_project.go (Claude Code 2.1.282's embedded JS).
//
// Nothing in this package drives the CLI's own binary. It reads and writes
// the configuration file directly, because the whole point is to run before
// the CLI's process exists (Trusted, checked ahead of spawn) or instead of
// answering its trust prompt (Grant, in place of the keystroke Hub used to
// send blindly — see the parent plan's "現状と問題" section for why that
// keystroke approach was abandoned).
//
// Provider differences — which file, which key, which write protocol — live
// in the providers table below, not in a switch on the provider name. Adding
// a provider later means adding one map entry, per the design principles
// table in CLAUDE.md ("残量ソースは... 1 本の表で持つ" applies to this table
// too, for the same reason).
package clitrust

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
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
	// spawnWrappedSpec.CWD. Neither the Hub nor go-pty rewrites it before the
	// CLI starts (go-pty hands it to CreateProcess as-is), so this is the
	// folder the CLI sees as its current directory.
	Dir string
}

// Result reports what Grant did, so the caller can tell the user and record
// it on the board without re-deriving the file path or key itself.
type Result struct {
	// Written is true only when this call recorded trust: it added the key,
	// or (Claude only) turned an entry the user never answered into a trusted
	// one. Any decision already in the file — trusted, untrusted, or a shape
	// this package does not understand — leaves Written false.
	Written bool
	// ConfigPath is the file Grant wrote to, or attempted to write to. Set
	// even when Grant returns an error, so a failure can still be reported
	// with "which file".
	ConfigPath string
	// Key is the key Grant wrote, or — when it did not write — the key whose
	// existing entry decided that. Empty only when Grant failed before it could
	// work out a key.
	Key string
	// Existing describes that existing entry when Written is false: "trusted",
	// "untrusted", or "other" (present but not a plain yes/no). Empty when
	// Written is true or Grant failed before reading the file.
	Existing string
}

// providerOps is the one seam between claude and codex. Everything above and
// below this type is provider-agnostic; everything that differs is one of
// these functions. grant and trusted take the child's working folder and work
// out the CLI's own key themselves, because which key that is (the folder,
// its repository, the main repository of a worktree) is itself a provider
// difference.
type providerOps struct {
	configPath func(env []string) string
	trusted    func(configPath, dir string) (bool, error)
	grant      func(configPath, dir string) (Result, error)
}

// providers holds the complete set of CLIs this package knows how to grant
// trust for. A provider not in this table is simply unsupported — Grant and
// Trusted report that as an error rather than guessing at a file format they
// have never confirmed against a real trust prompt.
var providers = map[string]providerOps{
	"claude": {
		configPath: subscription.ClaudeStateFileFromEnv,
		trusted:    claudeTrustedDir,
		grant:      claudeGrantDir,
	},
	"codex": {
		configPath: subscription.CodexConfigFileFromEnv,
		trusted:    codexTrustedDir,
		grant:      codexGrantDir,
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

// Trusted reports whether the CLI would treat t.Dir as trusted, using the
// CLI's own lookup (codex: the folder, then its repository root; Claude: its
// project key, then the folder and each parent up to the repository root). A
// missing configuration file, or no matching key, both mean "not trusted, no
// error": a CLI that has simply never seen this folder is the ordinary case,
// not a failure.
func Trusted(t Target) (bool, error) {
	ops, configPath, err := resolve(t)
	if err != nil {
		return false, err
	}
	return ops.trusted(configPath, t.Dir)
}

// Grant records t.Dir's trust target as trusted, but only when the CLI has no
// decision for it yet. A decision already in the file — trusted, untrusted,
// or anything else — is left exactly as it is; that is the user's own prior
// decision (invariant 3 in the parent plan), and Grant is never the thing that
// overrides it. For codex an "untrusted" entry for the folder, the repository,
// or any folder above them counts as such a decision even when codex's own
// lookup would not reach it (the user's answer to the v0.9 review, Q4a).
func Grant(t Target) (Result, error) {
	ops, configPath, err := resolve(t)
	if err != nil {
		return Result{ConfigPath: configPath}, err
	}
	result, err := ops.grant(configPath, t.Dir)
	result.ConfigPath = configPath
	return result, err
}

func resolve(t Target) (providerOps, string, error) {
	ops, ok := providers[t.Provider]
	if !ok {
		return providerOps{}, "", fmt.Errorf("clitrust: unsupported provider %q", t.Provider)
	}
	configPath := ops.configPath(t.Env)
	if configPath == "" {
		return providerOps{}, "", fmt.Errorf("clitrust: could not resolve the %s configuration file", t.Provider)
	}
	if unsupportedFolder(t.Dir) {
		return providerOps{}, configPath, errUnsupportedFolder
	}
	return ops, configPath, nil
}

// errUnsupportedFolder is returned for a working folder given as a network
// (UNC) path or with a \\?\ / \\.\ prefix. Neither CLI's key for such a
// folder has been observed, and the codex npm shim (cmd.exe) refuses a UNC
// current directory outright, so a key written before the launch would only
// be left behind (v0.9 release review, C4-A F2). Nothing is written; the
// child asks on its own screen if it starts at all.
var errUnsupportedFolder = errors.New("clitrust: folders given as a network path or with a \\\\?\\ prefix are not registered; the child asks on its own screen")

// errTrustTargetTooBroad is returned when the CLI's own trust target for the
// folder would be the home folder or a drive / filesystem root (a folder
// inside a git repository whose root is the home folder, for example). The
// CLI itself would record that when answered "yes", but a checkbox about "this
// folder" must not silently trust everything under the user's home.
var errTrustTargetTooBroad = errors.New("clitrust: the folder's trust target is the home folder or a drive root; not registered, the child asks on its own screen")

// unsupportedFolder reports a Windows working folder given as a network path
// (\\server\share, //server/share) or with a \\?\ or \\.\ prefix. On other
// OSes a leading "//" is an ordinary absolute path.
func unsupportedFolder(dir string) bool {
	if runtime.GOOS != "windows" {
		return false
	}
	return strings.HasPrefix(dir, `\\`) || strings.HasPrefix(dir, `//`)
}

// cleanAbs is the folder as the CLI's own path.resolve / AbsolutePathBuf sees
// it: absolute, "." and ".." removed, no trailing separator, and on Windows
// backslash-separated.
func cleanAbs(dir string) string {
	if abs, err := filepath.Abs(dir); err == nil {
		return abs
	}
	return filepath.Clean(dir)
}

// trustTargetTooBroad reports whether path is the user's home folder or a
// volume / filesystem root (see errTrustTargetTooBroad).
func trustTargetTooBroad(path string) bool {
	path = filepath.Clean(path)
	if filepath.Dir(path) == path {
		return true
	}
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		return false
	}
	return samePathString(path, filepath.Clean(home))
}

// samePathString compares two cleaned paths the way the OS's file system
// usually does: ASCII case-insensitively on Windows, exactly elsewhere.
func samePathString(a, b string) bool {
	if runtime.GOOS == "windows" {
		return asciiLower(a) == asciiLower(b)
	}
	return a == b
}

// asciiLower lowercases A-Z only, like Rust's str::to_ascii_lowercase (the
// function codex uses on its Windows project keys). strings.ToLower would also
// fold non-ASCII letters, producing a key codex never writes.
func asciiLower(s string) string {
	b := []byte(s)
	for i, c := range b {
		if c >= 'A' && c <= 'Z' {
			b[i] = c + ('a' - 'A')
		}
	}
	return string(b)
}
