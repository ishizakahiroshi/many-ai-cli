package provider

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"
)

type QuarantineRecord struct {
	OriginalName string `json:"original_name"`
	Quarantined  string `json:"quarantined"`
	DetectedAt   string `json:"detected_at"`
	Reason       string `json:"reason"`
	Digest       string `json:"digest"`
}

func QuarantineFile(path, quarantineRoot, reason string) (QuarantineRecord, error) {
	if strings.TrimSpace(path) == "" || strings.TrimSpace(quarantineRoot) == "" {
		return QuarantineRecord{}, fmt.Errorf("quarantine path is required")
	}
	input, err := os.Open(path)
	if err != nil {
		return QuarantineRecord{}, err
	}
	defer input.Close()
	if err := os.MkdirAll(quarantineRoot, 0o700); err != nil {
		return QuarantineRecord{}, err
	}
	stamp := time.Now().UTC().Format("20060102T150405.000000000Z")
	name := stamp + "-" + filepath.Base(path)
	target := filepath.Join(quarantineRoot, name)
	output, err := os.OpenFile(target, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		return QuarantineRecord{}, err
	}
	hash := sha256.New()
	if _, err := io.Copy(io.MultiWriter(output, hash), input); err != nil {
		_ = output.Close()
		return QuarantineRecord{}, err
	}
	if err := output.Close(); err != nil {
		return QuarantineRecord{}, err
	}
	return QuarantineRecord{
		OriginalName: filepath.Base(path),
		Quarantined:  target,
		DetectedAt:   time.Now().UTC().Format(time.RFC3339Nano),
		Reason:       reason,
		Digest:       hex.EncodeToString(hash.Sum(nil)),
	}, nil
}

// LastVerifiedRevision returns the most recently created revision that can
// still be read back for providerID, without looking at HEAD at all. It
// exists for the case HEAD itself is what is broken: currentLocked (and
// therefore Current/List's usual path) cannot be used to find a recovery
// candidate because it starts by reading HEAD. Revisions that fail to read
// (corrupt JSON, digest mismatch, wrong provider id, ...) are skipped rather
// than treated as a fatal error, since the point of this function is to find
// whatever still works.
func (s *HistoryStore) LastVerifiedRevision(providerID string) (RevisionRecord, bool, error) {
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return RevisionRecord{}, false, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	revisionDir := filepath.Join(s.root, providerID, "revisions")
	entries, err := os.ReadDir(revisionDir)
	if os.IsNotExist(err) {
		return RevisionRecord{}, false, nil
	}
	if err != nil {
		return RevisionRecord{}, false, fmt.Errorf("list revisions: %w", err)
	}
	var best RevisionRecord
	found := false
	for _, entry := range entries {
		if entry.IsDir() || filepath.Ext(entry.Name()) != ".json" {
			continue
		}
		revision := strings.TrimSuffix(entry.Name(), ".json")
		record, readErr := s.readRevisionLocked(providerID, revision)
		if readErr != nil {
			continue
		}
		if !found || record.CreatedAt > best.CreatedAt {
			best = record
			found = true
		}
	}
	if !found {
		return RevisionRecord{}, false, nil
	}
	return best, true, nil
}

// headStatus classifies whether a provider's HEAD file needs recovery. It is
// the single rule NeedsRecovery and RecoverHead both use to decide whether a
// provider is broken, so the two can never disagree about it.
type headStatus int

const (
	// headMissing means there is no override for this provider at all (the
	// HEAD file itself does not exist) — nothing to recover.
	headMissing headStatus = iota
	// headHealthy means HEAD exists and resolves to a readable revision.
	headHealthy
	// headBroken means HEAD exists but cannot be resolved: this covers both
	// a corrupt/invalid HEAD or revision file *and* a HEAD that points at a
	// revision id with no matching file on disk (which surfaces from
	// currentLocked as an os.IsNotExist error, same as headMissing's "file
	// not found" — the two must not be conflated by checking os.IsNotExist
	// on currentLocked's error, since that error can come from either the
	// HEAD file itself or the revision file HEAD points to).
	headBroken
)

// headStatusLocked assumes the caller already holds s.mu (matches the other
// *Locked helpers' convention).
func (s *HistoryStore) headStatusLocked(providerID string) (headStatus, error) {
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return headMissing, err
	}
	headPath := filepath.Join(s.root, providerID, "HEAD")
	if err := ensureInsideRoot(s.root, headPath); err != nil {
		return headMissing, err
	}
	if _, err := os.Stat(headPath); err != nil {
		if os.IsNotExist(err) {
			return headMissing, nil
		}
		return headMissing, err
	}
	if _, err := s.currentLocked(providerID); err != nil {
		return headBroken, nil
	}
	return headHealthy, nil
}

// NeedsRecovery reports whether providerID's HEAD file exists but cannot be
// resolved to a readable current override — i.e. whether RecoverHead applies
// to it. It exists so callers (the recovery API, the history dialog) can
// tell "no override yet" (false, nothing wrong) apart from "HEAD present but
// broken" (true) using the exact same rule RecoverHead itself uses to decide
// whether to run.
//
// Before this existed, LoadOverrides quarantined HEAD on *any* currentLocked
// error — including a HEAD that simply points at a revision file that no
// longer exists, which surfaces as os.IsNotExist — while RecoverHead's
// original check treated any os.IsNotExist error from currentLocked as "no
// override at all" and refused to recover exactly that case. A provider
// LoadOverrides had already flagged as broken could not be recovered.
func (s *HistoryStore) NeedsRecovery(providerID string) (bool, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	status, err := s.headStatusLocked(providerID)
	if err != nil {
		return false, err
	}
	return status == headBroken, nil
}

// RecoverHead repairs a provider whose HEAD pointer cannot be resolved
// (corrupt/invalid HEAD or revision file, or HEAD pointing at a revision id
// that no longer exists on disk) by quarantining the broken HEAD file and
// writing a fresh revision — either the payload of an existing revision the
// caller chose (typically one LastVerifiedRevision found), or, when revision
// is empty, an override-free Definition equivalent to Reset. It refuses to
// run when HEAD is already readable (callers should use Restore/Reset for
// that case) or when there is no override at all yet (the HEAD file itself
// does not exist — nothing to recover). In both refusal cases it returns an
// error and writes nothing. See headStatusLocked for the shared rule this
// uses with NeedsRecovery.
func (s *HistoryStore) RecoverHead(providerID, revision string) (RevisionRecord, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	status, err := s.headStatusLocked(providerID)
	if err != nil {
		return RevisionRecord{}, err
	}
	switch status {
	case headMissing:
		return RevisionRecord{}, errors.New("override does not exist; nothing to recover")
	case headHealthy:
		return RevisionRecord{}, errors.New("override HEAD is readable; use Restore")
	}

	headPath := filepath.Join(s.root, providerID, "HEAD")
	if _, err := QuarantineFile(headPath, filepath.Join(s.backupRoot, "quarantine"), "recover head"); err != nil {
		return RevisionRecord{}, fmt.Errorf("quarantine corrupt HEAD: %w", err)
	}

	payload := Definition{ID: providerID}
	if revision != "" {
		selected, err := s.readRevisionLocked(providerID, revision)
		if err != nil {
			return RevisionRecord{}, err
		}
		payload = selected.Payload
	}

	return s.writeRevisionLocked(providerID, payload, "", "restore")
}
