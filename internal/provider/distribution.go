package provider

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

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

func (s *DistributionStore) SaveDownloaded(bundle DistributionBundle, digest string) error {
	if s == nil {
		return fmt.Errorf("distribution store is nil")
	}
	raw, err := json.MarshalIndent(bundle, "", "  ")
	if err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	path := filepath.Join(s.root, "downloaded", bundle.Payload.CatalogVersion+"-"+digest+".json")
	return writeBytesAtomic(path, raw)
}

func (s *DistributionStore) Accept(bundle DistributionBundle, digest string) error {
	if s == nil {
		return fmt.Errorf("distribution store is nil")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	accepted := map[string]string{"catalog_version": bundle.Payload.CatalogVersion, "digest": digest}
	raw, err := json.Marshal(accepted)
	if err != nil {
		return err
	}
	return writeBytesAtomic(filepath.Join(s.root, "accepted.json"), raw)
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
