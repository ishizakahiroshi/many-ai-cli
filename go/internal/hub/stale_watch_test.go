package hub

import (
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// newTestBinaryGuard は path の現在の内容を「起動時のバイナリ」として固定した
// binaryGuard を返す（newBinaryGuard は os.Executable() を見るためテストに使えない）。
func newTestBinaryGuard(t *testing.T, path string) *binaryGuard {
	t.Helper()
	sha, size, mod, err := hashFileStat(path)
	if err != nil {
		t.Fatalf("hashFileStat: %v", err)
	}
	return &binaryGuard{exePath: path, startSHA: sha, lastSHA: sha, lastSize: size, lastMod: mod}
}

// writeBinary は内容を書き換え、mtime も必ず動かす。
// binaryGuard は (size, mtime) が同じなら再ハッシュしないため、同サイズで内容だけ
// 変えるケースを取りこぼさないよう mtime を明示的に進める。
func writeBinary(t *testing.T, path, content string, mod time.Time) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}
	if err := os.Chtimes(path, mod, mod); err != nil {
		t.Fatalf("chtimes: %v", err)
	}
}

// TestNoteStaleBinaryBroadcastsOnChangeOnly は「変化した瞬間だけ配信する」ことを確認する。
// /api/info は新規セッション起動のたびに呼ばれるため、毎回配信すると UI のバナーが
// 作り直され続け、承認バーで起きたのと同型の点滅を招く。
func TestNoteStaleBinaryBroadcastsOnChangeOnly(t *testing.T) {
	s := &Server{logger: slog.New(slog.NewTextHandler(io.Discard, nil))}

	if s.noteStaleBinaryChanged(false) {
		t.Fatal("initial false: changed=true, want false (no broadcast)")
	}
	if !s.noteStaleBinaryChanged(true) {
		t.Fatal("false->true: changed=false, want true (broadcast once)")
	}
	if s.noteStaleBinaryChanged(true) {
		t.Fatal("true->true: changed=true, want false (no resend)")
	}
	if !s.noteStaleBinaryChanged(false) {
		t.Fatal("true->false: changed=false, want true (clear the banner)")
	}
}

// TestInfoHandlerDetectsReplacedBinary は「make build で exe が差し替わったら
// /api/info が stale を申告し、通知対象になる」ことを実ファイルで確認する。
func TestInfoHandlerDetectsReplacedBinary(t *testing.T) {
	dir := t.TempDir()
	exe := filepath.Join(dir, "many-ai-cli.exe")
	base := time.Now().Add(-time.Hour)
	writeBinary(t, exe, "build-A", base)

	guard := newTestBinaryGuard(t, exe)
	if guard.IsStale() {
		t.Fatal("fresh binary reported as stale")
	}

	// make build 相当。size も mtime も変わる。
	writeBinary(t, exe, "build-B-longer", base.Add(time.Minute))
	if !guard.IsStale() {
		t.Fatal("replaced binary not reported as stale")
	}

	// 元へ戻したら stale が解ける（バナー固着の解除）。
	writeBinary(t, exe, "build-A", base.Add(2*time.Minute))
	if guard.IsStale() {
		t.Fatal("restored binary still reported as stale")
	}
}

// TestStaleBinaryGuardUnavailable は自己ハッシュを取れなかった環境
// （newBinaryGuard が startSHA を埋められなかった場合）で誤検知しないことを確認する。
func TestStaleBinaryGuardUnavailable(t *testing.T) {
	if (&binaryGuard{}).IsStale() {
		t.Fatal("guard without startSHA reported stale")
	}
}
