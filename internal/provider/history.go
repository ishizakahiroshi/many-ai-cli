package provider

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"syscall"
	"time"
)

const historySchemaVersion = 1

// ErrRevisionConflict is the sentinel a caller checks with errors.Is to tell a
// stale optimistic-lock write apart from every other validation failure.
// Callers on the API boundary map it to HTTP 409; everything else stays a
// generic failure. Before this existed, the Hub handlers had no way to
// distinguish "you were editing an old copy" from "the payload is invalid",
// and collapsed the built-in provider's very first override (parent == "")
// into the same conflict as a stale one.
var ErrRevisionConflict = errors.New("provider revision conflict")

type RevisionRecord struct {
	SchemaVersion  int        `json:"schema_version"`
	ProviderID     string     `json:"provider_id"`
	Revision       string     `json:"revision"`
	CreatedAt      string     `json:"created_at"`
	Reason         string     `json:"reason"`
	ParentRevision string     `json:"parent_revision,omitempty"`
	ContentDigest  string     `json:"content_digest"`
	Payload        Definition `json:"payload"`
}

type HistoryStore struct {
	mu         sync.Mutex
	root       string
	backupRoot string
}

func NewHistoryStore(root, backupRoot string) (*HistoryStore, error) {
	if strings.TrimSpace(root) == "" || strings.TrimSpace(backupRoot) == "" {
		return nil, fmt.Errorf("history and backup roots are required")
	}
	return &HistoryStore{root: filepath.Clean(root), backupRoot: filepath.Clean(backupRoot)}, nil
}

// SaveOverride merges payload onto whatever is currently overridden for
// providerID (if anything) and saves the result as a new revision, instead
// of replacing the previous override wholesale. Without this, saving any
// override — even one that only sets a single field, like the enabled
// toggle DELETE uses to soft-disable a built-in provider — discarded every
// other field a prior edit had customized, and froze the *entire* resolved
// definition (including fields the caller never touched) into the override
// forever, so it stopped tracking updates to the embedded/distribution base
// even for fields nobody had asked to override.
//
// baseline is what this provider resolves to without any override (the
// caller's embedded/distribution/user layers) — never written to disk,
// used only so a payload that is genuinely partial (e.g. {enabled: false}
// alone, with no history yet to merge onto) can still be validated as a
// complete, schema-valid Definition once merged with something. Passing
// Definition{} is fine when payload is already self-sufficient (has its own
// launch.executable, as every existing caller's full-snapshot payload does).
func (s *HistoryStore) SaveOverride(providerID string, payload, baseline Definition, expectedRevision, reason string) (RevisionRecord, error) {
	if s == nil {
		return RevisionRecord{}, fmt.Errorf("history store is nil")
	}
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return RevisionRecord{}, err
	}
	if payload.ID == "" {
		payload.ID = providerID
	}
	if payload.ID != providerID {
		return RevisionRecord{}, fmt.Errorf("payload provider id does not match %q", providerID)
	}
	if reason == "" {
		reason = "edit"
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	merged, err := s.mergeWithCurrentOverrideLocked(providerID, payload)
	if err != nil {
		return RevisionRecord{}, err
	}
	if err := validateOverrideAgainstBaseline(merged, baseline); err != nil {
		return RevisionRecord{}, err
	}
	return s.saveLocked(providerID, merged, expectedRevision, reason)
}

// SaveEffectiveOverride accepts the complete effective definition a UI edits
// and stores only the fields that differ from the provider's current base.
// Keeping this conversion at the service boundary prevents callers that send
// a full form snapshot from freezing every distribution field into the local
// override forever.
func (s *HistoryStore) SaveEffectiveOverride(providerID string, desired, baseline Definition, expectedRevision, reason string) (RevisionRecord, error) {
	if s == nil {
		return RevisionRecord{}, fmt.Errorf("history store is nil")
	}
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return RevisionRecord{}, err
	}
	if desired.ID == "" {
		desired.ID = providerID
	}
	if desired.ID != providerID {
		return RevisionRecord{}, fmt.Errorf("payload provider id does not match %q", providerID)
	}
	if err := validateOverrideAgainstBaseline(desired, Definition{}); err != nil {
		return RevisionRecord{}, err
	}
	delta, err := definitionOverrideDelta(baseline, desired)
	if err != nil {
		return RevisionRecord{}, err
	}
	delta.ID = providerID
	if reason == "" {
		reason = "edit"
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := validateOverrideAgainstBaseline(delta, baseline); err != nil {
		return RevisionRecord{}, err
	}
	return s.saveLocked(providerID, delta, expectedRevision, reason)
}

func definitionOverrideDelta(baseline, desired Definition) (Definition, error) {
	baseRaw, err := json.Marshal(baseline)
	if err != nil {
		return Definition{}, err
	}
	desiredRaw, err := json.Marshal(desired)
	if err != nil {
		return Definition{}, err
	}
	var baseMap, desiredMap map[string]json.RawMessage
	if err := json.Unmarshal(baseRaw, &baseMap); err != nil {
		return Definition{}, err
	}
	if err := json.Unmarshal(desiredRaw, &desiredMap); err != nil {
		return Definition{}, err
	}
	delete(baseMap, "source")
	delete(desiredMap, "source")
	deltaMap := diffDefinitionObject(baseMap, desiredMap)
	deltaMap["id"] = json.RawMessage(fmt.Sprintf("%q", desired.ID))
	var delta Definition
	if err := json.Unmarshal(marshalRawObject(deltaMap), &delta); err != nil {
		return Definition{}, err
	}
	return delta, nil
}

func diffDefinitionObject(base, desired map[string]json.RawMessage) map[string]json.RawMessage {
	out := make(map[string]json.RawMessage)
	for key, desiredValue := range desired {
		if key == "id" {
			continue
		}
		baseValue, exists := base[key]
		if exists && isJSONObject(baseValue) && isJSONObject(desiredValue) {
			var baseObject, desiredObject map[string]json.RawMessage
			_ = json.Unmarshal(baseValue, &baseObject)
			_ = json.Unmarshal(desiredValue, &desiredObject)
			nested := diffDefinitionObject(baseObject, desiredObject)
			if len(nested) > 0 {
				out[key] = marshalRawObject(nested)
			}
			continue
		}
		if !exists || string(baseValue) != string(desiredValue) {
			out[key] = append(json.RawMessage(nil), desiredValue...)
		}
	}
	return out
}

// mergeWithCurrentOverrideLocked applies payload on top of the current
// override's payload (if any) using the exact same field-merge rule the
// Registry itself uses to layer embedded/distribution/user/override
// definitions (mergeDefinition in merge.go): a key payload does not set is
// left as whatever the current override already had, and nested objects
// (launch, models, adapters, ...) merge per sub-field rather than being
// replaced wholesale.
func (s *HistoryStore) mergeWithCurrentOverrideLocked(providerID string, payload Definition) (Definition, error) {
	current, err := s.currentLocked(providerID)
	if err != nil {
		if os.IsNotExist(err) {
			return payload, nil
		}
		return Definition{}, err
	}
	return mergeDefinitionValues(current.Payload, payload)
}

// validateOverrideAgainstBaseline checks that payload merged onto baseline
// (never onto payload alone) is a complete, schema-valid Definition. This
// is deliberately not "validate payload in isolation": a sparse override
// like {enabled: false} has no launch.executable of its own, and never
// will — completeness only exists once it is combined with whatever
// embedded/distribution layer actually supplies the base defaults, which is
// also how the Registry itself validates the final merged result.
func validateOverrideAgainstBaseline(payload, baseline Definition) error {
	effective, err := mergeDefinitionValues(baseline, payload)
	if err != nil {
		return err
	}
	raw, err := json.Marshal(effective)
	if err != nil {
		return fmt.Errorf("encode override payload: %w", err)
	}
	diagnostics, err := ValidateDefinition(raw, DefaultAdapterCatalog())
	if err != nil {
		return err
	}
	if hasDiagnosticError(diagnostics) {
		return fmt.Errorf("override payload is invalid")
	}
	return nil
}

func mergeDefinitionValues(base, overlay Definition) (Definition, error) {
	baseRaw, err := json.Marshal(base)
	if err != nil {
		return Definition{}, fmt.Errorf("encode override base: %w", err)
	}
	overlayRaw, err := json.Marshal(overlay)
	if err != nil {
		return Definition{}, fmt.Errorf("encode override payload: %w", err)
	}
	var baseMap, overlayMap map[string]json.RawMessage
	if err := json.Unmarshal(baseRaw, &baseMap); err != nil {
		return Definition{}, err
	}
	if err := json.Unmarshal(overlayRaw, &overlayMap); err != nil {
		return Definition{}, err
	}
	var merged Definition
	if err := json.Unmarshal(marshalRawObject(mergeDefinition(baseMap, overlayMap)), &merged); err != nil {
		return Definition{}, err
	}
	return merged, nil
}

func (s *HistoryStore) saveLocked(providerID string, payload Definition, expectedRevision, reason string) (RevisionRecord, error) {
	if err := ensureInsideRoot(s.root, filepath.Join(s.root, providerID)); err != nil {
		return RevisionRecord{}, err
	}
	if err := ensureInsideRoot(s.backupRoot, filepath.Join(s.backupRoot, providerID)); err != nil && !os.IsNotExist(err) {
		return RevisionRecord{}, err
	}
	current, currentErr := s.currentLocked(providerID)
	if currentErr != nil && !os.IsNotExist(currentErr) {
		return RevisionRecord{}, currentErr
	}
	parent := ""
	if currentErr == nil {
		parent = current.Revision
	}
	if expectedRevision != parent {
		return RevisionRecord{}, fmt.Errorf("%w: expected %q, current %q", ErrRevisionConflict, expectedRevision, parent)
	}
	if currentErr == nil {
		if err := s.writeBackupLocked(current); err != nil {
			return RevisionRecord{}, err
		}
	}
	return s.writeRevisionLocked(providerID, payload, parent, reason)
}

// writeRevisionLocked writes payload as a new immutable revision and moves
// HEAD to point at it. It is the tail half of saveLocked (everything after
// the conflict check and pre-write backup), factored out so RecoverHead can
// reuse the exact same write-and-verify-and-flip-HEAD sequence without going
// through saveLocked's currentLocked/expectedRevision conflict check — that
// check requires HEAD to already be readable, which is precisely what does
// not hold when recovering from a corrupt HEAD. Callers must not change this
// sequence's behavior (backup is the caller's responsibility, same as
// before).
func (s *HistoryStore) writeRevisionLocked(providerID string, payload Definition, parent, reason string) (RevisionRecord, error) {
	createdAt := time.Now().UTC().Format(time.RFC3339Nano)
	digest := definitionDigest(payload)
	revision := revisionID(providerID, parent, digest, createdAt)
	record := RevisionRecord{
		SchemaVersion:  historySchemaVersion,
		ProviderID:     providerID,
		Revision:       revision,
		CreatedAt:      createdAt,
		Reason:         reason,
		ParentRevision: parent,
		ContentDigest:  digest,
		Payload:        payload,
	}
	revisionDir := filepath.Join(s.root, providerID, "revisions")
	if err := os.MkdirAll(revisionDir, 0o700); err != nil {
		return RevisionRecord{}, fmt.Errorf("create revision directory: %w", err)
	}
	if err := ensureInsideRoot(s.root, revisionDir); err != nil {
		return RevisionRecord{}, err
	}
	revisionPath := filepath.Join(revisionDir, revision+".json")
	if err := ensureInsideRoot(s.root, revisionPath); err != nil && !os.IsNotExist(err) {
		return RevisionRecord{}, err
	}
	if err := writeJSONAtomic(revisionPath, record); err != nil {
		return RevisionRecord{}, err
	}
	verified, err := readRevision(revisionPath, providerID)
	if err != nil || verified.ContentDigest != digest {
		return RevisionRecord{}, fmt.Errorf("verify revision %s: %w", revision, err)
	}
	headPath := filepath.Join(s.root, providerID, "HEAD")
	if err := ensureInsideRoot(s.root, headPath); err != nil && !os.IsNotExist(err) {
		return RevisionRecord{}, err
	}
	if err := writeTextAtomic(headPath, revision); err != nil {
		return RevisionRecord{}, err
	}
	return record, nil
}

func (s *HistoryStore) Restore(providerID, revision, expectedRevision string) (RevisionRecord, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	selected, err := s.readRevisionLocked(providerID, revision)
	if err != nil {
		return RevisionRecord{}, err
	}
	return s.saveLocked(providerID, selected.Payload, expectedRevision, "restore")
}

func (s *HistoryStore) Reset(providerID, expectedRevision string) (RevisionRecord, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.saveLocked(providerID, Definition{ID: providerID}, expectedRevision, "reset")
}

func (s *HistoryStore) Current(providerID string) (RevisionRecord, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.currentLocked(providerID)
}

func (s *HistoryStore) GetRevision(providerID, revision string) (RevisionRecord, error) {
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return RevisionRecord{}, err
	}
	if revision == "" || filepath.Base(revision) != revision {
		return RevisionRecord{}, fmt.Errorf("revision is invalid")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.readRevisionLocked(providerID, revision)
}

// BackupSnapshot records a verified point-in-time definition without making
// it the active override. Custom-provider CRUD uses this before destructive
// FileStore mutations, while built-in overrides continue to use saveLocked's
// revision/HEAD protocol.
func (s *HistoryStore) BackupSnapshot(providerID string, payload Definition, reason string) (RevisionRecord, error) {
	if s == nil {
		return RevisionRecord{}, fmt.Errorf("history store is nil")
	}
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return RevisionRecord{}, err
	}
	if payload.ID != providerID {
		return RevisionRecord{}, fmt.Errorf("payload provider id does not match %q", providerID)
	}
	payload.Source = SourceRef{}
	if err := validateOverrideAgainstBaseline(payload, Definition{}); err != nil {
		return RevisionRecord{}, err
	}
	if reason == "" {
		reason = "snapshot"
	}
	createdAt := time.Now().UTC().Format(time.RFC3339Nano)
	digest := definitionDigest(payload)
	record := RevisionRecord{
		SchemaVersion: historySchemaVersion,
		ProviderID:    providerID,
		Revision:      revisionID(providerID, "", digest, createdAt),
		CreatedAt:     createdAt,
		Reason:        reason,
		ContentDigest: digest,
		Payload:       payload,
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := s.writeBackupLocked(record); err != nil {
		return RevisionRecord{}, err
	}
	return record, nil
}

func (s *HistoryStore) List(providerID string) ([]RevisionRecord, error) {
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	entries, err := os.ReadDir(filepath.Join(s.root, providerID, "revisions"))
	if os.IsNotExist(err) {
		return nil, nil
	}
	if err != nil {
		return nil, fmt.Errorf("list revisions: %w", err)
	}
	paths := make([]string, 0, len(entries))
	for _, entry := range entries {
		if !entry.IsDir() && filepath.Ext(entry.Name()) == ".json" {
			paths = append(paths, entry.Name())
		}
	}
	sort.Strings(paths)
	result := make([]RevisionRecord, 0, len(paths))
	for _, name := range paths {
		path := filepath.Join(s.root, providerID, "revisions", name)
		if err := ensureInsideRoot(s.root, path); err != nil {
			return nil, err
		}
		record, readErr := readRevision(path, providerID)
		if readErr != nil {
			return nil, readErr
		}
		result = append(result, record)
	}
	return result, nil
}

func (s *HistoryStore) ListBackups(providerID string) ([]RevisionRecord, error) {
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	entries, err := os.ReadDir(filepath.Join(s.backupRoot, providerID))
	if os.IsNotExist(err) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	names := make([]string, 0, len(entries))
	for _, entry := range entries {
		if !entry.IsDir() && filepath.Ext(entry.Name()) == ".json" {
			names = append(names, entry.Name())
		}
	}
	sort.Strings(names)
	result := make([]RevisionRecord, 0, len(names))
	for _, name := range names {
		path := filepath.Join(s.backupRoot, providerID, name)
		if err := ensureInsideRoot(s.backupRoot, path); err != nil {
			return nil, err
		}
		record, readErr := readRevision(path, providerID)
		if readErr != nil {
			return nil, readErr
		}
		result = append(result, record)
	}
	return result, nil
}

func (s *HistoryStore) VerifyBackup(providerID, backupID string) (RevisionRecord, error) {
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return RevisionRecord{}, err
	}
	name, err := backupFileName(backupID)
	if err != nil {
		return RevisionRecord{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	path := filepath.Join(s.backupRoot, providerID, name)
	if err := ensureInsideRoot(s.backupRoot, path); err != nil {
		return RevisionRecord{}, err
	}
	return readRevision(path, providerID)
}

// backupFileName は backupID をファイル名へ正規化する唯一の口で、**検査もここで行う。**
// 以前は VerifyBackup と RestoreBackup が同じ正規化を書き写していて、検査が付いて
// いたのは VerifyBackup だけだった（RestoreBackup は backupID を素通しで Join して
// いた）。正規化と検査を 1 本にまとめて、次の呼び出し口が同じ穴を空けられなくする。
func backupFileName(backupID string) (string, error) {
	if filepath.Base(backupID) != backupID || strings.TrimSuffix(backupID, ".json") == "" {
		return "", fmt.Errorf("invalid backup id")
	}
	if filepath.Ext(backupID) == "" {
		return backupID + ".json", nil
	}
	return backupID, nil
}

func (s *HistoryStore) RestoreBackup(providerID, backupID, expectedRevision string) (RevisionRecord, error) {
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return RevisionRecord{}, err
	}
	name, err := backupFileName(backupID)
	if err != nil {
		return RevisionRecord{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	path := filepath.Join(s.backupRoot, providerID, name)
	if err := ensureInsideRoot(s.backupRoot, path); err != nil {
		return RevisionRecord{}, err
	}
	backup, err := readRevision(path, providerID)
	if err != nil {
		return RevisionRecord{}, err
	}
	return s.saveLocked(providerID, backup.Payload, expectedRevision, "restore")
}

func (s *HistoryStore) LoadOverrides() ([]Definition, []Diagnostic, error) {
	if s == nil {
		return nil, nil, fmt.Errorf("history store is nil")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	entries, err := os.ReadDir(s.root)
	if os.IsNotExist(err) {
		return nil, nil, nil
	}
	if err != nil {
		return nil, nil, fmt.Errorf("read override store: %w", err)
	}
	var definitions []Definition
	var diagnostics []Diagnostic
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		id := entry.Name()
		if err := ValidateHistoryProviderID(id); err != nil {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_override_id", Severity: SeverityError, Field: id, Message: "invalid override directory"})
			continue
		}
		if err := ensureInsideRoot(s.root, filepath.Join(s.root, id)); err != nil {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_override_path", Severity: SeverityError, Field: id, Message: "override directory is outside the store root"})
			continue
		}
		record, readErr := s.currentLocked(id)
		if readErr != nil {
			_, _ = QuarantineFile(filepath.Join(s.root, id, "HEAD"), filepath.Join(s.backupRoot, "quarantine"), "invalid override HEAD")
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_override", Severity: SeverityError, Field: id, Message: "override HEAD could not be read"})
			continue
		}
		record.Payload.Source = SourceRef{Origin: OriginOverride, Revision: record.Revision, Digest: record.ContentDigest}
		definitions = append(definitions, record.Payload)
	}
	return definitions, diagnostics, nil
}

func (s *HistoryStore) currentLocked(providerID string) (RevisionRecord, error) {
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return RevisionRecord{}, err
	}
	headPath := filepath.Join(s.root, providerID, "HEAD")
	if err := ensureInsideRoot(s.root, headPath); err != nil {
		return RevisionRecord{}, err
	}
	recoverInterruptedAtomicReplace(headPath)
	head, err := os.ReadFile(headPath)
	if err != nil {
		return RevisionRecord{}, err
	}
	return s.readRevisionLocked(providerID, strings.TrimSpace(string(head)))
}

// readRevisionLocked はパスを providerID と revision から組み立てる唯一の読み取り口。
// **両方をここで検査する。** revision だけを見ていた頃、Restore() は providerID を
// 検査せずにこの関数を呼んでいたため、"../.." を含む providerID が root の外を指せた。
// 公開関数の側だけで検査すると、新しい呼び出し口が増えたときに同じ穴が空く。
func (s *HistoryStore) readRevisionLocked(providerID, revision string) (RevisionRecord, error) {
	if err := ValidateHistoryProviderID(providerID); err != nil {
		return RevisionRecord{}, err
	}
	if revision == "" || filepath.Base(revision) != revision {
		return RevisionRecord{}, fmt.Errorf("invalid revision")
	}
	path := filepath.Join(s.root, providerID, "revisions", revision+".json")
	if err := ensureInsideRoot(s.root, path); err != nil {
		return RevisionRecord{}, err
	}
	return readRevision(path, providerID)
}

// writeBackupLocked の record.ProviderID は **ディスク上の revision ファイルの中身**
// から来る（saveLocked は currentLocked が読み戻したレコードを渡す）。引数として
// 受け取った providerID とは別物なので、書き込み先を組み立てる前にここでも検査する。
// 検査が無いと、細工された revision ファイル 1 個で backupRoot の外へ書ける。
func (s *HistoryStore) writeBackupLocked(record RevisionRecord) error {
	if err := ValidateHistoryProviderID(record.ProviderID); err != nil {
		return fmt.Errorf("backup provider id: %w", err)
	}
	if record.Revision == "" || filepath.Base(record.Revision) != record.Revision {
		return fmt.Errorf("backup revision is invalid")
	}
	path := filepath.Join(s.backupRoot, record.ProviderID, record.Revision+".json")
	if err := ensureInsideRoot(s.backupRoot, filepath.Join(s.backupRoot, record.ProviderID)); err != nil {
		return err
	}
	if err := ensureInsideRoot(s.backupRoot, path); err != nil && !os.IsNotExist(err) {
		return err
	}
	if err := writeJSONAtomic(path, record); err != nil {
		return fmt.Errorf("write provider backup: %w", err)
	}
	verified, err := readRevision(path, record.ProviderID)
	if err != nil || verified.ContentDigest != record.ContentDigest {
		return fmt.Errorf("verify provider backup: %w", err)
	}
	return nil
}

// readRevision は revision ファイルを読む唯一の関数。呼び出し口は 5 つあるが、
// path はいずれも検査済みの部品だけで組み立てられている（providerID は
// ValidateHistoryProviderID、revision と backupID は filepath.Base 一致、
// 一覧経路の name は os.ReadDir が返したエントリ名）。
// gosec の taint 解析は正規表現ベースの検証関数を sanitizer と認識できないため、
// 検証を足しても G703 が残る。`internal/hub/approval_patterns.go` の同種の
// 注記と同じ形で、検証している場所を名指しして抑止する。
//
// expectedProviderID は呼び出し口が「このディレクトリ／この provider 向けの
// はず」として持っている provider ID で、読み込んだレコードの ProviderID・
// Payload.ID の両方と突き合わせる。ContentDigest は record.Payload 自身との
// 整合性しか見ないため、provider A のディレクトリから読み込んだ自己整合的な
// revision ファイルをそのまま provider B のディレクトリへコピーされても、
// digest チェックだけでは検出できない。呼び出し口はどれもディレクトリを
// providerID から組み立てているので、この不一致は「別 provider の
// metadata / payload が混入した revision」の唯一の検出点になる。
// 同様に record.Revision もファイル名（拡張子を除いた部分）と突き合わせる。
// ContentDigest は Payload の整合性しか見ないため、ファイル名と食い違う
// Revision を record 内に書いても検出できない別経路になる。
func readRevision(path, expectedProviderID string) (RevisionRecord, error) {
	recoverInterruptedAtomicReplace(path)
	raw, err := os.ReadFile(path) // #nosec G703 -- ValidateHistoryProviderID + filepath.Base 検査 + backupFileName を通った部品のみで組み立てたパス
	if err != nil {
		return RevisionRecord{}, err
	}
	var record RevisionRecord
	if err := json.Unmarshal(raw, &record); err != nil {
		return RevisionRecord{}, fmt.Errorf("decode revision: %w", err)
	}
	if record.SchemaVersion != historySchemaVersion || record.Revision == "" || record.ContentDigest == "" {
		return RevisionRecord{}, fmt.Errorf("invalid revision metadata")
	}
	if expectedRevision := strings.TrimSuffix(filepath.Base(path), ".json"); record.Revision != expectedRevision {
		return RevisionRecord{}, fmt.Errorf("revision id mismatch: filename implies %q, record has %q", expectedRevision, record.Revision)
	}
	if record.ProviderID != expectedProviderID {
		return RevisionRecord{}, fmt.Errorf("revision provider id mismatch: expected %q, found %q", expectedProviderID, record.ProviderID)
	}
	if record.Payload.ID != expectedProviderID {
		return RevisionRecord{}, fmt.Errorf("revision payload id mismatch: expected %q, found %q", expectedProviderID, record.Payload.ID)
	}
	if definitionDigest(record.Payload) != record.ContentDigest {
		return RevisionRecord{}, fmt.Errorf("revision content digest mismatch")
	}
	// A stored override is a sparse delta against a baseline this function
	// cannot see, so the placeholder base must supply every field whose
	// presence another field's rule depends on. Otherwise a legitimate delta
	// such as {update:{enabled:true}} (args come from the manifest) or
	// {launch:{headless:{args:[...]}}} (format comes from the manifest) fails
	// here right after being written, and the save returns 422 with HEAD
	// never moved. TestSaveEffectiveOverrideAcceptsEverySingleFieldEdit
	// sweeps every built-in manifest field to catch the next such rule.
	validationBase := Definition{
		SchemaVersion: CurrentSchemaVersion,
		ID:            expectedProviderID,
		DisplayName:   expectedProviderID,
		Launch: &LaunchDefinition{
			Executable: "validation-placeholder",
			Headless:   &HeadlessDefinition{Format: "validation-placeholder"},
		},
		Update: &UpdateDefinition{Args: []string{"validation-placeholder"}},
	}
	if err := validateOverrideAgainstBaseline(record.Payload, validationBase); err != nil {
		return RevisionRecord{}, fmt.Errorf("revision payload schema is invalid: %w", err)
	}
	return record, nil
}

func definitionDigest(definition Definition) string {
	raw, _ := json.Marshal(definition)
	sum := sha256.Sum256(raw)
	return hex.EncodeToString(sum[:])
}

func revisionID(providerID, parent, digest, createdAt string) string {
	sum := sha256.Sum256([]byte(providerID + "\n" + parent + "\n" + digest + "\n" + createdAt))
	return hex.EncodeToString(sum[:])[:24]
}

func ValidateHistoryProviderID(id string) error {
	if err := validateID(id); err != nil {
		return err
	}
	if id == "shell" {
		return fmt.Errorf("shell is reserved")
	}
	return nil
}

func ensureInsideRoot(root, path string) error {
	absRoot, err := filepath.Abs(root)
	if err != nil {
		return err
	}
	absPath, err := filepath.Abs(path)
	if err != nil {
		return err
	}
	info, err := os.Lstat(absPath)
	// 途中の要素がディレクトリでない（ファイル）ときも「まだ存在しない」と同じく親へ遡る。
	// Windows はこれを ERROR_PATH_NOT_FOUND（IsNotExist）で返すが、POSIX は ENOTDIR を返すので、
	// 揃えないと同じ構成が Windows では通り Linux / macOS では lstat エラーで止まる。
	// その位置には何も作れないので、root の外へ抜ける経路にはならない。
	if os.IsNotExist(err) || errors.Is(err, syscall.ENOTDIR) {
		if filepath.Clean(absPath) == filepath.Clean(absRoot) {
			return nil
		}
		parent := filepath.Dir(absPath)
		if parent == absPath {
			return fmt.Errorf("path escapes store root")
		}
		return ensureInsideRoot(root, parent)
	}
	if err != nil {
		return err
	}
	if info.Mode()&os.ModeSymlink != 0 {
		return fmt.Errorf("store path must not be a symlink")
	}
	resolvedRoot, err := filepath.EvalSymlinks(absRoot)
	if err != nil {
		if os.IsNotExist(err) {
			resolvedRoot = absRoot
		} else {
			return err
		}
	}
	resolvedPath, err := filepath.EvalSymlinks(absPath)
	if err != nil {
		return err
	}
	rel, err := filepath.Rel(filepath.Clean(resolvedRoot), filepath.Clean(resolvedPath))
	if err != nil {
		return err
	}
	if rel == ".." || strings.HasPrefix(rel, ".."+string(os.PathSeparator)) {
		return fmt.Errorf("path escapes store root")
	}
	return nil
}
