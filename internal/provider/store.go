package provider

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
)

// ErrProviderAlreadyExists is returned by CreateNew when a provider with the
// same id is already stored, so a caller can tell "someone else created this
// id first" apart from a generic write failure.
var ErrProviderAlreadyExists = errors.New("provider already exists")

// Store is the narrow persistence boundary used by the Hub provider API. The
// store receives a fixed root from startup; request data never chooses a path.
type Store interface {
	Load() ([]Definition, []Diagnostic, error)
	// CreateNew persists a definition only if no file for its id exists yet.
	// It must fail atomically (ErrProviderAlreadyExists) rather than silently
	// overwriting when two creates for the same id race.
	CreateNew(Definition) error
	Save(Definition) error
	Delete(string) error
}

type FileStore struct {
	mu   sync.Mutex
	root string
}

func NewFileStore(root string) (*FileStore, error) {
	if strings.TrimSpace(root) == "" {
		return nil, fmt.Errorf("provider store root is required")
	}
	return &FileStore{root: filepath.Clean(root)}, nil
}

func (s *FileStore) Load() ([]Definition, []Diagnostic, error) {
	if s == nil {
		return nil, nil, fmt.Errorf("provider store is nil")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	entries, err := os.ReadDir(s.root)
	if os.IsNotExist(err) {
		return nil, nil, nil
	}
	if err != nil {
		return nil, nil, fmt.Errorf("read provider store: %w", err)
	}
	var names []string
	for _, entry := range entries {
		if entry.IsDir() || filepath.Ext(entry.Name()) != ".json" {
			continue
		}
		names = append(names, entry.Name())
	}
	sort.Strings(names)
	var definitions []Definition
	var diagnostics []Diagnostic
	for _, name := range names {
		path := filepath.Join(s.root, name)
		raw, readErr := os.ReadFile(path)
		if readErr != nil {
			return definitions, diagnostics, fmt.Errorf("read provider definition %q: %w", name, readErr)
		}
		var definition Definition
		if decodeErr := json.Unmarshal(raw, &definition); decodeErr != nil {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_user_definition", Severity: SeverityError, Field: name, Message: "definition could not be decoded"})
			continue
		}
		definition.Source.Origin = OriginUser
		definitionDiagnostics, validateErr := ValidateDefinition(raw, DefaultAdapterCatalog())
		if validateErr != nil {
			diagnostics = append(diagnostics, Diagnostic{Code: "invalid_user_definition", Severity: SeverityError, Field: name, Message: "definition could not be validated"})
			continue
		}
		for i := range definitionDiagnostics {
			definitionDiagnostics[i].Field = name + "." + definitionDiagnostics[i].Field
		}
		diagnostics = append(diagnostics, definitionDiagnostics...)
		if hasDiagnosticError(definitionDiagnostics) {
			continue
		}
		definitions = append(definitions, definition)
	}
	return definitions, diagnostics, nil
}

// validateAndEncodeUserDefinition is the single place Save and CreateNew
// derive the on-disk bytes from, so a validation rule added to one path can't
// be forgotten on the other.
func validateAndEncodeUserDefinition(definition Definition) ([]byte, error) {
	raw, err := json.MarshalIndent(definition, "", "  ")
	if err != nil {
		return nil, fmt.Errorf("encode provider definition: %w", err)
	}
	diagnostics, err := ValidateDefinition(raw, DefaultAdapterCatalog())
	if err != nil {
		return nil, err
	}
	if hasDiagnosticError(diagnostics) {
		return nil, fmt.Errorf("provider definition is invalid")
	}
	if IsBuiltinID(definition.ID) || definition.ID == "shell" {
		return nil, fmt.Errorf("provider id %q is reserved", definition.ID)
	}
	if err := ValidateUserID(definition.ID); err != nil {
		return nil, err
	}
	return raw, nil
}

// CreateNew persists a brand-new definition and fails with
// ErrProviderAlreadyExists if the id is already stored. The exclusive open is
// the atomic step: unlike Save's check-then-write, two concurrent CreateNew
// calls for the same id cannot both succeed, so the second writer never
// silently clobbers the first.
func (s *FileStore) CreateNew(definition Definition) error {
	if s == nil {
		return fmt.Errorf("provider store is nil")
	}
	raw, err := validateAndEncodeUserDefinition(definition)
	if err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := os.MkdirAll(s.root, 0o700); err != nil {
		return fmt.Errorf("create provider store: %w", err)
	}
	target := filepath.Join(s.root, definition.ID+".json")
	file, err := os.OpenFile(target, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		if os.IsExist(err) {
			return ErrProviderAlreadyExists
		}
		return fmt.Errorf("create provider definition: %w", err)
	}
	if _, err := file.Write(raw); err != nil {
		_ = file.Close()
		_ = os.Remove(target)
		return fmt.Errorf("write provider definition: %w", err)
	}
	if err := file.Close(); err != nil {
		_ = os.Remove(target)
		return fmt.Errorf("close provider definition: %w", err)
	}
	return nil
}

func (s *FileStore) Save(definition Definition) error {
	if s == nil {
		return fmt.Errorf("provider store is nil")
	}
	raw, err := validateAndEncodeUserDefinition(definition)
	if err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := os.MkdirAll(s.root, 0o700); err != nil {
		return fmt.Errorf("create provider store: %w", err)
	}
	tmp, err := os.CreateTemp(s.root, ".provider-*.json")
	if err != nil {
		return fmt.Errorf("create provider temp file: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if err := tmp.Chmod(0o600); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("chmod provider temp file: %w", err)
	}
	if _, err := tmp.Write(raw); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write provider definition: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close provider definition: %w", err)
	}
	target := filepath.Join(s.root, definition.ID+".json")
	if err := os.Rename(tmpName, target); err != nil {
		// Windows does not replace an existing file with Rename. Keep the
		// fallback narrow; C9 will add the verified backup/replace protocol.
		if _, statErr := os.Stat(target); statErr != nil {
			return fmt.Errorf("replace provider definition: %w", err)
		}
		if removeErr := os.Remove(target); removeErr != nil {
			return fmt.Errorf("replace provider definition: %w", err)
		}
		if renameErr := os.Rename(tmpName, target); renameErr != nil {
			return fmt.Errorf("replace provider definition: %w", renameErr)
		}
	}
	return nil
}

func (s *FileStore) Delete(id string) error {
	if s == nil {
		return fmt.Errorf("provider store is nil")
	}
	if err := ValidateUserID(id); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := os.Remove(filepath.Join(s.root, id+".json")); err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("provider %q is not stored", id)
		}
		return fmt.Errorf("delete provider definition: %w", err)
	}
	return nil
}

func IsBuiltinID(id string) bool {
	for _, builtin := range BuiltinProviderIDs {
		if id == builtin {
			return true
		}
	}
	return false
}

func ValidateUserID(id string) error {
	if err := validateID(id); err != nil {
		return err
	}
	if IsBuiltinID(id) || id == "shell" {
		return fmt.Errorf("provider id %q is reserved", id)
	}
	return nil
}

func hasDiagnosticError(diagnostics []Diagnostic) bool {
	for _, diagnostic := range diagnostics {
		if diagnostic.IsError() {
			return true
		}
	}
	return false
}
