package clitrust

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/pelletier/go-toml/v2"
)

// codexDoc is the slice of config.toml this package cares about: only the
// projects table, and only trust_level within each entry. codex's own
// config.toml can hold approval_policy, sandbox_mode, mcp_servers, features
// and more; none of it is modeled here because Grant/Trusted never touch
// anything else, and appending a table never requires understanding the rest
// of the file.
type codexDoc struct {
	Projects map[string]codexProjectEntry `toml:"projects"`
}

type codexProjectEntry struct {
	TrustLevel string `toml:"trust_level"`
}

// readCodexDoc parses configPath, treating a missing file as an empty
// document — codex itself starts up fine without config.toml (child plan's
// C3 作業内容 1), and Grant must behave the same way rather than treating
// "no config yet" as an error.
func readCodexDoc(configPath string) (codexDoc, []byte, error) {
	data, err := os.ReadFile(configPath)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return codexDoc{}, nil, nil
		}
		return codexDoc{}, nil, err
	}
	var doc codexDoc
	if err := toml.Unmarshal(data, &doc); err != nil {
		return codexDoc{}, nil, fmt.Errorf("clitrust: could not read %s as TOML: %w", configPath, err)
	}
	return doc, data, nil
}

func codexExistingState(entry codexProjectEntry) string {
	switch entry.TrustLevel {
	case "trusted":
		return "trusted"
	case "untrusted":
		return "untrusted"
	default:
		return "other"
	}
}

// codexTrusted reads only. codex does not lock config.toml itself (see
// codexGrant's doc comment for why Grant also skips locking), so a plain read
// is consistent with how codex reads the file at every startup.
func codexTrusted(configPath, key string) (bool, error) {
	doc, _, err := readCodexDoc(configPath)
	if err != nil {
		return false, err
	}
	entry, ok := doc.Projects[key]
	return ok && entry.TrustLevel == "trusted", nil
}

// codexGrant is the Grant half of the codex provider: read, check, append
// (never rewrite), verify.
//
// No lock is taken here — a deliberate difference from claudeGrant, recorded
// in the child plan's 判断ログ: codex reads config.toml at startup and writes
// it only when the user answers a prompt (trust, model change, ...), so the
// window where a concurrent write could race this append is small.
//
// The append is a real O_APPEND write of the new table only. The bytes already
// in the file are never rewritten, so even a Hub killed mid-write can at worst
// leave a torn table at the end — never a truncated config. Rewriting the whole
// file instead would open a window where the user's MCP servers, model and
// other settings exist only in this process's memory.
func codexGrant(configPath, key string) (Result, error) {
	doc, original, err := readCodexDoc(configPath)
	if err != nil {
		return Result{}, err
	}

	if entry, ok := doc.Projects[key]; ok {
		return Result{Written: false, Existing: codexExistingState(entry)}, nil
	}

	if err := os.MkdirAll(filepath.Dir(configPath), 0o700); err != nil {
		return Result{}, err
	}
	sizeBefore, err := appendCodexBlock(configPath, codexTrustBlock(key, len(original) == 0))
	if err != nil {
		return Result{}, err
	}

	verifyDoc, _, verifyErr := readCodexDoc(configPath)
	if verifyErr == nil {
		if entry, ok := verifyDoc.Projects[key]; ok && entry.TrustLevel == "trusted" {
			return Result{Written: true}, nil
		}
		verifyErr = fmt.Errorf("clitrust: %s does not contain %q as trusted after writing", configPath, key)
	}

	// 追記後に読めなくなった（または期待した値になっていない）ときは、
	// 追記した分だけを切り詰めて戻す。ファイルが元々無かった場合
	// （original == nil）は削除して「無かった」状態に戻す。
	if original == nil {
		_ = os.Remove(configPath)
	} else {
		_ = os.Truncate(configPath, sizeBefore)
	}
	return Result{}, verifyErr
}

// codexTrustBlock renders the same table codex itself writes when the user
// answers the trust prompt with "Trust and continue": [projects.'<key>']
// followed by trust_level = "trusted" (confirmed on Windows — parent plan's
// T3/T5 trace). The leading blank line separates it from whatever table ends
// the file; an empty file gets no leading blank line.
func codexTrustBlock(key string, emptyFile bool) string {
	block := "[projects." + tomlQuoteKey(key) + "]\ntrust_level = \"trusted\"\n"
	if emptyFile {
		return block
	}
	return "\n" + block
}

// tomlQuoteKey quotes key as a TOML key segment. TOML's literal string
// ('...') is preferred because it needs no escaping and matches what codex
// itself writes, but a literal string cannot contain a single quote, so a key
// with one falls back to a basic (double-quoted) string with the minimal
// escaping TOML requires.
func tomlQuoteKey(key string) string {
	if !strings.Contains(key, "'") {
		return "'" + key + "'"
	}
	var b strings.Builder
	b.WriteByte('"')
	for _, r := range key {
		switch r {
		case '"', '\\':
			b.WriteByte('\\')
			b.WriteRune(r)
		default:
			b.WriteRune(r)
		}
	}
	b.WriteByte('"')
	return b.String()
}

// appendCodexBlock appends block to configPath (creating it with 0600 when
// missing) and returns the file's size before the append, so a failed verify
// can cut exactly the appended bytes back off.
func appendCodexBlock(configPath, block string) (int64, error) {
	f, err := os.OpenFile(configPath, os.O_WRONLY|os.O_APPEND|os.O_CREATE, 0o600)
	if err != nil {
		return 0, err
	}
	sizeBefore, err := f.Seek(0, io.SeekEnd)
	if err != nil {
		_ = f.Close()
		return 0, err
	}
	if _, err := f.WriteString(block); err != nil {
		_ = f.Close()
		_ = os.Truncate(configPath, sizeBefore)
		return 0, err
	}
	if err := f.Sync(); err != nil {
		_ = f.Close()
		return 0, err
	}
	return sizeBefore, f.Close()
}
