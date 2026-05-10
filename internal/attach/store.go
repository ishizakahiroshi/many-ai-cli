package attach

import (
	"fmt"
	"os"
	"path/filepath"
	"time"
)

// detectExt returns the file extension for img based on magic bytes.
// Falls back to ".png" for unknown or unsupported formats.
func detectExt(data []byte) string {
	switch {
	case len(data) >= 8 && data[0] == 0x89 && data[1] == 'P' && data[2] == 'N' && data[3] == 'G':
		return ".png"
	case len(data) >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF:
		return ".jpg"
	case len(data) >= 6 && (string(data[:6]) == "GIF87a" || string(data[:6]) == "GIF89a"):
		return ".gif"
	case len(data) >= 12 && string(data[:4]) == "RIFF" && string(data[8:12]) == "WEBP":
		return ".webp"
	default:
		return ".png"
	}
}

// Save writes data to baseDir/<sessionID>/<YYYYMMDDHHmmss><ext> and returns
// the absolute file path and a provider-specific inject string.
// filename is the original file name; its extension takes priority over magic-byte detection.
func Save(baseDir string, sessionID int, provider string, data []byte, filename string) (path, inject string, err error) {
	dir := filepath.Join(baseDir, fmt.Sprintf("%d", sessionID))
	if err = os.MkdirAll(dir, 0o700); err != nil {
		return "", "", fmt.Errorf("attach.Save: mkdir %s: %w", dir, err)
	}

	ext := filepath.Ext(filename)
	if ext == "" {
		ext = detectExt(data)
	}

	savedName := time.Now().Format("20060102150405") + ext
	abs := filepath.Join(dir, savedName)

	if err = os.WriteFile(abs, data, 0o600); err != nil {
		return "", "", fmt.Errorf("attach.Save: write %s: %w", abs, err)
	}

	switch provider {
	case "claude":
		// \r でピッカーを閉じてファイル確定させる。次の pty_input の \r がメッセージ送信に使われる
		inject = "@" + abs + "\r"
	case "codex":
		inject = "@" + filepath.ToSlash(abs) + " "
	default:
		// fallback for future providers: bare path
		inject = abs + " "
	}

	return abs, inject, nil
}

// CleanOld removes files under baseDir whose mtime is older than retentionDays.
func CleanOld(baseDir string, retentionDays int) error {
	cutoff := time.Now().AddDate(0, 0, -retentionDays)

	return filepath.Walk(baseDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return nil
		}
		if !info.IsDir() && info.ModTime().Before(cutoff) {
			if removeErr := os.Remove(path); removeErr != nil {
				return fmt.Errorf("attach.CleanOld: remove %s: %w", path, removeErr)
			}
		}
		return nil
	})
}
