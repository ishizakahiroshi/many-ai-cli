package wrapper

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// opencodeLockStaleAfter は lock ファイルの PID が読めない／壊れているときの
// 最終手段としての mtime フォールバック。通常は PID 生存確認で奪取可否を決める。
const opencodeLockStaleAfter = 30 * time.Minute

// OpenCodeConfigFileName / OpenCodeLockSuffix は、このパッケージが利用者の作業
// フォルダへ書く生成物の名前。doctor の置き去り検査が同じ名前を探すために公開する。
// 検査側で名前を二重定義すると、片方だけ変わったときに検査が黙って空振りする。
const (
	OpenCodeConfigFileName = "opencode.json"
	OpenCodeLockSuffix     = ".many-ai-cli.lock"
)

// OpenCodeConfigPermissionKey は prepareOpenCodeConfig が opencode.json の
// permission へ差し込むキー。検査側はこのキーの有無で「many-ai-cli が書いた設定か」を
// 判定する（利用者自身の opencode.json と区別するため）。
const OpenCodeConfigPermissionKey = "*"

// OpenCodeLockHeldByLiveProcess は lock ファイルの保持プロセスが生存しているかを返す。
// 稼働中セッションの正常な生成物を、doctor が置き去りとして報告しないための判定。
// 生存判定そのもの（processAlive）は OS 別実装なので、検査側へ複製せずここで包む。
func OpenCodeLockHeldByLiveProcess(lockPath string) bool {
	st, ok := readOpenCodeLock(lockPath)
	if !ok {
		// PID が読めない壊れたロックは mtime で判断する（奪取条件と同じしきい値）。
		info, err := os.Stat(lockPath)
		if err != nil {
			return false
		}
		return time.Since(info.ModTime()) <= opencodeLockStaleAfter
	}
	return processAlive(st.PID)
}

// openCodeLockState は lock ファイルの中身。保持 PID に加えて、前セッションが
// cleanup を通らずに死んだ場合に opencode.json を巻き戻すための情報を持つ。
type openCodeLockState struct {
	PID      int               `json:"pid"`
	Rollback *openCodeRollback `json:"rollback,omitempty"`
}

// openCodeRollback は「書き換える前の姿」と「自分が書いた内容」の組。
// Rollback が nil のロックは「まだ opencode.json に触っていない」段階を表すので、
// そのロックを奪っても巻き戻しは要らない（nil と ConfigExisted=false は別物）。
// Written を持つのは、巻き戻す直前に現物と突き合わせるため。前セッションの死後に
// ユーザーや opencode 自身が書き換えた opencode.json を、こちらの都合で
// 消し戻さないようにする。
type openCodeRollback struct {
	ConfigExisted bool   `json:"config_existed"`
	Original      []byte `json:"original,omitempty"`
	Written       []byte `json:"written"`
}

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
//
// セッションが kill / クラッシュで defer を通らずに死ぬと cleanup が走らない。この場合
// 書き換え後の opencode.json が cwd に置き去りになり、次のセッションがそれを「オリジナル」
// として採用して cleanup で書き戻すため、permission の上書きがそのリポジトリへ永久に
// residue として残る（orchestration 子が書く "allow" を含む）。ロックに巻き戻し情報を
// 焼いておき、stale ロックを奪う時点で置き去りを回収してから次の書き換えに入る。
func prepareOpenCodeConfig(cwd string, permissionValue string, logger *slog.Logger) (cleanup func(), err error) {
	cfgPath := filepath.Join(cwd, OpenCodeConfigFileName)
	lockPath := cfgPath + OpenCodeLockSuffix

	lockFile, lockErr := os.OpenFile(lockPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if lockErr != nil {
		if os.IsExist(lockErr) {
			if stale, stolen := tryStealOpenCodeLock(lockPath); stolen {
				// 奪えた = 前セッションは死んでいる。この後の ReadFile より前に
				// 置き去りを回収する（順序が逆だと置き去りをオリジナルとして採用する）。
				recoverOrphanedOpenCodeConfig(cfgPath, stale, logger)
				lockFile, lockErr = os.OpenFile(lockPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
			}
		}
		if lockErr != nil {
			// スキップは黙って成功にしない（permission 上書きが効かない事実を呼び出し側へ伝える）。
			return func() {}, fmt.Errorf("opencode.json locked by another session (permission %q not applied): %w", permissionValue, lockErr)
		}
	}
	lockState := openCodeLockState{PID: os.Getpid()}
	lockBytes, lockMarshalErr := json.Marshal(lockState)
	if lockMarshalErr == nil {
		_, lockMarshalErr = lockFile.Write(lockBytes)
	}
	if lockMarshalErr != nil {
		_ = lockFile.Close()
		_ = os.Remove(lockPath)
		return func() {}, fmt.Errorf("write opencode lock pid: %w", lockMarshalErr)
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
	perm[OpenCodeConfigPermissionKey] = permissionValue
	merged["permission"] = perm

	data, marshalErr := json.MarshalIndent(merged, "", "  ")
	if marshalErr != nil {
		cleanupLock()
		return func() {}, marshalErr
	}

	// 巻き戻し情報は opencode.json を書き換える「前」にロックへ焼く。逆順にすると
	// 書き換え直後に kill されたセッションの置き去りを次回に検知できない。
	lockState.Rollback = &openCodeRollback{ConfigExisted: existed, Original: orig, Written: data}
	armed, armErr := json.Marshal(lockState)
	if armErr == nil {
		armErr = os.WriteFile(lockPath, armed, 0o600)
	}
	if armErr != nil {
		cleanupLock()
		return func() {}, fmt.Errorf("arm opencode rollback state: %w", armErr)
	}

	if writeErr := os.WriteFile(cfgPath, data, 0o600); writeErr != nil {
		cleanupLock()
		return func() {}, writeErr
	}

	cleanup = func() {
		defer cleanupLock()
		current, readErr := os.ReadFile(cfgPath)
		if readErr != nil {
			// The user may have removed the file while the session was running.
			// Do not recreate it or turn a concurrent edit into a rollback.
			if !errors.Is(readErr, os.ErrNotExist) {
				logOpenCode(logger, slog.LevelWarn,
					"failed to inspect opencode.json during cleanup; leaving it as-is",
					"path", cfgPath, "err", readErr)
			}
			return
		}
		if !bytes.Equal(current, data) {
			// The file was edited after this session wrote it. Restoring orig here
			// would discard the user's edit; orphan recovery applies the same guard.
			logOpenCode(logger, slog.LevelWarn,
				"opencode.json was modified during the session; leaving it as-is",
				"path", cfgPath)
			return
		}
		if existed {
			// cfgPath は起動時に読み込んだのと同じ session cwd + "opencode.json" で、
			// orig もそのファイルから読み込んだ元の内容。ユーザーが自分のマシンで自分の
			// cwd を指定しているため path traversal のリスクは無い。
			if err := os.WriteFile(cfgPath, orig, 0o600); err != nil { // #nosec G703 -- cfgPath は起動時に読んだのと同じ session cwd + opencode.json、orig は同ファイル由来
				logOpenCode(logger, slog.LevelWarn, "failed to restore opencode.json during cleanup",
					"path", cfgPath, "err", err)
			}
		} else {
			if err := os.Remove(cfgPath); err != nil && !errors.Is(err, os.ErrNotExist) {
				logOpenCode(logger, slog.LevelWarn, "failed to remove generated opencode.json during cleanup",
					"path", cfgPath, "err", err)
			}
		}
	}
	return cleanup, nil
}

// recoverOrphanedOpenCodeConfig は奪取した stale ロックに残っていた巻き戻し情報で、
// 前セッションが置き去りにした opencode.json を書き換え前の姿へ戻す。
// 現物が「前セッションが書いた内容」と 1 バイトでも違う場合は、その後に
// ユーザーか opencode 自身が書き換えたとみなして触らない。
func recoverOrphanedOpenCodeConfig(cfgPath string, st openCodeLockState, logger *slog.Logger) {
	if st.Rollback == nil {
		return
	}
	current, readErr := os.ReadFile(cfgPath)
	if readErr != nil {
		// 既に消えている（＝回収済み）か読めない。どちらも触らない。
		return
	}
	if !bytes.Equal(current, st.Rollback.Written) {
		// 置き去りの permission がまだ効いている可能性があるので黙って見送らない。
		logOpenCode(logger, slog.LevelWarn,
			"opencode.json left behind by a session that did not clean up, but it was modified afterwards; leaving it as-is",
			"path", cfgPath, "stale_pid", st.PID)
		return
	}
	if st.Rollback.ConfigExisted {
		_ = os.WriteFile(cfgPath, st.Rollback.Original, 0o600) // #nosec G703 -- cfgPath は奪ったロックと同じ session cwd + opencode.json、Original は同ファイル由来
	} else if rmErr := os.Remove(cfgPath); rmErr != nil {
		logOpenCode(logger, slog.LevelWarn, "failed to remove orphaned opencode.json",
			"path", cfgPath, "stale_pid", st.PID, "err", rmErr)
		return
	}
	logOpenCode(logger, slog.LevelInfo, "recovered opencode.json left behind by a session that did not clean up",
		"path", cfgPath, "stale_pid", st.PID, "restored", st.Rollback.ConfigExisted)
}

func logOpenCode(logger *slog.Logger, level slog.Level, msg string, args ...any) {
	if logger == nil {
		return
	}
	logger.Log(context.Background(), level, msg, args...)
}

// readOpenCodeLock は lock ファイルを読み、PID と巻き戻し情報を返す。
// v0.6.0 以前が書いた「PID だけの平文」ロックも読めるようにしておく
// （旧バイナリのセッションが走っている最中に新バイナリが起動しても、
// 生存 PID を見落として横取りしないため）。
func readOpenCodeLock(lockPath string) (openCodeLockState, bool) {
	data, err := os.ReadFile(lockPath)
	if err != nil {
		return openCodeLockState{}, false
	}
	var st openCodeLockState
	if json.Unmarshal(data, &st) == nil && st.PID > 0 {
		return st, true
	}
	if pid, convErr := strconv.Atoi(strings.TrimSpace(string(data))); convErr == nil && pid > 0 {
		return openCodeLockState{PID: pid}, true
	}
	return openCodeLockState{}, false
}

// tryStealOpenCodeLock はロック保持プロセスが死んでいる、または PID 不明で mtime が
// stale のときだけロックを削除して true を返す。生存中 PID には触れない。
// 奪取できた場合は、そのロックに残っていた巻き戻し情報も返す。
func tryStealOpenCodeLock(lockPath string) (openCodeLockState, bool) {
	if st, ok := readOpenCodeLock(lockPath); ok {
		if processAlive(st.PID) {
			return openCodeLockState{}, false
		}
		_ = os.Remove(lockPath)
		return st, true
	}
	// PID が読めない壊れたロック: mtime フォールバック
	info, statErr := os.Stat(lockPath)
	if statErr != nil {
		return openCodeLockState{}, false
	}
	if time.Since(info.ModTime()) <= opencodeLockStaleAfter {
		return openCodeLockState{}, false
	}
	_ = os.Remove(lockPath)
	return openCodeLockState{}, true
}
