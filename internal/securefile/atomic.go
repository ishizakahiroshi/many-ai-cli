package securefile

import (
	"fmt"
	"os"
	"path/filepath"
)

// WriteAtomic writes a private file through a same-directory temporary file
// and rename. Readers never observe a partially written file.
func WriteAtomic(path string, data []byte, mode os.FileMode) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("securefile.WriteAtomic mkdir: %w", err)
	}
	tmp, err := os.CreateTemp(dir, "."+filepath.Base(path)+".tmp-")
	if err != nil {
		return fmt.Errorf("securefile.WriteAtomic create temp: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if err := tmp.Chmod(mode); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("securefile.WriteAtomic chmod temp: %w", err)
	}
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("securefile.WriteAtomic write temp: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("securefile.WriteAtomic sync temp: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("securefile.WriteAtomic close temp: %w", err)
	}
	if err := os.Rename(tmpName, path); err != nil {
		return fmt.Errorf("securefile.WriteAtomic rename: %w", err)
	}
	if err := os.Chmod(path, mode); err != nil {
		return fmt.Errorf("securefile.WriteAtomic chmod: %w", err)
	}
	_ = RestrictFile(path)
	return nil
}
