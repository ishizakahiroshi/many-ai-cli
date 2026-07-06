package wrapper

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"time"
)

// opencodeLockStaleAfter は opencode.json.many-ai-cli.lock を stale と判断する経過時間。
// 通常の opencode セッションは数分〜数十分で終わる想定なので、30 分以上残っている
// ロックは前回の SIGKILL/クラッシュ由来と判断して自分が上書きする。
const opencodeLockStaleAfter = 30 * time.Minute

// prepareOpenCodeConfig は cwd/opencode.json に {"permission":{"*":<permissionValue>}} を
// マージして書き込み、元のファイルを復元するクリーンアップ関数を返す。
// permissionValue は通常セッションでは "ask"（承認を Hub UI に出す）、orchestration 子など
// 承認バイパスセッションでは "allow"（全許可）を渡す。
// opencode.json が存在しない場合はファイルを新規作成し、クリーンアップ時に削除する。
//
// PTYW-1: 同一 cwd で 2 セッションが同時に起動すると、後発が先発の書き換え後を
// 「オリジナル」として保存し、両方 defer cleanup が動いてもファイルが真のオリジナルに
// 戻らない競合があった。排他ロックファイル (`opencode.json.many-ai-cli.lock`) を
// `O_EXCL` で作成し、既存ロックがあれば permission 上書きをスキップして「他セッション
// の設定に相乗り」する動作にする（安全側フォールバック）。ロック取得できなかったときは
// cleanup=nop で戻り、他セッション終了時に相手が復元する。
// なお SIGKILL/電源断で defer 未実行のまま残った古いロック (30 分以上前) は stale と
// 判定して奪う。恒久上書きの根本策 (シグナルハンドラでの cleanup) は別途進言事項として扱う。
func prepareOpenCodeConfig(cwd string, permissionValue string) (cleanup func(), err error) {
	cfgPath := filepath.Join(cwd, "opencode.json")
	lockPath := cfgPath + ".many-ai-cli.lock"

	lockFile, lockErr := os.OpenFile(lockPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if lockErr != nil {
		// 既存ロックが stale なら奪う。それでも駄目なら nop cleanup で相乗り。
		if os.IsExist(lockErr) {
			if info, statErr := os.Stat(lockPath); statErr == nil && time.Since(info.ModTime()) > opencodeLockStaleAfter {
				_ = os.Remove(lockPath)
				lockFile, lockErr = os.OpenFile(lockPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
			}
		}
		if lockErr != nil {
			return func() {}, nil
		}
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
