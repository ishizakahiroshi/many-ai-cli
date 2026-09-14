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

func writeBytesAtomic(path string, data []byte) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return fmt.Errorf("create directory for %s: %w", filepath.Base(path), err)
	}
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
	if err := os.Rename(tmpName, path); err != nil {
		if _, statErr := os.Stat(path); statErr != nil {
			return err
		}
		if removeErr := os.Remove(path); removeErr != nil {
			return err
		}
		if renameErr := os.Rename(tmpName, path); renameErr != nil {
			return renameErr
		}
	}
	return nil
}
