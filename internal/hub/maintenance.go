package hub

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/attach"
	"many-ai-cli/internal/sessionlog"
)

// recoverTranscripts は logs/sessions/*.jsonl のうち、対応する .txt が
// 無い、もしくは .jsonl より古いものを遡って WriteTranscriptFile で生成する。
// Hub クラッシュ等で wrapperMessageLoop の終了処理を通れず .txt が作成
// されなかった場合の救済（通常運用では Close 直後に .txt が生成される）。
func (s *Server) recoverTranscripts() {
	s.logMaintenanceMu.Lock()
	defer s.logMaintenanceMu.Unlock()

	dir := filepath.Join(s.cfg.Hub.LogDir, "sessions")
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".jsonl") {
			continue
		}
		jsonlPath := filepath.Join(dir, e.Name())
		txtPath := sessionlog.TranscriptPath(jsonlPath)
		jsonlInfo, statErr := os.Stat(jsonlPath)
		if statErr != nil {
			continue
		}
		if txtInfo, err := os.Stat(txtPath); err == nil {
			if !txtInfo.ModTime().Before(jsonlInfo.ModTime()) {
				continue
			}
		}
		if err := sessionlog.WriteTranscriptFile(jsonlPath, txtPath); err != nil {
			s.logger.Warn("transcript recovery failed", "path", txtPath, "err", err)
		}
	}
}

// cleanSpawnLogs removes wrap-process spawn logs (logs/spawn/*.log) older than 7 days.
// These files capture stdout/stderr of each spawned wrap process for trouble-shooting
// (especially GUI-launched Hub where stderr is otherwise lost). One file per spawn is
// kept short-term to debug startup failures; trimming on Hub start prevents accumulation.
func (s *Server) cleanSpawnLogs() {
	dir := filepath.Join(s.cfg.Hub.LogDir, "spawn")
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	cutoff := time.Now().Add(-7 * 24 * time.Hour)
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		if info.ModTime().Before(cutoff) {
			_ = os.Remove(filepath.Join(dir, e.Name()))
		}
	}
}

// cleanSessionLogs removes session log triplets (.log / .jsonl / .txt) in
// logs/sessions/ that are older than cfg.Log.SessionRetentionDays days.
// A retention of 0 disables cleanup; negative values are treated as 0.
func (s *Server) cleanSessionLogs() {
	s.logMaintenanceMu.Lock()
	defer s.logMaintenanceMu.Unlock()

	s.cfgMu.Lock()
	days := s.cfg.Log.SessionRetentionDays
	s.cfgMu.Unlock()
	if days <= 0 {
		return
	}
	dir := filepath.Join(s.cfg.Hub.LogDir, "sessions")
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	cutoff := time.Now().Add(-time.Duration(days) * 24 * time.Hour)
	if s.sessionStore != nil {
		if err := s.sessionStore.PruneOlderThan(cutoff); err != nil {
			s.logger.Warn("sqlite session store cleanup failed", "err", err)
		}
	}
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		if info.ModTime().Before(cutoff) {
			_ = os.Remove(filepath.Join(dir, e.Name()))
		}
	}
}

// maintenanceInterval は常駐中に掃除タスク（添付・セッションログ・spawn ログ）を
// 周期実行する間隔。起動時に一度走るだけだと Hub を再起動しない常駐運用
// （VPS の Docker 等）で保持日数・容量上限を超えても削除されないため、定期的に回す。
const maintenanceInterval = 6 * time.Hour

// maintenanceLoop は maintenanceInterval ごとに掃除タスクを再実行する。
// 起動直後の初回実行は Run 側の safeGo で別途行うため、ここでは初回 tick を待ってから走る。
func (s *Server) maintenanceLoop(ctx context.Context) {
	t := time.NewTicker(maintenanceInterval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			s.cleanAttachments()
			s.cleanSpawnLogs()
			s.cleanSessionLogs()
		}
	}
}

// attachmentsDir は添付ファイルの保存先 ~/.many-ai-cli/attachments を返す。
// 4 箇所で散らばっていた os.UserHomeDir + filepath.Join のリテラル重複を集約する。
func attachmentsDir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".many-ai-cli", "attachments"), nil
}

// cleanAttachments removes attachment files older than the configured retention
// (cfg.Log.AttachmentRetentionDays) and, if a total-size cap is configured
// (cfg.Log.AttachmentMaxTotalMB), trims the oldest files until under that cap.
// Empty session directories are then pruned.
func (s *Server) cleanAttachments() {
	attachDir, err := attachmentsDir()
	if err != nil {
		return
	}
	s.cfgMu.Lock()
	retentionDays := s.cfg.Log.AttachmentRetentionDays
	maxTotalMB := s.cfg.Log.AttachmentMaxTotalMB
	s.cfgMu.Unlock()

	if retentionDays > 0 {
		if err := attach.CleanOld(attachDir, retentionDays); err != nil {
			s.logger.Warn("attach cleanup failed", "err", err)
		}
	}
	if maxTotalMB > 0 {
		if err := attach.EnforceTotalSize(attachDir, int64(maxTotalMB)*1024*1024); err != nil {
			s.logger.Warn("attach size enforcement failed", "err", err)
		}
	}
	entries, err := os.ReadDir(attachDir)
	if err != nil {
		return
	}
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		sub := filepath.Join(attachDir, e.Name())
		children, _ := os.ReadDir(sub)
		if len(children) == 0 {
			_ = os.Remove(sub)
		}
	}
}

// finalizeTranscript は JSONL パスからトランスクリプトファイルを生成する。
// wrapperMessageLoop と session_dismiss の 2 箇所で同一の transcript 生成コードが
// 重複していたため、ここに集約する。
func (s *Server) finalizeTranscript(id int, jsonlPath string) {
	if jsonlPath == "" {
		return
	}
	// .jsonl が無い（セッションログ無効など）場合は .txt を生成しない。
	// 無駄な警告ログを避けるため stat で存在確認する。
	if _, err := os.Stat(jsonlPath); err != nil {
		return
	}
	transcriptPath := sessionlog.TranscriptPath(jsonlPath)
	if err := sessionlog.WriteTranscriptFile(jsonlPath, transcriptPath); err != nil {
		s.logger.Warn("transcript generation failed", "session_id", id, "path", transcriptPath, "err", err)
	}
}
