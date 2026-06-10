package attach

import (
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

// detectExt returns the file extension based on magic bytes.
// Falls back to ".bin" for unknown or unsupported formats.
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
	case len(data) >= 5 && string(data[:5]) == "%PDF-":
		return ".pdf"
	default:
		return ".bin"
	}
}

func extFromFilename(filename string) string {
	ext := strings.ToLower(filepath.Ext(filepath.Base(filename)))
	if len(ext) < 2 || len(ext) > 32 {
		return ""
	}
	for _, r := range ext[1:] {
		if r >= 'a' && r <= 'z' {
			continue
		}
		if r >= '0' && r <= '9' {
			continue
		}
		if r == '_' || r == '-' {
			continue
		}
		return ""
	}
	return ext
}

// Save writes data to baseDir/<sessionID>/<YYYYMMDDHHmmss_NNNNNNNNN><ext> and returns
// the absolute file path and a provider-specific inject string.
// filename is the original file name; its extension takes priority over magic-byte detection.
func Save(baseDir string, sessionID int, provider string, data []byte, filename string) (path, inject string, err error) {
	dir := filepath.Join(baseDir, fmt.Sprintf("%d", sessionID))
	if err = os.MkdirAll(dir, 0o700); err != nil {
		return "", "", fmt.Errorf("attach.Save: mkdir %s: %w", dir, err)
	}

	ext := extFromFilename(filename)
	if ext == "" {
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
			if removeErr := os.Remove(path); removeErr != nil { // #nosec G122 -- 自ユーザー所有の添付ディレクトリ内のみを掃除（外部入力パスなし）
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

// attachFile は EnforceTotalSize の内部用。1 添付ファイルのパス・サイズ・mtime。
type attachFile struct {
	path  string
	size  int64
	mtime time.Time
}

// EnforceTotalSize は baseDir 配下の添付ファイル合計サイズが maxBytes を超えている場合、
// mtime が古いファイルから順に削除して上限内に収める。maxBytes <= 0 のときは何もしない。
// 削除後、空になったセッションサブディレクトリも片付ける。
// 個別の削除失敗はログして継続する（walk を止めない）。
func EnforceTotalSize(baseDir string, maxBytes int64) error {
	if maxBytes <= 0 {
		return nil
	}

	var files []attachFile
	var total int64
	if err := filepath.Walk(baseDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			slog.Debug("attach.EnforceTotalSize: walk error", "path", path, "err", err)
			return nil // walk を止めない
		}
		if info.IsDir() {
			return nil
		}
		files = append(files, attachFile{path: path, size: info.Size(), mtime: info.ModTime()})
		total += info.Size()
		return nil
	}); err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return fmt.Errorf("attach.EnforceTotalSize: walk %s: %w", baseDir, err)
	}

	if total <= maxBytes {
		return nil
	}

	// 古い mtime から削除（同 mtime はパス順で安定化）
	sort.Slice(files, func(i, j int) bool {
		if files[i].mtime.Equal(files[j].mtime) {
			return files[i].path < files[j].path
		}
		return files[i].mtime.Before(files[j].mtime)
	})
	for _, f := range files {
		if total <= maxBytes {
			break
		}
		if removeErr := os.Remove(f.path); removeErr != nil { // #nosec G122 -- 自ユーザー所有の添付ディレクトリ内のみを掃除（外部入力パスなし）
			slog.Debug("attach.EnforceTotalSize: remove failed", "path", f.path, "err", removeErr)
			continue // 消せなかった分は total から引かない
		}
		total -= f.size
	}

	// 空になったセッションサブディレクトリを削除
	entries, err := os.ReadDir(baseDir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return fmt.Errorf("attach.EnforceTotalSize: readdir %s: %w", baseDir, err)
	}
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		subDir := filepath.Join(baseDir, e.Name())
		children, readErr := os.ReadDir(subDir)
		if readErr != nil {
			slog.Debug("attach.EnforceTotalSize: readdir failed", "path", subDir, "err", readErr)
			continue
		}
		if len(children) == 0 {
			if removeErr := os.Remove(subDir); removeErr != nil {
				slog.Debug("attach.EnforceTotalSize: remove dir failed", "path", subDir, "err", removeErr)
			}
		}
	}
	return nil
}
