package attach

import (
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// allowedExts は添付ファイルとして許可する拡張子の allowlist（小文字）。
var allowedExts = map[string]bool{
	".png":  true,
	".jpg":  true,
	".jpeg": true,
	".webp": true,
	".gif":  true,
}

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

// Save writes data to baseDir/<sessionID>/<YYYYMMDDHHmmss_NNNNNNNNN><ext> and returns
// the absolute file path and a provider-specific inject string.
// filename is the original file name; its extension takes priority over magic-byte detection.
func Save(baseDir string, sessionID int, provider string, data []byte, filename string) (path, inject string, err error) {
	dir := filepath.Join(baseDir, fmt.Sprintf("%d", sessionID))
	if err = os.MkdirAll(dir, 0o700); err != nil {
		return "", "", fmt.Errorf("attach.Save: mkdir %s: %w", dir, err)
	}

	// 拡張子を allowlist で検証し、allowlist 外なら magic-byte 判定にフォールバック
	ext := strings.ToLower(filepath.Ext(filename))
	if ext == "" || !allowedExts[ext] {
		ext = detectExt(data)
	}

	now := time.Now()
	baseName := fmt.Sprintf("%s_%09d", now.Format("20060102150405"), now.Nanosecond())
	var abs string
	for i := 0; ; i++ {
		savedName := baseName + ext
		if i > 0 {
			savedName = fmt.Sprintf("%s_%d%s", baseName, i, ext)
		}
		abs = filepath.Join(dir, savedName)
		f, openErr := os.OpenFile(abs, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
		if os.IsExist(openErr) {
			continue
		}
		if openErr != nil {
			return "", "", fmt.Errorf("attach.Save: create %s: %w", abs, openErr)
		}
		if _, err = f.Write(data); err != nil {
			_ = f.Close()
			_ = os.Remove(abs)
			return "", "", fmt.Errorf("attach.Save: write %s: %w", abs, err)
		}
		if err = f.Close(); err != nil {
			_ = os.Remove(abs)
			return "", "", fmt.Errorf("attach.Save: close %s: %w", abs, err)
		}
		break
	}

	switch provider {
	case "claude":
		// Claude Code は @path 直後の Enter を画像だけの送信として処理することがある。
		// 本文と同じ入力行に残すため、Codex と同じく空白で区切る。
		inject = "@" + abs + " "
	case "codex":
		inject = "@" + filepath.ToSlash(abs) + " "
	default:
		// fallback for future providers: bare path
		inject = abs + " "
	}

	return abs, inject, nil
}

// CleanOld removes files under baseDir whose mtime is older than retentionDays,
// then removes empty session subdirectories.
// Individual removal failures are logged and skipped; a single failure does not abort the walk.
func CleanOld(baseDir string, retentionDays int) error {
	cutoff := time.Now().AddDate(0, 0, -retentionDays)

	// 1パス目: 古いファイルを削除（失敗はログして継続）
	if err := filepath.Walk(baseDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			slog.Debug("attach.CleanOld: walk error", "path", path, "err", err)
			return nil // walk を止めない
		}
		if info.IsDir() {
			return nil
		}
		if info.ModTime().Before(cutoff) {
			if removeErr := os.Remove(path); removeErr != nil {
				slog.Debug("attach.CleanOld: remove failed", "path", path, "err", removeErr)
				// walk を止めない（return nil）
			}
		}
		return nil
	}); err != nil {
		return fmt.Errorf("attach.CleanOld: walk %s: %w", baseDir, err)
	}

	// 2パス目: 空になったセッションサブディレクトリを削除
	entries, err := os.ReadDir(baseDir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return fmt.Errorf("attach.CleanOld: readdir %s: %w", baseDir, err)
	}
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		subDir := filepath.Join(baseDir, e.Name())
		children, readErr := os.ReadDir(subDir)
		if readErr != nil {
			slog.Debug("attach.CleanOld: readdir failed", "path", subDir, "err", readErr)
			continue
		}
		if len(children) == 0 {
			if removeErr := os.Remove(subDir); removeErr != nil {
				slog.Debug("attach.CleanOld: remove dir failed", "path", subDir, "err", removeErr)
			}
		}
	}
	return nil
}
