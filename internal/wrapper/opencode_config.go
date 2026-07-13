package wrapper

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// opencodeLockStaleAfter は lock ファイルの PID が読めない／壊れているときの
// 最終手段としての mtime フォールバック。通常は PID 生存確認で奪取可否を決める。
const opencodeLockStaleAfter = 30 * time.Minute

// prepareOpenCodeConfig は cwd/opencode.json に {"permission":{"*":<permissionValue>}} を
// マージして書き込み、元のファイルを復元するクリーンアップ関数を返す。
// permissionValue は通常セッションでは "ask"（承認を Hub UI に出す）、orchestration 子など
// 承認バイパスセッションでは "allow"（全許可）を渡す。
// opencode.json が存在しない場合はファイルを新規作成し、クリーンアップ時に削除する。
//
// 同一 cwd で 2 セッションが同時に起動すると、後発が先発の書き換え後を「オリジナル」として
// 保存し、両方 defer cleanup が動いてもファイルが真のオリジナルに戻らない競合があった。
// 排他ロックファイル (`opencode.json.many-ai-cli.lock`) を `O_EXCL` で作成し、中身に
// 保持 PID を書く。既存ロックの PID が生存中なら上書きをスキップしてエラーを返す
// （呼び出し側で Warn）。PID が死んでいる、または読めず mtime が stale のときだけ奪取する。
func prepareOpenCodeConfig(cwd string, permissionValue string) (cleanup func(), err error) {
	cfgPath := filepath.Join(cwd, "opencode.json")
	lockPath := cfgPath + ".many-ai-cli.lock"

	lockFile, lockErr := os.OpenFile(lockPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if lockErr != nil {
		if os.IsExist(lockErr) && tryStealOpenCodeLock(lockPath) {
			lockFile, lockErr = os.OpenFile(lockPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
		}
		if lockErr != nil {
			// スキップは黙って成功にしない（permission 上書きが効かない事実を呼び出し側へ伝える）。
			return func() {}, fmt.Errorf("opencode.json locked by another session (permission %q not applied): %w", permissionValue, lockErr)
		}
	}
	if _, wErr := fmt.Fprintf(lockFile, "%d\n", os.Getpid()); wErr != nil {
		_ = lockFile.Close()
		_ = os.Remove(lockPath)
		return func() {}, fmt.Errorf("write opencode lock pid: %w", wErr)
	}
	_ = lockFile.Close()

	// 以降で失敗したらロックファイルは必ず消してから return する（自身が取得した
	// ロックのみ・stale 奪取のケースも含めて cleanup 関数側で最終削除する）。
	cleanupLock := func() { _ = os.Remove(lockPath) }

	orig, readErr := os.ReadFile(cfgPath)
	existed := readErr == nil

	var merged map[string]any
	if existed {
		if jsonErr := json.Unmarshal(orig, &merged); jsonErr != nil {
			cleanupLock()
			return func() {}, jsonErr
		}
	} else {
		if !errors.Is(readErr, os.ErrNotExist) {
			cleanupLock()
			return func() {}, readErr
		}
		merged = map[string]any{}
	}

	// permission フィールドをマージ。既存エントリを保持しつつ "*" を追加する。
	perm, _ := merged["permission"].(map[string]any)
	if perm == nil {
		perm = map[string]any{}
	}
	perm["*"] = permissionValue
	merged["permission"] = perm

	data, marshalErr := json.MarshalIndent(merged, "", "  ")
	if marshalErr != nil {
		cleanupLock()
		return func() {}, marshalErr
	}
	if writeErr := os.WriteFile(cfgPath, data, 0o600); writeErr != nil {
		cleanupLock()
		return func() {}, writeErr
	}

	cleanup = func() {
		if existed {
			// cfgPath は起動時に読み込んだのと同じ session cwd + "opencode.json" で、
			// orig もそのファイルから読み込んだ元の内容。ユーザーが自分のマシンで自分の
			// cwd を指定しているため path traversal のリスクは無い。
			_ = os.WriteFile(cfgPath, orig, 0o600) // #nosec G703 -- cfgPath は起動時に読んだのと同じ session cwd + opencode.json、orig は同ファイル由来
		} else {
			_ = os.Remove(cfgPath)
		}
		cleanupLock()
	}
	return cleanup, nil
}

// tryStealOpenCodeLock はロック保持プロセスが死んでいる、または PID 不明で mtime が
// stale のときだけロックを削除して true を返す。生存中 PID には触れない。
func tryStealOpenCodeLock(lockPath string) bool {
	data, err := os.ReadFile(lockPath)
	if err == nil {
		pidStr := strings.TrimSpace(string(data))
		if pid, convErr := strconv.Atoi(pidStr); convErr == nil && pid > 0 {
			if processAlive(pid) {
				return false
			}
			_ = os.Remove(lockPath)
			return true
		}
	}
	// PID が読めない壊れたロック: mtime フォールバック
	info, statErr := os.Stat(lockPath)
	if statErr != nil {
		return false
	}
	if time.Since(info.ModTime()) <= opencodeLockStaleAfter {
		return false
	}
	_ = os.Remove(lockPath)
	return true
}
