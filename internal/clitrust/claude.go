package clitrust

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math/rand/v2"
	"os"
	"path/filepath"
	"time"
)

// Claude Code's own .claude.json writer uses proper-lockfile: a directory
// named "<file>.lock" as the mutual-exclusion primitive, with a 10 second
// staleness window (a lock older than that is assumed to belong to a process
// that died holding it). These are vars, not consts, purely so tests can
// shrink the timeout/interval instead of a test actually waiting 5 real
// seconds for the "someone else holds the lock" case.
var (
	claudeLockRetryInterval   = 100 * time.Millisecond
	claudeLockTimeout         = 5 * time.Second
	claudeLockStaleAfter      = 10 * time.Second
	claudeRenameRetries       = 5
	claudeRenameRetryInterval = 50 * time.Millisecond
	claudeStaleTempFileAge    = time.Minute
)

// claudeDefaultProjectEntry is the JSON object Claude Code itself writes for a
// brand-new projects entry, taken verbatim from the parent plan's trace of
// Claude Code 2.1.281's embedded JS ("Claude Code 2.1.281 本体から読み取った
// 仕様"). Only hasTrustDialogAccepted is meaningful here; the other seven keys
// are included so that a project entry many-ai-cli creates is byte-for-byte
// indistinguishable from one a real "Yes" answer would have left, and so a
// later Claude Code write to this entry sees exactly the shape it expects.
const claudeDefaultProjectEntry = `{"allowedTools":[],"mcpContextUris":[],"mcpServers":{},"enabledMcpjsonServers":[],"disabledMcpjsonServers":[],"hasTrustDialogAccepted":true,"hasClaudeMdExternalIncludesApproved":false,"hasClaudeMdExternalIncludesWarningShown":false}`

// claudeProjectFlags reads only the one field Grant/Trusted care about. Using
// a narrow struct instead of a generic map means an entry with other shapes
// (a stray null, an array where an object was expected) fails to unmarshal
// into HasTrustDialogAccepted cleanly and is reported as "other" rather than
// panicking or silently defaulting to untrusted.
type claudeProjectFlags struct {
	HasTrustDialogAccepted *bool `json:"hasTrustDialogAccepted"`
}

// claudeExistingState names an entry's hasTrustDialogAccepted. "untrusted"
// is false — which Claude writes only as the default of a new entry, never as
// an answer, so claudeGrantUnderLock treats it as "not answered yet".
func claudeExistingState(entry json.RawMessage) string {
	var flags claudeProjectFlags
	if err := json.Unmarshal(entry, &flags); err != nil || flags.HasTrustDialogAccepted == nil {
		return "other"
	}
	if *flags.HasTrustDialogAccepted {
		return "trusted"
	}
	return "untrusted"
}

// readClaudeRoot reads configPath as a shallow map so every sibling of
// "projects" — oauthAccount, userID, machineID, caches, whatever a future
// Claude Code version adds — travels through untouched as json.RawMessage.
// Decoding those into concrete Go values and re-marshaling would risk exactly
// the kind of silent change this package must not make: a large integer
// losing precision through float64, for one (C2 完了条件の「大きな整数」).
//
// A missing file is not an error: a machine that has never run Claude Code
// has no ~/.claude.json yet, and Grant must be able to create one, the same
// way Claude Code's own first trust prompt would.
func readClaudeRoot(configPath string) (map[string]json.RawMessage, error) {
	data, err := os.ReadFile(configPath)
	if err != nil {
		if os.IsNotExist(err) {
			return map[string]json.RawMessage{}, nil
		}
		return nil, err
	}
	var root map[string]json.RawMessage
	if err := json.Unmarshal(data, &root); err != nil {
		return nil, fmt.Errorf("clitrust: could not read %s as JSON: %w", configPath, err)
	}
	if root == nil {
		root = map[string]json.RawMessage{}
	}
	return root, nil
}

func readClaudeProjects(root map[string]json.RawMessage) (map[string]json.RawMessage, error) {
	raw, ok := root["projects"]
	if !ok {
		return map[string]json.RawMessage{}, nil
	}
	var projects map[string]json.RawMessage
	if err := json.Unmarshal(raw, &projects); err != nil {
		return nil, fmt.Errorf(`clitrust: could not read "projects" as an object: %w`, err)
	}
	if projects == nil {
		projects = map[string]json.RawMessage{}
	}
	return projects, nil
}

func claudeTrustedDir(configPath, dir string) (bool, error) {
	return claudeTrusted(configPath, claudePlanFor(dir))
}

func claudeGrantDir(configPath, dir string) (Result, error) {
	plan := claudePlanFor(dir)
	if trustTargetTooBroad(plan.target) {
		return Result{Key: plan.writeKey}, errTrustTargetTooBroad
	}
	return claudeGrant(configPath, plan)
}

// claudeTrusted reads only — it never takes the lock, matching how Claude
// Code itself reads the file at every startup without locking.
func claudeTrusted(configPath string, plan claudePlan) (bool, error) {
	root, err := readClaudeRoot(configPath)
	if err != nil {
		return false, err
	}
	projects, err := readClaudeProjects(root)
	if err != nil {
		return false, err
	}
	_, trusted := claudeTrustingKey(projects, plan)
	return trusted, nil
}

// claudeTrustingKey is Claude's own trust check (pI): the project key first,
// then the folder and each parent in plan.walk. It returns the key whose entry
// is trusted.
func claudeTrustingKey(projects map[string]json.RawMessage, plan claudePlan) (string, bool) {
	for _, key := range append([]string{plan.writeKey}, plan.walk...) {
		if entry, ok := projects[key]; ok && claudeExistingState(entry) == "trusted" {
			return key, true
		}
	}
	return "", false
}

// claudeGrant is the Grant half of the Claude provider. Every step mirrors a
// numbered step in the child plan's C2 ("作業内容"): lock, re-read, record the
// key only if Claude does not trust the folder yet, write atomically, unlock,
// then verify.
func claudeGrant(configPath string, plan claudePlan) (Result, error) {
	key := plan.writeKey
	result, err := claudeGrantUnderLock(configPath, plan)
	if err != nil || !result.Written {
		return result, err
	}
	// 手順 5: ロックを外してから読み直す。Claude Code は読み直しに失敗した
	// とき、ロック中でも手元のキャッシュを土台に書くことがある（本体の
	// saveConfigWithLock の re-base 経路）。その書き込みで消えていないかを、
	// 他のプロセスがロックを取れる状態に戻してから確かめる。
	root, err := readClaudeRoot(configPath)
	if err != nil {
		return Result{Key: key}, err
	}
	projects, err := readClaudeProjects(root)
	if err != nil {
		return Result{Key: key}, err
	}
	if entry, ok := projects[key]; !ok || claudeExistingState(entry) != "trusted" {
		return Result{Key: key}, fmt.Errorf("clitrust: %s does not contain %q as trusted after writing", configPath, key)
	}
	return result, nil
}

func claudeGrantUnderLock(configPath string, plan claudePlan) (Result, error) {
	key := plan.writeKey
	release, err := acquireClaudeLock(configPath + ".lock")
	if err != nil {
		// 呼び出し側はロックが取れなくても子の起動は止めない（不変条件 4）。
		// その場合、子の画面には確認がそのまま出る。
		return Result{Key: key}, err
	}
	defer release()

	root, err := readClaudeRoot(configPath)
	if err != nil {
		return Result{Key: key}, err
	}
	projects, err := readClaudeProjects(root)
	if err != nil {
		return Result{Key: key}, err
	}

	if trustingKey, ok := claudeTrustingKey(projects, plan); ok {
		return Result{Written: false, Key: trustingKey, Existing: "trusted"}, nil
	}

	entry, exists := projects[key]
	switch {
	case !exists:
		projects[key] = json.RawMessage(claudeDefaultProjectEntry)
	case claudeExistingState(entry) == "untrusted":
		// hasTrustDialogAccepted:false is the default every new entry starts
		// with, not an answer: Claude's "No, exit" writes nothing, and its
		// "Yes" turns this same entry into {...entry, hasTrustDialogAccepted:
		// true}. Do exactly that — every other field stays as it was.
		accepted, err := claudeAcceptedEntry(entry)
		if err != nil {
			return Result{Written: false, Key: key, Existing: "other"}, nil
		}
		projects[key] = accepted
	default:
		return Result{Written: false, Key: key, Existing: claudeExistingState(entry)}, nil
	}

	projectsJSON, err := marshalClaudeJSON(projects, "")
	if err != nil {
		return Result{Key: key}, err
	}
	root["projects"] = projectsJSON

	if err := writeClaudeRootAtomically(configPath, root); err != nil {
		return Result{Key: key}, err
	}
	return Result{Written: true, Key: key}, nil
}

// claudeAcceptedEntry returns entry with hasTrustDialogAccepted set to true
// and every other field carried through as raw JSON (so a large number or a
// nested value comes back unchanged).
func claudeAcceptedEntry(entry json.RawMessage) (json.RawMessage, error) {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(entry, &fields); err != nil || fields == nil {
		return nil, fmt.Errorf("clitrust: project entry is not an object")
	}
	fields["hasTrustDialogAccepted"] = json.RawMessage("true")
	return marshalClaudeJSON(fields, "")
}

// acquireClaudeLock takes the proper-lockfile-shaped directory lock at
// lockDir, retrying every claudeLockRetryInterval up to claudeLockTimeout. If
// the lock is held but its mtime is older than claudeLockStaleAfter, it is
// removed and retaken exactly once — mirroring proper-lockfile's own stale
// handling — rather than treated as permanently unavailable.
func acquireClaudeLock(lockDir string) (release func(), err error) {
	deadline := time.Now().Add(claudeLockTimeout)
	staleReclaimed := false
	for {
		mkErr := os.Mkdir(lockDir, 0o700)
		if mkErr == nil {
			return func() { _ = os.Remove(lockDir) }, nil
		}
		if !os.IsExist(mkErr) {
			return nil, fmt.Errorf("clitrust: could not create lock %s: %w", lockDir, mkErr)
		}
		if !staleReclaimed {
			if info, statErr := os.Stat(lockDir); statErr == nil && time.Since(info.ModTime()) > claudeLockStaleAfter {
				_ = os.Remove(lockDir)
				staleReclaimed = true
				continue
			}
		}
		if time.Now().After(deadline) {
			return nil, fmt.Errorf("clitrust: could not acquire lock %s within %s", lockDir, claudeLockTimeout)
		}
		time.Sleep(claudeLockRetryInterval)
	}
}

// marshalClaudeJSON encodes v the way Claude Code's JSON.stringify does, as
// far as the bytes differ in ways a reader of the file would notice: no
// <-style escaping of < > & (encoding/json's default, which
// JSON.stringify does not do — it applies to json.RawMessage values too, so
// the nested projects object needs this as well), and no trailing newline.
// Key order is not preserved: Go sorts map keys. Values are unchanged, and
// Claude Code rewrites the file in its own order on its next save.
func marshalClaudeJSON(v any, indent string) ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if indent != "" {
		enc.SetIndent("", indent)
	}
	if err := enc.Encode(v); err != nil {
		return nil, err
	}
	return bytes.TrimSuffix(buf.Bytes(), []byte("\n")), nil
}

// writeClaudeRootAtomically writes root to configPath the way Claude Code's
// own writer does (its atomic write helper, called with allowSymlink): write
// through a symlink to its target instead of replacing the link, keep the
// target's permissions, fsync a temp file next to the target, then rename it
// over the target. A concurrent reader never observes a partial file.
//
// A ~/.claude.json kept as a symlink (a dotfiles manager, a synced folder) is
// the reason for resolving first: renaming onto the link path would turn the
// link into a plain file, and the user's real file would silently stop
// receiving Claude Code's later writes.
func writeClaudeRootAtomically(configPath string, root map[string]json.RawMessage) error {
	target := configPath
	if resolved, err := filepath.EvalSymlinks(configPath); err == nil {
		target = resolved
	}
	cleanStaleClaudeTempFiles(target)

	body, err := marshalClaudeJSON(root, "  ")
	if err != nil {
		return err
	}
	mode := os.FileMode(0o600)
	if info, err := os.Stat(target); err == nil {
		mode = info.Mode().Perm()
	}
	if err := os.MkdirAll(filepath.Dir(target), 0o700); err != nil {
		return err
	}
	tmpPath := fmt.Sprintf("%s.many-ai-cli-%d-%d.tmp", target, os.Getpid(), rand.Int64())
	if err := writeSyncedFile(tmpPath, body, mode); err != nil {
		_ = os.Remove(tmpPath)
		return err
	}
	defer os.Remove(tmpPath) // rename が成功していれば対象は無く no-op。

	var renameErr error
	for attempt := 0; attempt < claudeRenameRetries; attempt++ {
		renameErr = os.Rename(tmpPath, target)
		if renameErr == nil {
			return nil
		}
		// Windows は書き込み直後の rename が共有違反で一時的に失敗することが
		// ある。原因の分類はせず、短い間隔で何度か取り直す。
		time.Sleep(claudeRenameRetryInterval)
	}
	return fmt.Errorf("clitrust: could not replace %s: %w", target, renameErr)
}

// writeSyncedFile creates path exclusively, writes body, and fsyncs before
// closing, so the rename that follows never publishes a file whose contents
// are still only in the page cache.
func writeSyncedFile(path string, body []byte, mode os.FileMode) error {
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, mode)
	if err != nil {
		return err
	}
	if _, err := f.Write(body); err != nil {
		_ = f.Close()
		return err
	}
	if err := f.Sync(); err != nil {
		_ = f.Close()
		return err
	}
	return f.Close()
}

// cleanStaleClaudeTempFiles removes leftover "*.many-ai-cli-<pid>-<n>.tmp"
// files from a previous, forcibly-killed write, but only ones older than
// claudeStaleTempFileAge. This is the "reclaim on the next start" half of the
// residue rule in internal/doctor/residue.go's header comment — the file this
// package writes is not covered by that detector, so the reclaim has to live
// here instead. Failure to clean is never fatal: a leftover temp file next to
// configPath does not stop Grant from writing its own.
func cleanStaleClaudeTempFiles(configPath string) {
	matches, err := filepath.Glob(configPath + ".many-ai-cli-*.tmp")
	if err != nil {
		return
	}
	cutoff := time.Now().Add(-claudeStaleTempFileAge)
	for _, match := range matches {
		info, statErr := os.Stat(match)
		if statErr != nil {
			continue
		}
		if info.ModTime().Before(cutoff) {
			_ = os.Remove(match)
		}
	}
}
