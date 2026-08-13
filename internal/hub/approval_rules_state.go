package hub

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/wrapper"
)

// approvalRuleStateFilename は「今どのファイルへ承認ルールを注入しているか」の台帳。
//
// 注入ブロックが消えるのは Hub の graceful shutdown（server.go の shutdown_wait）と
// 承認機能の無効化のときだけで、Hub がプロセスごと kill されると removeApprovalRules が
// 走らない。対象を覚えている s.approvalRuleTargets は in-memory なのでそこで失われ、
// 利用者のリポジトリの AGENTS.md に注入ブロックが残ったままになる。実際に PlainSheet と
// trusted-context-mcp では、その置き去りが commit されて公開リポジトリに載っていた。
// 同じ集合をディスクにも持たせ、次回起動時に回収できるようにする。
const approvalRuleStateFilename = "approval-rule-targets.json"

const approvalRuleStateVersion = 1

type approvalRuleStateEntry struct {
	Path      string           `json:"path"`
	Providers []string         `json:"providers"`
	Mode      approvalRuleMode `json:"mode"`
}

type approvalRuleStateFile struct {
	Version int                      `json:"version"`
	Targets []approvalRuleStateEntry `json:"targets"`
}

func approvalRuleStatePath() (string, error) {
	dir, err := config.Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, approvalRuleStateFilename), nil
}

// persistApprovalTargetsLocked は台帳をディスクへ書き出す。
// s.approvalRulesMu を保持したまま呼ぶこと（in-memory とディスクがずれると
// 回収対象を取りこぼす／既に外した対象をもう一度外そうとする）。
func (s *Server) persistApprovalTargetsLocked() {
	path, err := approvalRuleStatePath()
	if err != nil {
		s.logger.Warn("approval rule state path unavailable", "err", err)
		return
	}
	state := approvalRuleStateFile{Version: approvalRuleStateVersion}
	for _, target := range s.approvalRuleTargets {
		state.Targets = append(state.Targets, approvalRuleStateEntry{
			Path:      target.Path,
			Providers: target.Providers,
			Mode:      target.Mode,
		})
	}
	// map の走査順で毎回並びが変わると差分が読めないため固定する。
	sort.Slice(state.Targets, func(i, j int) bool {
		return approvalTargetKey(state.Targets[i].Path) < approvalTargetKey(state.Targets[j].Path)
	})
	if len(state.Targets) == 0 {
		if rmErr := os.Remove(path); rmErr != nil && !errors.Is(rmErr, os.ErrNotExist) {
			s.logger.Warn("remove approval rule state failed", "path", path, "err", rmErr)
		}
		return
	}
	if writeErr := writeApprovalRuleState(path, state); writeErr != nil {
		s.logger.Warn("persist approval rule state failed", "path", path, "err", writeErr)
	}
}

// recoverOrphanedApprovalRules は前回の Hub が正常終了せずに残した注入ブロックを
// 起動時に回収する。起動直後はまだ 1 セッションも接続していないので、台帳に載って
// いる対象はすべて前回分の置き去りとして扱ってよい。再接続してくるセッションには
// wrapper_loop 側の injectApprovalRules が入れ直す。
// 外せなかった対象は台帳に残し、次の起動でもう一度試す。
func (s *Server) recoverOrphanedApprovalRules() {
	path, err := approvalRuleStatePath()
	if err != nil {
		s.logger.Warn("approval rule state path unavailable", "err", err)
		return
	}
	state, ok := s.readApprovalRuleState(path)
	if !ok || len(state.Targets) == 0 {
		return
	}

	var failed []approvalRuleStateEntry
	for _, entry := range state.Targets {
		target := approvalRuleTarget{Path: entry.Path, Providers: entry.Providers, Mode: entry.Mode}
		if rmErr := wrapper.RemoveRules(target.wrapperProvider(), target.Path); rmErr != nil {
			s.logger.Warn("failed to recover an approval rule block left by the previous Hub",
				"path", entry.Path, "err", rmErr)
			failed = append(failed, entry)
			continue
		}
	}
	s.logger.Info("recovered approval rule blocks left behind by a Hub that did not shut down cleanly",
		"recovered", len(state.Targets)-len(failed), "failed", len(failed))

	if len(failed) == 0 {
		if rmErr := os.Remove(path); rmErr != nil && !errors.Is(rmErr, os.ErrNotExist) {
			s.logger.Warn("remove approval rule state failed", "path", path, "err", rmErr)
		}
		return
	}
	if writeErr := writeApprovalRuleState(path, approvalRuleStateFile{
		Version: approvalRuleStateVersion,
		Targets: failed,
	}); writeErr != nil {
		s.logger.Warn("persist approval rule state failed", "path", path, "err", writeErr)
	}
}

// readApprovalRuleState は台帳を読む。壊れていた場合は ok=false を返し、
// 呼び出し側は何もしない（読めない台帳を根拠に利用者のファイルを書き換えない）。
func (s *Server) readApprovalRuleState(path string) (approvalRuleStateFile, bool) {
	data, err := os.ReadFile(path) // #nosec G304 -- config.Dir() 配下の固定ファイル名（外部入力なし）
	if err != nil {
		if !errors.Is(err, os.ErrNotExist) {
			s.logger.Warn("read approval rule state failed", "path", path, "err", err)
		}
		return approvalRuleStateFile{}, false
	}
	var state approvalRuleStateFile
	if jsonErr := json.Unmarshal(data, &state); jsonErr != nil {
		s.logger.Warn("approval rule state is corrupt; leaving instruction files untouched",
			"path", path, "err", jsonErr)
		return approvalRuleStateFile{}, false
	}
	return state, true
}

// writeApprovalRuleState は runfile.go と同じ「temp へ書いて rename」で置き換える。
func writeApprovalRuleState(path string, state approvalRuleStateFile) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("mkdir approval rule state dir: %w", err)
	}
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal approval rule state: %w", err)
	}
	tmp, err := os.CreateTemp(dir, "approval-rule-targets-*.json.tmp")
	if err != nil {
		return fmt.Errorf("create temp approval rule state: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // no-op after successful Rename

	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write temp approval rule state: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("sync temp approval rule state: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close temp approval rule state: %w", err)
	}
	if err := os.Chmod(tmpName, 0o600); err != nil {
		return fmt.Errorf("chmod temp approval rule state: %w", err)
	}
	return os.Rename(tmpName, path)
}
