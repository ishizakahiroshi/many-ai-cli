// Package whisperruntime embeds OS-local runtime payloads (e.g. the Windows
// Visual C++ runtime DLLs that whisper-server links against) and lays them down
// next to the managed whisper-server binary so the install is self-contained
// (no System32 / no machine-wide runtime dependency). Everything lives under
// ~/.many-ai-cli/whisper/ so an uninstall (RemoveAll) leaves zero trace.
//
// Build-safety: files/ ships placeholder READMEs so `go:embed` always finds at
// least one file and the build succeeds even before real signed DLLs are
// dropped in. The real payload is added at release/CI time by
// fetch_windows_runtime.ps1; until then Ensure simply copies nothing.
package whisperruntime

import (
	"embed"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
)

//go:embed all:files
var files embed.FS

// Ensure copies the embedded runtime payload for osArch (e.g. "windows-amd64")
// into binDir. It is idempotent (files already present are skipped) and a no-op
// when no payload is embedded for the platform (e.g. Linux/macOS builds with
// GGML_OPENMP=OFF need no bundled runtime).
func Ensure(binDir, osArch string) error {
	root := "files/" + osArch
	entries, err := fs.ReadDir(files, root)
	if err != nil {
		// このディレクトリ自体が無い = この OS/arch 用の同梱物なし。正常系。
		return nil
	}
	madeDir := false
	for _, e := range entries {
		if e.IsDir() || isPlaceholder(e.Name()) {
			continue
		}
		if !madeDir {
			if err := os.MkdirAll(binDir, 0o700); err != nil {
				return err
			}
			madeDir = true
		}
		dst := filepath.Join(binDir, e.Name())
		if _, statErr := os.Stat(dst); statErr == nil {
			continue // 既存はスキップ（冪等）
		}
		data, readErr := files.ReadFile(root + "/" + e.Name())
		if readErr != nil {
			return readErr
		}
		if err := os.WriteFile(dst, data, 0o700); err != nil {
			return err
		}
	}
	return nil
}

// HasPayload reports whether any non-placeholder runtime file is embedded for
// osArch. Useful for tests/diagnostics ("are the real DLLs bundled yet?").
func HasPayload(osArch string) bool {
	entries, err := fs.ReadDir(files, "files/"+osArch)
	if err != nil {
		return false
	}
	for _, e := range entries {
		if !e.IsDir() && !isPlaceholder(e.Name()) {
			return true
		}
	}
	return false
}

func isPlaceholder(name string) bool {
	lower := strings.ToLower(name)
	// ドットファイル（.gitkeep / .gitignore 等の VCS・scaffold メタ）と README を
	// 除外する。実ランタイムは .dll など非ドットファイルなので影響しない。
	return strings.HasPrefix(lower, ".") || strings.HasSuffix(lower, ".md")
}
