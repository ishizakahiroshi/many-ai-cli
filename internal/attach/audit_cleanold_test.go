package attach

import (
	"os"
	"testing"
	"time"
)

// TestCleanOldNonPositiveRetentionIsNoOp は retentionDays<=0 のとき CleanOld が
// 一切ファイルを削除しないこと（config.go の「0 で日数ベースの削除を無効化」と整合）を確認する。
// 過去にガードが無く、retentionDays==0 だと cutoff=現在時刻となり全添付を削除し得た回帰。
func TestCleanOldNonPositiveRetentionIsNoOp(t *testing.T) {
	for _, retentionDays := range []int{0, -1, -7} {
		retentionDays := retentionDays
		t.Run("", func(t *testing.T) {
			baseDir := t.TempDir()

			// 古い mtime のファイル（cutoff=now ならガード無しで消えてしまう対象）
			oldPath, _, err := Save(baseDir, 1, "claude", []byte("old"), "")
			if err != nil {
				t.Fatalf("Save old: %v", err)
			}
			old := time.Now().Add(-365 * 24 * time.Hour)
			if err := os.Chtimes(oldPath, old, old); err != nil {
				t.Fatalf("Chtimes: %v", err)
			}

			// 直近のファイル
			recentPath, _, err := Save(baseDir, 2, "codex", []byte("recent"), "")
			if err != nil {
				t.Fatalf("Save recent: %v", err)
			}

			if err := CleanOld(baseDir, retentionDays); err != nil {
				t.Fatalf("CleanOld(retentionDays=%d): %v", retentionDays, err)
			}

			// retentionDays<=0 では何も削除されないこと
			if _, err := os.Stat(oldPath); err != nil {
				t.Errorf("retentionDays=%d: old file %s should NOT be deleted: %v", retentionDays, oldPath, err)
			}
			if _, err := os.Stat(recentPath); err != nil {
				t.Errorf("retentionDays=%d: recent file %s should NOT be deleted: %v", retentionDays, recentPath, err)
			}
		})
	}
}
