// Package nvidianim contains the small amount of NVIDIA NIM state shared by
// the Hub. API keys are deliberately kept out of config.yaml and protocol
// structs.
package nvidianim

import (
	"errors"
	"os"
	"path/filepath"
	"strings"

	"many-ai-cli/internal/securefile"
)

const APIKeyEnv = "NVIDIA_API_KEY"

const apiKeyFileName = "nvidia_api_key"

type KeySource string

const (
	KeySourceNone KeySource = "none"
	KeySourceEnv  KeySource = "env"
	KeySourceFile KeySource = "file"
)

var (
	ErrEmptyAPIKey       = errors.New("NVIDIA API key must not be empty")
	ErrInvalidAPIKeyText = errors.New("NVIDIA API key contains invalid characters")
	errAPIKeyRead        = errors.New("failed to read NVIDIA API key")
)

// APIKeyPath returns the private API-key file path under configDir.
func APIKeyPath(configDir string) string {
	return filepath.Join(configDir, "secrets", apiKeyFileName)
}

// ResolveAPIKey prefers a non-empty Hub process environment variable and
// falls back to the private key file. It never returns file contents in an
// error.
func ResolveAPIKey(configDir string) (string, KeySource, error) {
	if value := strings.TrimSpace(os.Getenv(APIKeyEnv)); value != "" {
		if err := validateAPIKey(value); err != nil {
			return "", KeySourceNone, err
		}
		return value, KeySourceEnv, nil
	}

	data, err := os.ReadFile(APIKeyPath(configDir))
	if errors.Is(err, os.ErrNotExist) {
		return "", KeySourceNone, nil
	}
	if err != nil {
		return "", KeySourceNone, errAPIKeyRead
	}
	value := strings.TrimSpace(string(data))
	if value == "" {
		return "", KeySourceNone, nil
	}
	if err := validateAPIKey(value); err != nil {
		return "", KeySourceNone, err
	}
	return value, KeySourceFile, nil
}

// SaveAPIKey writes a key atomically with owner-only directory and file
// permissions. An empty value is rejected; callers must use DeleteAPIKey for
// explicit removal.
func SaveAPIKey(configDir, value string) error {
	value = strings.TrimSpace(value)
	if value == "" {
		return ErrEmptyAPIKey
	}
	if err := validateAPIKey(value); err != nil {
		return err
	}
	path := APIKeyPath(configDir)
	if err := securefile.EnsurePrivateDir(filepath.Dir(path)); err != nil {
		return errors.New("failed to secure NVIDIA API key directory")
	}
	if err := securefile.WriteAtomic(path, []byte(value), 0o600); err != nil {
		return errors.New("failed to save NVIDIA API key")
	}
	return nil
}

// DeleteAPIKey removes the file-backed key. It does not change a process
// environment variable, which remains the higher-priority source.
func DeleteAPIKey(configDir string) error {
	err := os.Remove(APIKeyPath(configDir))
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return errors.New("failed to delete NVIDIA API key")
	}
	return nil
}

func validateAPIKey(value string) error {
	if strings.ContainsAny(value, "\x00\r\n") {
		return ErrInvalidAPIKeyText
	}
	return nil
}
