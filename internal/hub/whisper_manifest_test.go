package hub

import (
	"archive/zip"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

func TestWhisperBinariesWindowsEntry(t *testing.T) {
	entry, ok := whisperBinaries["windows/amd64"]
	if !ok {
		t.Fatal("windows/amd64 entry missing from whisperBinaries")
	}
	if entry.Runtime != "windows-amd64" {
		t.Fatalf("Runtime = %q, want windows-amd64", entry.Runtime)
	}
	mustContain(t, entry.ServerNames, "whisper-server.exe")
	// 公式 zip からは server 一式のみ抽出し SDL2.dll は同梱しない（D1）。
	mustContain(t, entry.KeepFromArchive, "whisper-server.exe")
	for _, k := range entry.KeepFromArchive {
		if k == "SDL2.dll" {
			t.Fatal("SDL2.dll must not be extracted from the zip")
		}
	}
}

func TestWhisperBinaryForHostUnknown(t *testing.T) {
	if _, ok := whisperBinaries["plan9/riscv64"]; ok {
		t.Fatal("unexpected entry for plan9/riscv64")
	}
}

func TestWhisperArchiveKind(t *testing.T) {
	cases := []struct {
		entry whisperBinaryEntry
		want  string
	}{
		{whisperBinaryEntry{Archive: "zip"}, "zip"},
		{whisperBinaryEntry{Archive: "tar.gz"}, "tar.gz"},
		{whisperBinaryEntry{URL: "https://x/y/whisper-server-linux-amd64.tar.gz"}, "tar.gz"},
		{whisperBinaryEntry{URL: "https://x/y/foo.tgz"}, "tar.gz"},
		{whisperBinaryEntry{URL: "https://x/y/whisper-bin-x64.zip"}, "zip"},
	}
	for _, c := range cases {
		if got := whisperArchiveKind(c.entry); got != c.want {
			t.Errorf("whisperArchiveKind(%+v) = %q, want %q", c.entry, got, c.want)
		}
	}
}

func TestWhisperArchiveName(t *testing.T) {
	got := whisperArchiveName(whisperBinaryEntry{URL: "https://x/y/whisper-bin-x64.zip", Archive: "zip"})
	if got != "whisper-bin-x64.zip" {
		t.Fatalf("whisperArchiveName = %q, want whisper-bin-x64.zip", got)
	}
}

func TestWhisperServerNamesNonEmpty(t *testing.T) {
	names := whisperServerNames()
	if len(names) == 0 {
		t.Fatal("whisperServerNames returned empty")
	}
	// host が map に在れば entry.ServerNames、無ければ既定候補。いずれも
	// whisper-server 系の名前を必ず含む。
	if !containsAny(names, "whisper-server", "whisper-server.exe") {
		t.Fatalf("server names %v missing whisper-server", names)
	}
}

func TestBakedWhisperServerPath(t *testing.T) {
	if bakedWhisperServerPath() != "" {
		t.Skip("MANY_AI_CLI_WHISPER_SERVER already set in env")
	}
	dir := t.TempDir()
	bin := filepath.Join(dir, "whisper-server")
	if err := os.WriteFile(bin, []byte("#!/bin/sh\n"), 0o700); err != nil {
		t.Fatal(err)
	}
	t.Setenv(whisperServerEnvVar, bin)
	if got := bakedWhisperServerPath(); got != bin {
		t.Fatalf("bakedWhisperServerPath = %q, want %q", got, bin)
	}
	// 焼き込みパスがあれば、どの OS でも managed はサポート扱い。
	if !whisperManagedSupported() {
		t.Fatal("whisperManagedSupported should be true when baked binary is set")
	}
	// findWhisperServerExe は dir 内に何も無くても baked パスを返す。
	if p, err := findWhisperServerExe(t.TempDir()); err != nil || p != bin {
		t.Fatalf("findWhisperServerExe = (%q,%v), want (%q,nil)", p, err, bin)
	}

	t.Setenv(whisperServerEnvVar, filepath.Join(dir, "missing"))
	if bakedWhisperServerPath() != "" {
		t.Fatal("bakedWhisperServerPath should be empty when the file is absent")
	}
}

func TestExtractZipSelectedFlattens(t *testing.T) {
	zipPath := filepath.Join(t.TempDir(), "src.zip")
	makeZip(t, zipPath, map[string]string{
		"Release/whisper-server.exe": "server",
		"Release/whisper.dll":        "dll",
		"Release/SDL2.dll":           "sdl",
		"Release/main.exe":           "main",
	})
	dest := t.TempDir()
	if err := extractZipSelected(zipPath, dest, []string{"whisper-server.exe", "whisper.dll"}); err != nil {
		t.Fatal(err)
	}
	if !fileExistsTest(filepath.Join(dest, "whisper-server.exe")) {
		t.Fatal("whisper-server.exe not extracted (should be flattened to dest root)")
	}
	if !fileExistsTest(filepath.Join(dest, "whisper.dll")) {
		t.Fatal("whisper.dll not extracted")
	}
	if fileExistsTest(filepath.Join(dest, "SDL2.dll")) {
		t.Fatal("SDL2.dll should be skipped")
	}
	if fileExistsTest(filepath.Join(dest, "main.exe")) {
		t.Fatal("main.exe should be skipped")
	}
	// Release/ サブディレクトリは作られない（平坦化）。
	if fileExistsTest(filepath.Join(dest, "Release", "whisper-server.exe")) {
		t.Fatal("nested Release/ path should not exist")
	}
}

func TestEnsureWhisperRuntimeNoPayload(t *testing.T) {
	// 実行ホストに同梱物が無い（または placeholder のみ）場合でも冪等に成功する。
	if runtime.GOOS != "windows" {
		dir := t.TempDir()
		if err := ensureWhisperRuntime(dir); err != nil {
			t.Fatalf("ensureWhisperRuntime = %v, want nil", err)
		}
	}
}

// --- helpers ---

func makeZip(t *testing.T, path string, files map[string]string) {
	t.Helper()
	f, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	zw := zip.NewWriter(f)
	for name, body := range files {
		w, err := zw.Create(name)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := w.Write([]byte(body)); err != nil {
			t.Fatal(err)
		}
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
}

func fileExistsTest(path string) bool {
	st, err := os.Stat(path)
	return err == nil && !st.IsDir()
}

func mustContain(t *testing.T, list []string, want string) {
	t.Helper()
	if !containsAny(list, want) {
		t.Fatalf("%v does not contain %q", list, want)
	}
}

func containsAny(list []string, wants ...string) bool {
	for _, item := range list {
		for _, w := range wants {
			if item == w {
				return true
			}
		}
	}
	return false
}
