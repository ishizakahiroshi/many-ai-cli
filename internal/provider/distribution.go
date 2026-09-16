package provider

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

// ErrNoAcceptedDistribution is the sentinel LoadAcceptedBundle returns when
// nothing has ever been accepted — an expected, normal state a caller
// building a registry layer should treat as "empty layer, no diagnostic".
// Every other LoadAcceptedBundle failure (corrupt pointer, tampered bundle,
// schema mismatch) is a real problem that must not collapse into the same
// "as if nothing were accepted" outcome, so callers check this specific
// sentinel rather than treating any error the same way.
var ErrNoAcceptedDistribution = errors.New("no accepted distribution")

const MaxDistributionBundleBytes = 2 * 1024 * 1024

const OfficialDistributionHost = "raw.githubusercontent.com"

type DistributionPayload struct {
	SchemaVersion     int               `json:"schema_version"`
	CatalogVersion    string            `json:"catalog_version"`
	CreatedAt         string            `json:"created_at"`
	MinimumAppVersion string            `json:"minimum_app_version,omitempty"`
	Definitions       []Definition      `json:"definitions"`
	Digests           map[string]string `json:"digests"`
}

type DistributionBundle struct {
	Payload   DistributionPayload `json:"payload"`
	KeyID     string              `json:"key_id"`
	Signature string              `json:"signature"`
}

type DistributionStatus struct {
	CatalogVersion string `json:"catalog_version"`
	Digest         string `json:"digest"`
	State          string `json:"state"`
	Path           string `json:"path,omitempty"`
}

func FetchDistribution(ctx context.Context, rawURL string, client *http.Client) ([]byte, error) {
	u, err := url.Parse(rawURL)
	if err != nil || u.Scheme != "https" || !strings.EqualFold(u.Hostname(), OfficialDistributionHost) {
		return nil, fmt.Errorf("distribution URL is not an allowed HTTPS endpoint")
	}
	if ctx == nil {
		ctx = context.Background()
	}
	if client == nil {
		client = &http.Client{Timeout: 20 * time.Second}
	}
	clone := *client
	previousRedirect := clone.CheckRedirect
	clone.CheckRedirect = func(req *http.Request, history []*http.Request) error {
		if req.URL.Scheme != "https" || !strings.EqualFold(req.URL.Hostname(), OfficialDistributionHost) {
			return fmt.Errorf("distribution redirect target is not allowed")
		}
		if previousRedirect != nil {
			return previousRedirect(req, history)
		}
		return nil
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
	if err != nil {
		return nil, err
	}
	response, err := clone.Do(request)
	if err != nil {
		return nil, err
	}
	defer response.Body.Close()
	if response.StatusCode/100 != 2 {
		return nil, fmt.Errorf("distribution endpoint returned HTTP %s", response.Status)
	}
	if contentType := response.Header.Get("Content-Type"); contentType != "" && !strings.Contains(strings.ToLower(contentType), "json") {
		return nil, fmt.Errorf("distribution endpoint returned unsupported content type")
	}
	data, err := io.ReadAll(io.LimitReader(response.Body, MaxDistributionBundleBytes+1))
	if err != nil {
		return nil, err
	}
	if len(data) > MaxDistributionBundleBytes {
		return nil, fmt.Errorf("distribution bundle exceeds %d bytes", MaxDistributionBundleBytes)
	}
	return data, nil
}

func VerifyDistributionBundle(raw []byte, trustedKeys map[string]ed25519.PublicKey, currentAppVersion string) (DistributionBundle, string, error) {
	if len(raw) > MaxDistributionBundleBytes {
		return DistributionBundle{}, "", fmt.Errorf("distribution bundle exceeds %d bytes", MaxDistributionBundleBytes)
	}
	var bundle DistributionBundle
	if err := decodeSingleJSON(raw, &bundle); err != nil {
		return DistributionBundle{}, "", fmt.Errorf("decode distribution bundle: %w", err)
	}
	if bundle.Payload.SchemaVersion != CurrentSchemaVersion || strings.TrimSpace(bundle.Payload.CatalogVersion) == "" {
		return DistributionBundle{}, "", fmt.Errorf("unsupported distribution payload")
	}
	key, ok := trustedKeys[bundle.KeyID]
	if !ok || len(key) != ed25519.PublicKeySize {
		return DistributionBundle{}, "", fmt.Errorf("unknown distribution signing key")
	}
	signature, err := base64.StdEncoding.DecodeString(bundle.Signature)
	if err != nil || len(signature) != ed25519.SignatureSize {
		return DistributionBundle{}, "", fmt.Errorf("invalid distribution signature encoding")
	}
	canonical, err := canonicalDistributionPayload(bundle.Payload)
	if err != nil {
		return DistributionBundle{}, "", err
	}
	if !ed25519.Verify(key, canonical, signature) {
		return DistributionBundle{}, "", fmt.Errorf("distribution signature verification failed")
	}
	if currentAppVersion != "" && bundle.Payload.MinimumAppVersion != "" && compareVersions(currentAppVersion, bundle.Payload.MinimumAppVersion) < 0 {
		return DistributionBundle{}, "", fmt.Errorf("distribution requires app version %s", bundle.Payload.MinimumAppVersion)
	}
	if len(bundle.Payload.Definitions) > MaxDefinitionItems {
		return DistributionBundle{}, "", fmt.Errorf("distribution contains too many definitions")
	}
	seen := make(map[string]struct{}, len(bundle.Payload.Definitions))
	for _, definition := range bundle.Payload.Definitions {
		if _, exists := seen[definition.ID]; exists {
			return DistributionBundle{}, "", fmt.Errorf("distribution contains duplicate provider %q", definition.ID)
		}
		seen[definition.ID] = struct{}{}
		definitionRaw, marshalErr := json.Marshal(definition)
		if marshalErr != nil {
			return DistributionBundle{}, "", marshalErr
		}
		diagnostics, validateErr := ValidateDefinition(definitionRaw, DefaultAdapterCatalog())
		if validateErr != nil || hasDiagnosticError(diagnostics) {
			return DistributionBundle{}, "", fmt.Errorf("distribution definition %q is invalid", definition.ID)
		}
		expectedDigest := bundle.Payload.Digests[definition.ID]
		if expectedDigest == "" || expectedDigest != definitionDigest(definition) {
			return DistributionBundle{}, "", fmt.Errorf("distribution digest mismatch for %q", definition.ID)
		}
	}
	bundleDigest := sha256.Sum256(raw)
	return bundle, hex.EncodeToString(bundleDigest[:]), nil
}

func canonicalDistributionPayload(payload DistributionPayload) ([]byte, error) {
	definitions := append([]Definition(nil), payload.Definitions...)
	sort.Slice(definitions, func(i, j int) bool { return definitions[i].ID < definitions[j].ID })
	payload.Definitions = definitions
	return json.Marshal(payload)
}

func BuildDistributionPayload(definitions []Definition, catalogVersion, createdAt, minimumAppVersion string) DistributionPayload {
	ordered := append([]Definition(nil), definitions...)
	sort.Slice(ordered, func(i, j int) bool { return ordered[i].ID < ordered[j].ID })
	digests := make(map[string]string, len(ordered))
	for _, definition := range ordered {
		digests[definition.ID] = definitionDigest(definition)
	}
	return DistributionPayload{
		SchemaVersion:     CurrentSchemaVersion,
		CatalogVersion:    catalogVersion,
		CreatedAt:         createdAt,
		MinimumAppVersion: minimumAppVersion,
		Definitions:       ordered,
		Digests:           digests,
	}
}

func SignDistributionPayload(payload DistributionPayload, keyID string, privateKey ed25519.PrivateKey) (DistributionBundle, error) {
	canonical, err := canonicalDistributionPayload(payload)
	if err != nil {
		return DistributionBundle{}, err
	}
	signature := ed25519.Sign(privateKey, canonical)
	return DistributionBundle{Payload: payload, KeyID: keyID, Signature: base64.StdEncoding.EncodeToString(signature)}, nil
}

// distributionPathComponentPattern is the allowlist every filename fragment
// derived from distribution content (a digest, or read back out of a locally
// stored pointer file) must satisfy before it can be joined onto a path
// under the store root. A bundle is ed25519-signed, but a retired/leaked key
// or a bug in a future publishing pipeline would let an attacker choose the
// digest string arbitrarily, and a local pointer file (accepted.json,
// previous.json) is plain JSON on disk that this store must not simply trust
// either — so this check runs regardless of how "trusted" the source claims
// to be.
var distributionPathComponentPattern = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`)

// windowsReservedDeviceNames are the base names (before any extension)
// Windows treats as device names rather than ordinary files, even when
// running this store on a non-Windows OS: distribution bundles are the same
// files regardless of the platform they were produced on, and a bundle
// crafted to try "con.json" should be rejected everywhere, not only where it
// would misbehave.
var windowsReservedDeviceNames = map[string]struct{}{
	"con": {}, "prn": {}, "aux": {}, "nul": {},
	"com1": {}, "com2": {}, "com3": {}, "com4": {}, "com5": {}, "com6": {}, "com7": {}, "com8": {}, "com9": {},
	"lpt1": {}, "lpt2": {}, "lpt3": {}, "lpt4": {}, "lpt5": {}, "lpt6": {}, "lpt7": {}, "lpt8": {}, "lpt9": {},
}

func validateDistributionPathComponent(value string) error {
	if !distributionPathComponentPattern.MatchString(value) {
		return fmt.Errorf("invalid distribution path component %q", value)
	}
	if value == "." || value == ".." {
		return fmt.Errorf("invalid distribution path component %q", value)
	}
	base := strings.TrimSuffix(value, filepath.Ext(value))
	if _, reserved := windowsReservedDeviceNames[strings.ToLower(base)]; reserved {
		return fmt.Errorf("distribution path component %q is a reserved name", value)
	}
	return nil
}

func distributionContentDigest(raw []byte) string {
	sum := sha256.Sum256(raw)
	return hex.EncodeToString(sum[:])
}

// DistributionStore's containment model: every path it touches is built from
// either a fixed literal ("downloaded", "accepted", "accepted.json", ...) or
// a value that has passed validateDistributionPathComponent, and every
// directory it writes into or reads a validated-component file from is also
// checked with ensureInsideRoot (history.go), which follows symlinks via
// filepath.EvalSymlinks before checking containment. On Windows, NTFS
// junctions/reparse points that Go's os.Lstat does not report as
// ModeSymlink are a known gap in that check (same caveat as HistoryStore);
// this store does not attempt anything stronger than HistoryStore already
// does for that case.
type DistributionStore struct {
	mu   sync.Mutex
	root string
}

func NewDistributionStore(root string) (*DistributionStore, error) {
	if strings.TrimSpace(root) == "" {
		return nil, fmt.Errorf("distribution store root is required")
	}
	return &DistributionStore{root: filepath.Clean(root)}, nil
}

// SaveDownloaded persists verified bundle bytes under a digest-only filename
// so promoting one later (Accept) never needs anything other than the digest
// to find it again. It stores the exact bytes it was given — never a
// re-marshal of a decoded struct — because every later integrity check in
// this file (Accept, LoadAcceptedBundle, Rollback) works by re-hashing
// stored bytes and comparing to the digest a pointer file claims; a
// re-marshaled copy would not reproduce the original hash and every one of
// those checks would either false-fail or have to skip verification.
func (s *DistributionStore) SaveDownloaded(raw []byte, digest string) error {
	if s == nil {
		return fmt.Errorf("distribution store is nil")
	}
	if len(raw) > MaxDistributionBundleBytes {
		return fmt.Errorf("distribution bundle exceeds %d bytes", MaxDistributionBundleBytes)
	}
	if err := validateDistributionPathComponent(digest); err != nil {
		return err
	}
	if distributionContentDigest(raw) != digest {
		return fmt.Errorf("distribution digest does not match payload bytes")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	dir := filepath.Join(s.root, "downloaded")
	if err := ensureInsideRoot(s.root, dir); err != nil && !os.IsNotExist(err) {
		return err
	}
	return writeBytesAtomic(filepath.Join(dir, digest+".json"), raw)
}

// LoadDownloadedBundle reads a previously downloaded (not necessarily
// accepted) bundle by digest, for showing a candidate/current diff before
// the user decides whether to accept it. It re-verifies the content digest
// the same way LoadAcceptedBundle does, but does not check the signature —
// callers that need trust re-established (Accept) do that separately.
func (s *DistributionStore) LoadDownloadedBundle(digest string) (DistributionBundle, error) {
	if s == nil {
		return DistributionBundle{}, fmt.Errorf("distribution store is nil")
	}
	if err := validateDistributionPathComponent(digest); err != nil {
		return DistributionBundle{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	dir := filepath.Join(s.root, "downloaded")
	if err := ensureInsideRoot(s.root, dir); err != nil {
		return DistributionBundle{}, err
	}
	path := filepath.Join(dir, digest+".json")
	recoverInterruptedAtomicReplace(path)
	raw, err := os.ReadFile(path)
	if err != nil {
		return DistributionBundle{}, fmt.Errorf("read downloaded distribution: %w", err)
	}
	if distributionContentDigest(raw) != digest {
		return DistributionBundle{}, fmt.Errorf("downloaded distribution content digest mismatch")
	}
	var bundle DistributionBundle
	if err := json.Unmarshal(raw, &bundle); err != nil {
		return DistributionBundle{}, fmt.Errorf("decode downloaded distribution: %w", err)
	}
	return bundle, nil
}

// Accept promotes an already-downloaded bundle to the accepted pointer by
// digest. It re-reads the bundle from the "downloaded" cache and re-runs the
// full VerifyDistributionBundle check (schema, per-definition digest,
// signature, trusted key, minimum app version) rather than trusting a caller
// to hand it an already-verified value: the on-disk downloaded copy could
// have been tampered with after SaveDownloaded ran, and a signing key could
// have been retired between download and accept. Accept never mutates
// accepted.json / previous.json until every one of those checks — and the
// integrity read of the downloaded file itself — has already succeeded.
func (s *DistributionStore) Accept(digest string, trustedKeys map[string]ed25519.PublicKey, currentAppVersion string) (DistributionStatus, error) {
	if s == nil {
		return DistributionStatus{}, fmt.Errorf("distribution store is nil")
	}
	if err := validateDistributionPathComponent(digest); err != nil {
		return DistributionStatus{}, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	downloadedDir := filepath.Join(s.root, "downloaded")
	if err := ensureInsideRoot(s.root, downloadedDir); err != nil {
		return DistributionStatus{}, err
	}
	downloadedPath := filepath.Join(downloadedDir, digest+".json")
	recoverInterruptedAtomicReplace(downloadedPath)
	raw, err := os.ReadFile(downloadedPath)
	if err != nil {
		return DistributionStatus{}, fmt.Errorf("read downloaded distribution: %w", err)
	}
	bundle, verifiedDigest, err := VerifyDistributionBundle(raw, trustedKeys, currentAppVersion)
	if err != nil {
		return DistributionStatus{}, err
	}
	if verifiedDigest != digest {
		// The file at downloaded/<digest>.json no longer hashes to <digest>:
		// either it was edited on disk after SaveDownloaded verified it, or
		// this store's own naming invariant was violated by something else
		// writing into this directory.
		return DistributionStatus{}, fmt.Errorf("downloaded distribution content no longer matches its digest")
	}
	acceptedDir := filepath.Join(s.root, "accepted")
	if err := ensureInsideRoot(s.root, acceptedDir); err != nil {
		return DistributionStatus{}, err
	}
	pointerPath := filepath.Join(s.root, "accepted.json")
	recoverInterruptedAtomicReplace(pointerPath)
	if current, readErr := os.ReadFile(pointerPath); readErr == nil {
		if writeErr := writeBytesAtomic(filepath.Join(s.root, "previous.json"), current); writeErr != nil {
			return DistributionStatus{}, writeErr
		}
	} else if !os.IsNotExist(readErr) {
		return DistributionStatus{}, readErr
	}
	if err := writeBytesAtomic(filepath.Join(acceptedDir, digest+".json"), raw); err != nil {
		return DistributionStatus{}, err
	}
	status := DistributionStatus{CatalogVersion: bundle.Payload.CatalogVersion, Digest: digest, State: "accepted"}
	pointer, err := json.Marshal(status)
	if err != nil {
		return DistributionStatus{}, err
	}
	if err := writeBytesAtomic(pointerPath, pointer); err != nil {
		return DistributionStatus{}, err
	}
	return status, nil
}

func (s *DistributionStore) Status() (DistributionStatus, error) {
	if s == nil {
		return DistributionStatus{}, fmt.Errorf("distribution store is nil")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.statusLocked()
}

func (s *DistributionStore) statusLocked() (DistributionStatus, error) {
	pointerPath := filepath.Join(s.root, "accepted.json")
	recoverInterruptedAtomicReplace(pointerPath)
	raw, err := os.ReadFile(pointerPath)
	if os.IsNotExist(err) {
		return DistributionStatus{State: "none"}, nil
	}
	if err != nil {
		return DistributionStatus{}, err
	}
	var accepted DistributionStatus
	if err := json.Unmarshal(raw, &accepted); err != nil {
		return DistributionStatus{}, fmt.Errorf("decode accepted distribution pointer: %w", err)
	}
	if accepted.State == "" {
		accepted.State = "accepted"
	}
	return accepted, nil
}

// Rollback restores the previous accepted pointer. It fully verifies the
// previous pointer's target bundle — digest format, containment, and content
// hash — before writing anything, so a corrupted or tampered previous.json
// or accepted/<digest>.json fails the rollback without touching the current
// accepted pointer at all.
func (s *DistributionStore) Rollback() (DistributionStatus, error) {
	if s == nil {
		return DistributionStatus{}, fmt.Errorf("distribution store is nil")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	previousPath := filepath.Join(s.root, "previous.json")
	recoverInterruptedAtomicReplace(previousPath)
	previous, err := os.ReadFile(previousPath)
	if err != nil {
		return DistributionStatus{}, err
	}
	var accepted DistributionStatus
	if err := json.Unmarshal(previous, &accepted); err != nil {
		return DistributionStatus{}, fmt.Errorf("decode previous distribution pointer: %w", err)
	}
	if accepted.Digest != "" {
		if err := validateDistributionPathComponent(accepted.Digest); err != nil {
			return DistributionStatus{}, fmt.Errorf("previous distribution pointer is invalid: %w", err)
		}
		acceptedDir := filepath.Join(s.root, "accepted")
		if err := ensureInsideRoot(s.root, acceptedDir); err != nil {
			return DistributionStatus{}, err
		}
		bundlePath := filepath.Join(acceptedDir, accepted.Digest+".json")
		recoverInterruptedAtomicReplace(bundlePath)
		bundleRaw, err := os.ReadFile(bundlePath)
		if err != nil {
			return DistributionStatus{}, fmt.Errorf("read previous distribution bundle: %w", err)
		}
		if distributionContentDigest(bundleRaw) != accepted.Digest {
			return DistributionStatus{}, fmt.Errorf("previous distribution bundle content digest mismatch")
		}
	}
	currentPath := filepath.Join(s.root, "accepted.json")
	recoverInterruptedAtomicReplace(currentPath)
	if current, readErr := os.ReadFile(currentPath); readErr == nil {
		if writeErr := writeBytesAtomic(filepath.Join(s.root, "rollback-before.json"), current); writeErr != nil {
			return DistributionStatus{}, writeErr
		}
	} else if !os.IsNotExist(readErr) {
		return DistributionStatus{}, readErr
	}
	if err := writeBytesAtomic(currentPath, previous); err != nil {
		return DistributionStatus{}, err
	}
	if accepted.State == "" {
		accepted.State = "accepted"
	}
	return accepted, nil
}

// LoadAcceptedBundle re-verifies the pointer's target before returning it:
// the digest must still be a valid path component, the stored bytes must
// still hash to that digest, and the schema version must still be current.
// None of that repeats the signature check (accepting already did, and the
// trusted-key set is not threaded through every reader of this store) —
// this is a tamper/corruption check on content this store itself wrote, not
// a re-establishment of trust in an external signer.
func (s *DistributionStore) LoadAcceptedBundle() (DistributionBundle, error) {
	if s == nil {
		return DistributionBundle{}, fmt.Errorf("distribution store is nil")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	status, err := s.statusLocked()
	if err != nil {
		return DistributionBundle{}, err
	}
	if status.Digest == "" {
		return DistributionBundle{}, ErrNoAcceptedDistribution
	}
	if err := validateDistributionPathComponent(status.Digest); err != nil {
		return DistributionBundle{}, err
	}
	acceptedDir := filepath.Join(s.root, "accepted")
	if err := ensureInsideRoot(s.root, acceptedDir); err != nil {
		return DistributionBundle{}, err
	}
	bundlePath := filepath.Join(acceptedDir, status.Digest+".json")
	recoverInterruptedAtomicReplace(bundlePath)
	raw, err := os.ReadFile(bundlePath)
	if err != nil {
		return DistributionBundle{}, err
	}
	if distributionContentDigest(raw) != status.Digest {
		return DistributionBundle{}, fmt.Errorf("accepted distribution content digest mismatch")
	}
	var bundle DistributionBundle
	if err := json.Unmarshal(raw, &bundle); err != nil {
		return DistributionBundle{}, fmt.Errorf("decode accepted distribution: %w", err)
	}
	if bundle.Payload.SchemaVersion != CurrentSchemaVersion {
		return DistributionBundle{}, fmt.Errorf("accepted distribution schema is invalid")
	}
	return bundle, nil
}

func compareVersions(left, right string) int {
	leftParts := strings.Split(strings.TrimPrefix(left, "v"), ".")
	rightParts := strings.Split(strings.TrimPrefix(right, "v"), ".")
	length := len(leftParts)
	if len(rightParts) > length {
		length = len(rightParts)
	}
	for i := 0; i < length; i++ {
		l, _ := strconv.Atoi(partAt(leftParts, i))
		r, _ := strconv.Atoi(partAt(rightParts, i))
		if l < r {
			return -1
		}
		if l > r {
			return 1
		}
	}
	return 0
}

func partAt(parts []string, index int) string {
	if index >= len(parts) {
		return "0"
	}
	return parts[index]
}
