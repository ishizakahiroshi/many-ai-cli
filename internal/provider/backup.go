package provider

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

func writeJSONAtomic(path string, value any) error {
	data, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		return fmt.Errorf("encode %s: %w", filepath.Base(path), err)
	}
	return writeBytesAtomic(path, data)
}

func writeTextAtomic(path, value string) error {
	return writeBytesAtomic(path, []byte(value+"\n"))
}

// atomicReplaceShadowSuffix marks the file writeBytesAtomic uses to hold a
// target's previous content while replacing it on a platform where rename
// cannot overwrite in one step (see the comment on the Rename fallback
// below). recoverInterruptedAtomicReplace looks for this exact suffix.
const atomicReplaceShadowSuffix = ".atomic-replace-shadow"

// recoverInterruptedAtomicReplace restores path from a leftover shadow file
// if path is missing but the shadow is present. This is the one window
// writeBytesAtomic's replace-via-rename cannot close by itself: a crash
// between "move the old file aside" and "move the new file into place"
// leaves path absent and the old content sitting in the shadow file. Callers
// that read a file writeBytesAtomic maintains should call this first so an
// interrupted write self-heals into "still has the last valid content"
// instead of "file not found" — the whole point of doing the rename dance
// (see below) instead of just deleting path outright.
//
// path is always built by this package from a store root plus validated IDs
// or fixed names, and callers in history.go / distribution.go run
// ensureInsideRoot on it first; shadow only appends a fixed suffix. That is
// the justification for the #nosec G703 markers below.
func recoverInterruptedAtomicReplace(path string) {
	shadow := path + atomicReplaceShadowSuffix
	if _, err := os.Stat(path); err == nil { // #nosec G703 -- path is a store-root path built by this package (see func comment)
		// path is healthy; a leftover shadow here is noise from a replace
		// that completed but never got to clean up, and leaving it around
		// could accidentally "recover" a stale value for a future replace's
		// own crash. Clear it now that we know path itself is fine.
		_ = os.Remove(shadow) // #nosec G703 -- shadow is path plus a fixed suffix
		return
	} else if !os.IsNotExist(err) {
		return
	}
	if _, err := os.Stat(shadow); err != nil { // #nosec G703 -- shadow is path plus a fixed suffix
		return
	}
	_ = os.Rename(shadow, path) // #nosec G703 -- both paths stay inside the store root (see func comment)
}

func writeBytesAtomic(path string, data []byte) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return fmt.Errorf("create directory for %s: %w", filepath.Base(path), err)
	}
	recoverInterruptedAtomicReplace(path)
	tmp, err := os.CreateTemp(filepath.Dir(path), ".provider-atomic-*")
	if err != nil {
		return err
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if err := tmp.Chmod(0o600); err != nil {
		_ = tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Rename(tmpName, path); err == nil {
		return nil
	}
	// Windows does not let Rename replace an existing file (POSIX does).
	// The previous fallback here called os.Remove(path) and then renamed
	// the temp file into place: a crash in that gap left path permanently
	// gone with nothing to replace it, and every caller of this function —
	// HistoryStore's HEAD pointer, the distribution accepted/previous
	// pointers — reads "file does not exist" as "never written", not as
	// "corrupted", so the loss was silent. Moving the existing file aside
	// first means that same crash instead leaves a shadow file holding the
	// content that was there before, which recoverInterruptedAtomicReplace
	// (called by this function and by the read paths in history.go and
	// distribution.go) can restore.
	shadow := path + atomicReplaceShadowSuffix
	_ = os.Remove(shadow)
	hadExisting := false
	if _, statErr := os.Stat(path); statErr == nil {
		if renameErr := os.Rename(path, shadow); renameErr != nil {
			return fmt.Errorf("move existing %s aside: %w", filepath.Base(path), renameErr)
		}
		hadExisting = true
	}
	if renameErr := os.Rename(tmpName, path); renameErr != nil {
		if hadExisting {
			if restoreErr := os.Rename(shadow, path); restoreErr != nil {
				return fmt.Errorf("replace %s failed and could not restore original: %w (restore error: %v)", filepath.Base(path), renameErr, restoreErr)
			}
		}
		return fmt.Errorf("replace %s: %w", filepath.Base(path), renameErr)
	}
	if hadExisting {
		_ = os.Remove(shadow)
	}
	return nil
}
