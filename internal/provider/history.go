package provider

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"
)

const historySchemaVersion = 1

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

func (s *HistoryStore) SaveOverride(providerID string, payload Definition, expectedRevision, reason string) (RevisionRecord, error) {
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
	raw, err := json.Marshal(payload)
	if err != nil {
		return RevisionRecord{}, fmt.Errorf("encode override payload: %w", err)
	}
	diagnostics, err := ValidateDefinition(raw, DefaultAdapterCatalog())
	if err != nil {
		return RevisionRecord{}, err
	}
	if hasDiagnosticError(diagnostics) {
		return RevisionRecord{}, fmt.Errorf("override payload is invalid")
	}
	if reason == "" {
		reason = "edit"
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.saveLocked(providerID, payload, expectedRevision, reason)
}

func (s *HistoryStore) saveLocked(providerID string, payload Definition, expectedRevision, reason string) (RevisionRecord, error) {
	current, currentErr := s.currentLocked(providerID)
	if currentErr != nil && !os.IsNotExist(currentErr) {
		return RevisionRecord{}, currentErr
	}
	parent := ""
	if currentErr == nil {
		parent = current.Revision
	}
	if expectedRevision != "" && expectedRevision != parent {
		return RevisionRecord{}, fmt.Errorf("revision conflict: expected %s, current %s", expectedRevision, parent)
	}
	if currentErr == nil {
		if err := s.writeBackupLocked(current); err != nil {
			return RevisionRecord{}, err
		}
	}
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
	revisionPath := filepath.Join(revisionDir, revision+".json")
	if err := writeJSONAtomic(revisionPath, record); err != nil {
		return RevisionRecord{}, err
	}
	verified, err := readRevision(revisionPath)
	if err != nil || verified.ContentDigest != digest {
		return RevisionRecord{}, fmt.Errorf("verify revision %s: %w", revision, err)
	}
	if err := writeTextAtomic(filepath.Join(s.root, providerID, "HEAD"), revision); err != nil {
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
	return s.saveLocked(providerID, Definition{SchemaVersion: CurrentSchemaVersion, ID: providerID, DisplayName: providerID}, expectedRevision, "reset")
}

func (s *HistoryStore) Current(providerID string) (RevisionRecord, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.currentLocked(providerID)
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
		record, readErr := readRevision(filepath.Join(s.root, providerID, "revisions", name))
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
		record, readErr := readRevision(filepath.Join(s.backupRoot, providerID, name))
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
	return readRevision(filepath.Join(s.backupRoot, providerID, name))
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
	backup, err := readRevision(filepath.Join(s.backupRoot, providerID, name))
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
	head, err := os.ReadFile(filepath.Join(s.root, providerID, "HEAD"))
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
	return readRevision(filepath.Join(s.root, providerID, "revisions", revision+".json"))
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
	if err := writeJSONAtomic(path, record); err != nil {
		return fmt.Errorf("write provider backup: %w", err)
	}
	verified, err := readRevision(path)
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
func readRevision(path string) (RevisionRecord, error) {
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
	if definitionDigest(record.Payload) != record.ContentDigest {
		return RevisionRecord{}, fmt.Errorf("revision content digest mismatch")
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
