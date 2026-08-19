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

import (
	"os"
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
			s.applyTranscriptStat(req.id, path, info.Size())
		}
	}()
}

// applyTranscriptStat は観測結果を書き戻し、transcript が伸びていれば
// TranscriptGrewAt を進めて session_update を broadcast する。
//
// **伸びていない間は何も送らない。** 停滞中は毎秒 broadcast する価値が無く、UI 側は
// TranscriptGrewAt からの経過を自分で 1Hz 計算できる（カードの応答経過と同じ方式）。
func (s *Server) applyTranscriptStat(id int, path string, size int64) {
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
	grew := first || size > ses.transcriptSize
	ses.transcriptSize = size
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
}
