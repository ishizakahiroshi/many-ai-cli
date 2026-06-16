package wrapper

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
)

// prepareOpenCodeConfig は cwd/opencode.json に {"permission":{"*":"ask"}} をマージして
// 書き込み、元のファイルを復元するクリーンアップ関数を返す。
// opencode.json が存在しない場合はファイルを新規作成し、クリーンアップ時に削除する。
func prepareOpenCodeConfig(cwd string) (cleanup func(), err error) {
	cfgPath := filepath.Join(cwd, "opencode.json")

	orig, readErr := os.ReadFile(cfgPath)
	existed := readErr == nil

	var merged map[string]any
	if existed {
		if jsonErr := json.Unmarshal(orig, &merged); jsonErr != nil {
			return func() {}, jsonErr
		}
	} else {
		if !errors.Is(readErr, os.ErrNotExist) {
			return func() {}, readErr
		}
		merged = map[string]any{}
	}

	// permission フィールドをマージ。既存エントリを保持しつつ "*": "ask" を追加する。
	perm, _ := merged["permission"].(map[string]any)
	if perm == nil {
		perm = map[string]any{}
	}
	perm["*"] = "ask"
	merged["permission"] = perm

	data, marshalErr := json.MarshalIndent(merged, "", "  ")
	if marshalErr != nil {
		return func() {}, marshalErr
	}
	if writeErr := os.WriteFile(cfgPath, data, 0o600); writeErr != nil {
		return func() {}, writeErr
	}

	cleanup = func() {
		if existed {
			_ = os.WriteFile(cfgPath, orig, 0o600)
		} else {
			_ = os.Remove(cfgPath)
		}
	}
	return cleanup, nil
}
