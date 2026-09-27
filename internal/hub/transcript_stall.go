package hub

// transcript_stall.go: provider 自身の transcript（Codex の rollout JSONL 等）が
// 最後に伸びた時刻を追跡し、UI へ配る。
//
// なぜ PTY 出力（lastOutputAt）で代用しないか:
// Codex TUI は "Working (36m 09s)" のカウンタを毎秒再描画するため、モデルが 1 個の
// 出力を延々と吐き続けている間も PTY 出力は途切れない。lastOutputAt は更新され続け、
// セッションは running のまま「順調に見える」。transcript はターンが進んだときだけ
// 伸びるので、伸びていない時間がそのまま「同じ応答を生成し続けている時間」になる。
//
// 由来: docs/local/bugfix_codex-long-silence-not-surfaced_2026-08-19.md
// 2026-08-19、Codex セッションがツール引数の生成中に同一文字列の反復（縮退ループ）へ
// 落ち、255,395 文字を生成して出力トークン上限で切断された。その 38 分間 rollout は
// 1 行も伸びなかったが、UI には 5 分固定閾値の「⚠ 長時間処理中」しか出ず、重いだけの
// ターンと区別できなかった。
//
// transcript の中身は読まない。サイズが増えたかどうかだけを見る。transcript には
// プロンプト・ソース断片・資格情報が入りうるため、agentLogLocation と同じく
// 「パスとメタデータまで」に留める。
//
// サブエージェント（Task 等）を持つ provider ではこの前提が崩れる。Claude Code は
// サブエージェント実行中、親 transcript には一切書かず、出力は
// <sessionDir>/subagents/agent-<id>.jsonl 側へ流れる。親だけを見ていると、
// サブエージェントが正常に働いている時間がそのまま停滞として積算される
// （実測: 33 分の実行に対し親 transcript が伸びたのは 4 行だけだった）。
// 由来: docs/local/bugfix_subagent-run-reported-as-stall_2026-09-14.md
//
// そのため親 transcript のサイズに加えて、サブエージェントディレクトリの直下
// エントリの最新 mtime も「ターンが進んだ」証拠として扱う。ここでも中身は
// 読まない・ファイル名も保持しない。provider ごとの分岐は subagentDirForTranscript
// 1 か所に閉じてある。

import (
	"os"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

// transcriptCheckRequest は 1 セッションぶんの transcript 検査要求。
// path が空なら「パス解決から」、非空なら「その path を stat するだけ」。
type transcriptCheckRequest struct {
	id   int
	path string
}

// collectTranscriptChecksLocked は running セッションの中から、この tick で
// transcript を見に行くべきものを集める。**呼び出し側は sessionsMu を保持していること。**
// ファイル I/O はここでは行わない（ロック下で stat / ReadDir しない）。
func (s *Server) collectTranscriptChecksLocked(now time.Time) []transcriptCheckRequest {
	var reqs []transcriptCheckRequest
	for id, ses := range s.sessions {
		// running 以外は「応答を生成し続けている」状態ではないので追跡しない。
		if ses.State != "running" {
			continue
		}
		if ses.transcriptPath == "" {
			// 未解決。解決はディレクトリ走査を伴うので間隔を長く取る。
			if !ses.transcriptResolvedAt.IsZero() && now.Sub(ses.transcriptResolvedAt) < transcriptResolveAfter {
				continue
			}
			ses.transcriptResolvedAt = now
			reqs = append(reqs, transcriptCheckRequest{id: id})
			continue
		}
		if !ses.transcriptStatAt.IsZero() && now.Sub(ses.transcriptStatAt) < transcriptStatAfter {
			continue
		}
		ses.transcriptStatAt = now
		reqs = append(reqs, transcriptCheckRequest{id: id, path: ses.transcriptPath})
	}
	return reqs
}

// subagentDirForTranscript は transcript パスからサブエージェントディレクトリを導く。
// 該当しない場合（拡張子が違う・claude 以外等）は空文字を返し、呼び出し側は何もしない。
//
// **provider ごとの分岐はこの関数 1 か所に閉じる。** 他の CLI がサブエージェントの
// ファイルを書くようになったら、ここに分岐を足す（CLAUDE.md 設計原則索引の
// 「残量ソースは 1 本の表で持つ」と同じ考え方だが、現時点で該当するのは claude
// だけなので表は作らず if 1 本で済ませてある）。
//
// claude の transcript は <sessionDir>.jsonl なので、拡張子を落とした
// <sessionDir>/subagents がサブエージェントの出力先（internal/hub/workflow_task_detail.go
// の transcriptPath := sessionDir + ".jsonl" と同じ対応関係の逆向き）。
func subagentDirForTranscript(path string) string {
	const claudeExt = ".jsonl"
	if !strings.HasSuffix(path, claudeExt) {
		return ""
	}
	sessionDir := strings.TrimSuffix(path, claudeExt)
	if sessionDir == "" {
		return ""
	}
	return filepath.Join(sessionDir, "subagents")
}

// latestSubagentMTime はサブエージェントディレクトリ直下のエントリのうち最新の
// mtime を返す。ディレクトリが無い・読めない・該当しない provider の場合はゼロ値
// （存在しないのが通常状態なのでエラーはログに出さない）。中身は読まず、
// ファイル名も保持しない。
func latestSubagentMTime(transcriptPath string) time.Time {
	dir := subagentDirForTranscript(transcriptPath)
	if dir == "" {
		return time.Time{}
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		return time.Time{}
	}
	var latest time.Time
	for _, entry := range entries {
		info, err := entry.Info()
		if err != nil {
			continue
		}
		if mt := info.ModTime(); mt.After(latest) {
			latest = mt
		}
	}
	return latest
}

// queueTranscriptChecks は集めた要求を 1 本の goroutine で順に処理する。
// stat 自体は軽いが、パス解決は 3 日ぶんのディレクトリ走査と候補ファイルの先頭行
// 読み込みを伴うので、状態ティッカーの経路から切り離す。
func (s *Server) queueTranscriptChecks(reqs []transcriptCheckRequest) {
	if len(reqs) == 0 {
		return
	}
	go func() {
		for _, req := range reqs {
			path := req.path
			if path == "" {
				loc := s.agentLogForSession(req.id)
				if !loc.Available || loc.Path == "" {
					continue
				}
				path = loc.Path
			}
			info, err := os.Stat(path)
			if err != nil {
				continue
			}
			// claude は transcript ファイルを特定できないときプロジェクトディレクトリを
			// 返す。ディレクトリのサイズは中身の増減を表さないので追跡対象にしない。
			if info.IsDir() {
				continue
			}
			subagentAt := latestSubagentMTime(path)
			s.applyTranscriptStat(req.id, path, info.Size(), subagentAt)
		}
	}()
}

// applyTranscriptStat は観測結果を書き戻し、transcript が伸びているか
// サブエージェントディレクトリが動いていれば TranscriptGrewAt を進めて
// session_update を broadcast する。
//
// **伸びていない間は何も送らない。** 停滞中は毎秒 broadcast する価値が無く、UI 側は
// TranscriptGrewAt からの経過を自分で 1Hz 計算できる（カードの応答経過と同じ方式）。
func (s *Server) applyTranscriptStat(id int, path string, size int64, subagentAt time.Time) {
	now := time.Now()
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	// 初回観測は「今伸びた」として扱う。それ以前にいつ伸びていたかは分からないので、
	// 停滞を過大に見積もらない側へ倒す。
	first := ses.transcriptPath == "" || ses.TranscriptGrewAt == ""
	ses.transcriptPath = path
	ses.transcriptStatAt = now
	subagentGrew := subagentAt.After(ses.transcriptSubagentAt)
	grew := first || size > ses.transcriptSize || subagentGrew
	ses.transcriptSize = size
	if subagentGrew {
		ses.transcriptSubagentAt = subagentAt
	}
	if !grew {
		s.sessionsMu.Unlock()
		return
	}
	ses.TranscriptGrewAt = now.Format(time.RFC3339)
	msg := proto.Message{
		Type: "session_update", SessionID: id,
		Provider: ses.Provider, Display: ses.Display, CWD: ses.CWD, Branch: ses.Branch,
		Label: ses.Label, Model: ses.Model, Route: ses.Route, State: ses.State,
		OutputIdle: ses.Activity.OutputIdle, WorkflowActive: ses.Activity.WorkflowActive,
		AwaitingUser: ses.Activity.AwaitingUser, AwaitingApproval: ses.Activity.AwaitingApproval,
		Activity: activityMessage(ses.Activity), LastOutputAt: ses.LastOutputAt,
		TranscriptGrewAt: ses.TranscriptGrewAt,
	}
	s.sessionsMu.Unlock()
	s.broadcast(msg)
}

// resetTranscriptTrackingLocked は新しいターンの開始時に停滞追跡を初期化する。
// **呼び出し側は sessionsMu を保持していること。**
//
// これが無いと、前のターンが終わってから次の入力までの待ち時間がそのまま停滞として
// 積算され、新しいターンの開始直後に「30 分停滞」と表示される。
func resetTranscriptTrackingLocked(ses *session, now time.Time) {
	ses.TranscriptGrewAt = now.Format(time.RFC3339)
	ses.transcriptStatAt = time.Time{}
	ses.transcriptSubagentAt = time.Time{}
}
