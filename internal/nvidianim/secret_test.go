package nvidianim

import (
	"os"
	"path/filepath"
	"testing"
)

func TestResolveAPIKeyEnvOverridesFile(t *testing.T) {
	t.Setenv(APIKeyEnv, "")
	configDir := t.TempDir()
	if err := SaveAPIKey(configDir, "file-key-example"); err != nil {
		t.Fatal(err)
	}
	t.Setenv(APIKeyEnv, " env-key-example ")

	got, source, err := ResolveAPIKey(configDir)
	if err != nil {
		t.Fatal(err)
	}
	if got != "env-key-example" || source != KeySourceEnv {
		t.Fatalf("ResolveAPIKey() = (%q, %q), want env key and env source", got, source)
	}
}

func TestResolveAPIKeyFileFallback(t *testing.T) {
	t.Setenv(APIKeyEnv, "")
	configDir := t.TempDir()
	if err := SaveAPIKey(configDir, " file-key-example\n"); err != nil {
		t.Fatal(err)
	}

	got, source, err := ResolveAPIKey(configDir)
	if err != nil {
		t.Fatal(err)
	}
	if got != "file-key-example" || source != KeySourceFile {
		t.Fatalf("ResolveAPIKey() = (%q, %q), want file key and file source", got, source)
	}
}

func TestResolveAPIKeyMissing(t *testing.T) {
	t.Setenv(APIKeyEnv, "")
	got, source, err := ResolveAPIKey(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if got != "" || source != KeySourceNone {
		t.Fatalf("ResolveAPIKey() = (%q, %q), want no key", got, source)
	}
}

func TestSaveAPIKeyRejectsEmptyAndControlCharacters(t *testing.T) {
	configDir := t.TempDir()
	for _, value := range []string{"", " \t ", "line1\nline2", "key\x00suffix"} {
		if err := SaveAPIKey(configDir, value); err == nil {
			t.Fatalf("SaveAPIKey(%q) succeeded, want rejection", value)
		}
	}
	if _, err := os.Stat(APIKeyPath(configDir)); !os.IsNotExist(err) {
		t.Fatalf("invalid key created a file: stat err = %v", err)
	}
}

func TestSaveDeleteAPIKeyUsesPrivatePath(t *testing.T) {
	t.Setenv(APIKeyEnv, "")
	configDir := t.TempDir()
	if err := SaveAPIKey(configDir, "test-key-example"); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(configDir, "secrets", apiKeyFileName)); err != nil {
		t.Fatalf("key file was not written: %v", err)
	}
	if err := DeleteAPIKey(configDir); err != nil {
		t.Fatal(err)
	}
	if _, source, err := ResolveAPIKey(configDir); err != nil || source != KeySourceNone {
		t.Fatalf("after delete ResolveAPIKey() source = %q, err = %v, want none", source, err)
	}
}
