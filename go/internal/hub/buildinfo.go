package hub

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"sync"
	"time"
)

// BuildInfo は main パッケージ（ldflags 注入）から渡される人間可読のビルド識別子。
// バージョン文字列だけでは同一バージョン内のビルド差（修正前/修正後）を区別できない
// ため、稼働中 Hub の素性確認を補助する付加情報として /api/info に出す。
type BuildInfo struct {
	GitCommit string // -X main.gitCommit 注入（未注入時は空）
	BuildTime string // -X main.buildTime 注入（未注入時は空）
}

// binaryGuard は「稼働中 Hub が起動時のバイナリのままか」を判定する。
//
// プロセスは起動時に実行ファイルをメモリへ載せるため、後から make build 等で
// ディスクの exe を差し替えても走り続ける Hub は古いまま動き続ける。これが
// 「ビルドしたのに直らない」混乱の正体（bugfix_chat-proxy-request-body-truncation）。
//
// guard は起動時に自身の実行ファイル内容の SHA256 を確定し、以後 /api/info が
// 呼ばれるたびに「現在ディスク上の同じパスの exe」と内容ハッシュを比較する。
// 食い違えば「ディスクには新バイナリがあるのに古い Hub が動き続けている」=stale。
//
// 比較コスト対策: 毎回 20MB 級のバイナリをハッシュし直すのは無駄なので、まず
// os.Stat の (size, mtime) を見て前回と同じなら再ハッシュしない。変化したときだけ
// 再ハッシュして結果をキャッシュする。
type binaryGuard struct {
	exePath  string // os.Executable() の解決結果（空なら判定不能）
	startSHA string // 起動時の内容ハッシュ（空なら判定不能）

	mu       sync.Mutex
	lastSize int64
	lastMod  time.Time
	lastSHA  string // (lastSize,lastMod) 時点の内容ハッシュのキャッシュ
}

// newBinaryGuard は起動時の実行ファイルパスと内容ハッシュを確定する。
// 解決・ハッシュに失敗しても致命的にはしない（stale 判定が無効化されるだけ）。
func newBinaryGuard() *binaryGuard {
	g := &binaryGuard{}
	exe, err := os.Executable()
	if err != nil {
		return g
	}
	g.exePath = exe
	if sha, size, mod, err := hashFileStat(exe); err == nil {
		g.startSHA = sha
		g.lastSHA = sha
		g.lastSize = size
		g.lastMod = mod
	}
	return g
}

// StartSHA は起動時に確定した実行ファイル内容ハッシュ（/api/info の binary_sha256）。
func (g *binaryGuard) StartSHA() string {
	if g == nil {
		return ""
	}
	return g.startSHA
}

// IsStale は「ディスク上の exe が起動時と内容で食い違っているか」を返す。
// 判定不能（exe 未解決・起動時ハッシュ取得失敗・現在の stat 失敗）の場合は false。
func (g *binaryGuard) IsStale() bool {
	if g == nil || g.exePath == "" || g.startSHA == "" {
		return false
	}
	fi, err := os.Stat(g.exePath)
	if err != nil {
		// 差し替え途中などで一時的に stat できないだけかもしれない。
		// 古いと誤断定して自動再起動を誘発しないよう false 寄せ。
		return false
	}
	size, mod := fi.Size(), fi.ModTime()

	g.mu.Lock()
	defer g.mu.Unlock()
	if size == g.lastSize && mod.Equal(g.lastMod) {
		return g.lastSHA != g.startSHA
	}
	// (size,mtime) が変わった → 再ハッシュしてキャッシュ更新。
	sha, _, _, err := hashFileStat(g.exePath)
	if err != nil {
		return false
	}
	g.lastSize = size
	g.lastMod = mod
	g.lastSHA = sha
	return sha != g.startSHA
}

// hashFileStat は path の内容 SHA256(16進) と stat の (size, mtime) を返す。
func hashFileStat(path string) (sha string, size int64, mod time.Time, err error) {
	f, err := os.Open(path)
	if err != nil {
		return "", 0, time.Time{}, fmt.Errorf("open executable %q: %w", path, err)
	}
	defer f.Close()
	fi, err := f.Stat()
	if err != nil {
		return "", 0, time.Time{}, fmt.Errorf("stat executable %q: %w", path, err)
	}
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", 0, time.Time{}, fmt.Errorf("hash executable %q: %w", path, err)
	}
	return hex.EncodeToString(h.Sum(nil)), fi.Size(), fi.ModTime(), nil
}
