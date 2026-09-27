package provider

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func TestFileStoreCreateNewRejectsExistingID(t *testing.T) {
	store, err := NewFileStore(filepath.Join(t.TempDir(), "providers.d"))
	if err != nil {
		t.Fatal(err)
	}
	definition := testOverride("example-cli", "Example CLI")
	if err := store.CreateNew(definition); err != nil {
		t.Fatalf("first CreateNew failed: %v", err)
	}
	if err := store.CreateNew(definition); !errors.Is(err, ErrProviderAlreadyExists) {
		t.Fatalf("second CreateNew err = %v, want ErrProviderAlreadyExists", err)
	}
}

// TestFileStoreCreateNewIsAtomicUnderConcurrency pins the fix for the
// check-then-save race in the Hub's create handler: a Lookup that finds
// nothing, followed by a separate Save, let two concurrent requests for the
// same id both pass the check and one silently clobber the other. CreateNew
// must let exactly one of N concurrent callers win.
func TestFileStoreCreateNewIsAtomicUnderConcurrency(t *testing.T) {
	store, err := NewFileStore(filepath.Join(t.TempDir(), "providers.d"))
	if err != nil {
		t.Fatal(err)
	}
	const attempts = 8
	var wg sync.WaitGroup
	successes := make([]bool, attempts)
	for i := 0; i < attempts; i++ {
		wg.Add(1)
		go func(index int) {
			defer wg.Done()
			err := store.CreateNew(testOverride("racer", "Racer"))
			successes[index] = err == nil
			if err != nil && !errors.Is(err, ErrProviderAlreadyExists) {
				t.Errorf("unexpected CreateNew error: %v", err)
			}
		}(i)
	}
	wg.Wait()
	won := 0
	for _, ok := range successes {
		if ok {
			won++
		}
	}
	if won != 1 {
		t.Fatalf("winners = %d, want exactly 1", won)
	}
}

func TestFileStoreSaveStillOverwritesExisting(t *testing.T) {
	root := filepath.Join(t.TempDir(), "providers.d")
	store, err := NewFileStore(root)
	if err != nil {
		t.Fatal(err)
	}
	definition := testOverride("example-cli", "Example CLI")
	if err := store.CreateNew(definition); err != nil {
		t.Fatal(err)
	}
	definition.DisplayName = "Updated CLI"
	if err := store.Save(definition); err != nil {
		t.Fatalf("Save on an existing id should overwrite: %v", err)
	}
	raw, err := os.ReadFile(filepath.Join(root, "example-cli.json"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "Updated CLI") {
		t.Fatalf("stored definition was not updated: %s", raw)
	}
}
