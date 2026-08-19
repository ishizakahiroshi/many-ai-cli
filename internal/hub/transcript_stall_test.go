package hub

import (
	"testing"
	"time"
)

// TestCollectTranscriptChecksSkipsNonRunningSessions は running 以外を追跡対象に
// しないことを確認する。standby のセッションは応答を生成していないので、停滞を
// 測る意味が無い（測ると「次の入力を待っている時間」を停滞として数えてしまう）。
func TestCollectTranscriptChecksSkipsNonRunningSessions(t *testing.T) {
	s := newTestServer()
	running := registerTestSession(s, 1, "codex")
	running.transcriptPath = "rollout-running.jsonl"
	idle := registerTestSession(s, 2, "codex")
	idle.transcriptPath = "rollout-idle.jsonl"
	idle.State = "standby"

	s.sessionsMu.Lock()
	reqs := s.collectTranscriptChecksLocked(time.Now())
	s.sessionsMu.Unlock()

	if len(reqs) != 1 || reqs[0].id != 1 {
		t.Fatalf("collectTranscriptChecksLocked() = %+v, want exactly session 1", reqs)
	}
}

// TestCollectTranscriptChecksRespectsStatInterval は 200ms ごとに回る状態ティッカーが
// 毎 tick で stat を投げないことを確認する。
func TestCollectTranscriptChecksRespectsStatInterval(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	ses.transcriptPath = "rollout.jsonl"
	now := time.Now()

	ses.transcriptStatAt = now.Add(-transcriptStatAfter / 2)
	s.sessionsMu.Lock()
	reqs := s.collectTranscriptChecksLocked(now)
	s.sessionsMu.Unlock()
	if len(reqs) != 0 {
		t.Fatalf("collectTranscriptChecksLocked() = %+v within the stat interval, want none", reqs)
	}

	ses.transcriptStatAt = now.Add(-(transcriptStatAfter + time.Second))
	s.sessionsMu.Lock()
	reqs = s.collectTranscriptChecksLocked(now)
	s.sessionsMu.Unlock()
	if len(reqs) != 1 {
		t.Fatalf("collectTranscriptChecksLocked() = %+v after the stat interval, want 1", reqs)
	}
}

// TestCollectTranscriptChecksResolvesPathLessOften は未解決セッションの再解決が
// stat より長い間隔で行われることを確認する。解決は 3 日ぶんのディレクトリ走査と
// 候補ファイルの先頭行読みを伴うので、stat と同じ頻度で回してはいけない。
func TestCollectTranscriptChecksResolvesPathLessOften(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	now := time.Now()
	ses.transcriptResolvedAt = now.Add(-(transcriptStatAfter + time.Second))

	s.sessionsMu.Lock()
	reqs := s.collectTranscriptChecksLocked(now)
	s.sessionsMu.Unlock()
	if len(reqs) != 0 {
		t.Fatalf("unresolved transcript re-resolved after only the stat interval: %+v", reqs)
	}

	ses.transcriptResolvedAt = now.Add(-(transcriptResolveAfter + time.Second))
	s.sessionsMu.Lock()
	reqs = s.collectTranscriptChecksLocked(now)
	s.sessionsMu.Unlock()
	if len(reqs) != 1 || reqs[0].path != "" {
		t.Fatalf("collectTranscriptChecksLocked() = %+v, want one resolve request (empty path)", reqs)
	}
}

// TestApplyTranscriptStatAdvancesOnlyWhenFileGrows は本機能の中核。
// transcript のサイズが変わらない限り TranscriptGrewAt を進めてはいけない。
// 進めてしまうと停滞が永久に 0 秒になり、検知そのものが成立しない。
func TestApplyTranscriptStatAdvancesOnlyWhenFileGrows(t *testing.T) {
	s := newTestServer()
	registerTestSession(s, 1, "codex")

	s.applyTranscriptStat(1, "rollout.jsonl", 100)
	s.sessionsMu.Lock()
	first := s.sessions[1].TranscriptGrewAt
	s.sessionsMu.Unlock()
	if first == "" {
		t.Fatal("first observation left TranscriptGrewAt empty; the UI would never show a stall")
	}

	const marker = "2020-01-01T00:00:00Z"
	s.sessionsMu.Lock()
	s.sessions[1].TranscriptGrewAt = marker
	s.sessionsMu.Unlock()

	s.applyTranscriptStat(1, "rollout.jsonl", 100) // 同じサイズ = ターンが進んでいない
	s.sessionsMu.Lock()
	stalled := s.sessions[1].TranscriptGrewAt
	s.sessionsMu.Unlock()
	if stalled != marker {
		t.Fatalf("TranscriptGrewAt = %q after an unchanged transcript, want %q", stalled, marker)
	}

	s.applyTranscriptStat(1, "rollout.jsonl", 101) // 伸びた = ターンが進んだ
	s.sessionsMu.Lock()
	grown := s.sessions[1].TranscriptGrewAt
	s.sessionsMu.Unlock()
	if grown == marker {
		t.Fatal("TranscriptGrewAt did not advance after the transcript grew")
	}
}

// TestMarkRunningResetsTranscriptTrackingOnNewTurn は新しいターンの開始で停滞が
// リセットされることを確認する。これが無いと、前のターンが終わってから次の入力
// までの待ち時間がそのまま停滞として積算される。
func TestMarkRunningResetsTranscriptTrackingOnNewTurn(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	ses.State = "standby"
	const stale = "2020-01-01T00:00:00Z"
	ses.TranscriptGrewAt = stale
	ses.transcriptStatAt = time.Now()

	s.markRunning(1)

	s.sessionsMu.Lock()
	got := s.sessions[1].TranscriptGrewAt
	statAt := s.sessions[1].transcriptStatAt
	s.sessionsMu.Unlock()
	if got == stale {
		t.Fatalf("new turn kept the previous turn's TranscriptGrewAt (%q); the idle gap would be reported as a stall", got)
	}
	if !statAt.IsZero() {
		t.Fatal("transcriptStatAt should be cleared so the new turn is stat'd without waiting a full interval")
	}
}

// TestMarkRunningKeepsTranscriptTrackingWithinSameTurn は本機能が壊れる最短の道を塞ぐ。
//
// Codex TUI は "Working (36m 09s)" のカウンタを毎秒再描画するので markRunning は
// ターン中ずっと呼ばれ続ける。ここでリセットしてしまうと停滞は常に 0 秒になり、
// 38 分無音でも UI は何も出せない（これが元の症状そのもの）。
func TestMarkRunningKeepsTranscriptTrackingWithinSameTurn(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex") // registerTestSession は running で作る
	const marker = "2020-01-01T00:00:00Z"
	ses.TranscriptGrewAt = marker

	s.markRunning(1)

	s.sessionsMu.Lock()
	got := s.sessions[1].TranscriptGrewAt
	s.sessionsMu.Unlock()
	if got != marker {
		t.Fatalf("TranscriptGrewAt = %q mid-turn, want %q (PTY redraw must not erase the stall)", got, marker)
	}
}
