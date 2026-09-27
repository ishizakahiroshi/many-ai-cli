package hub

import (
	"os"
	"path/filepath"
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

	s.applyTranscriptStat(1, "rollout.jsonl", 100, time.Time{})
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

	s.applyTranscriptStat(1, "rollout.jsonl", 100, time.Time{}) // 同じサイズ = ターンが進んでいない
	s.sessionsMu.Lock()
	stalled := s.sessions[1].TranscriptGrewAt
	s.sessionsMu.Unlock()
	if stalled != marker {
		t.Fatalf("TranscriptGrewAt = %q after an unchanged transcript, want %q", stalled, marker)
	}

	s.applyTranscriptStat(1, "rollout.jsonl", 101, time.Time{}) // 伸びた = ターンが進んだ
	s.sessionsMu.Lock()
	grown := s.sessions[1].TranscriptGrewAt
	s.sessionsMu.Unlock()
	if grown == marker {
		t.Fatal("TranscriptGrewAt did not advance after the transcript grew")
	}
}

// TestApplyTranscriptStatAdvancesWhenSubagentDirGrows は本 bugfix の中核。
// 親 transcript のサイズが同じでも、サブエージェントディレクトリの mtime が進めば
// TranscriptGrewAt を進める（サブエージェント実行中は親に書かれない = 停滞ではない）。
func TestApplyTranscriptStatAdvancesWhenSubagentDirGrows(t *testing.T) {
	s := newTestServer()
	registerTestSession(s, 1, "claude")

	s.applyTranscriptStat(1, "rollout.jsonl", 100, time.Time{})
	const marker = "2020-01-01T00:00:00Z"
	s.sessionsMu.Lock()
	s.sessions[1].TranscriptGrewAt = marker
	s.sessionsMu.Unlock()

	subagentAt := time.Now()
	s.applyTranscriptStat(1, "rollout.jsonl", 100, subagentAt) // サイズは同じ、サブエージェントだけ動いた
	s.sessionsMu.Lock()
	grown := s.sessions[1].TranscriptGrewAt
	s.sessionsMu.Unlock()
	if grown == marker {
		t.Fatal("TranscriptGrewAt did not advance when the subagent directory's mtime moved forward")
	}
}

// TestSubagentDirForTranscriptRequiresJsonlSuffix はディレクトリが無い（または該当しない
// provider の）場合に、従来どおりサイズだけで判定されることを確認する。
func TestSubagentDirForTranscriptRequiresJsonlSuffix(t *testing.T) {
	if got := subagentDirForTranscript("rollout.json"); got != "" {
		t.Fatalf("subagentDirForTranscript(%q) = %q, want empty (not a claude transcript)", "rollout.json", got)
	}
	if got := subagentDirForTranscript(""); got != "" {
		t.Fatalf("subagentDirForTranscript(\"\") = %q, want empty", got)
	}
}

// TestLatestSubagentMTimeMissingDirFallsBackToSizeOnly はサブエージェント
// ディレクトリが存在しないときにゼロ値へ倒れ、サイズだけの判定を壊さないことを
// 確認する（既存の TestApplyTranscriptStatAdvancesOnlyWhenFileGrows が対象の経路）。
func TestLatestSubagentMTimeMissingDirFallsBackToSizeOnly(t *testing.T) {
	dir := t.TempDir()
	transcriptPath := filepath.Join(dir, "session-1.jsonl") // subagents ディレクトリは作らない

	got := latestSubagentMTime(transcriptPath)
	if !got.IsZero() {
		t.Fatalf("latestSubagentMTime() = %v, want zero value when the subagents dir does not exist", got)
	}
}

// TestLatestSubagentMTimeTakesNewestEntry は「直下エントリのうち最新」を取れているかを
// 確認する。ここを取り違える（古い方を返す）と、走っているサブエージェントが居ても
// 観測値が進まず、本 bugfix の修正が黙って効かなくなる。エラーにはならないので
// テストが無いと気づけない。
func TestLatestSubagentMTimeTakesNewestEntry(t *testing.T) {
	dir := t.TempDir()
	subagentDir := filepath.Join(dir, "subagents")
	if err := os.MkdirAll(subagentDir, 0o755); err != nil {
		t.Fatalf("MkdirAll(subagents) failed: %v", err)
	}
	older := time.Now().Add(-2 * time.Hour)
	newer := time.Now().Add(-1 * time.Hour)
	for name, mt := range map[string]time.Time{"agent-old.jsonl": older, "agent-new.jsonl": newer} {
		path := filepath.Join(subagentDir, name)
		if err := os.WriteFile(path, []byte("{}"), 0o644); err != nil {
			t.Fatalf("WriteFile(%s) failed: %v", name, err)
		}
		if err := os.Chtimes(path, mt, mt); err != nil {
			t.Fatalf("Chtimes(%s) failed: %v", name, err)
		}
	}

	// ファイルシステムによっては mtime の精度が秒単位まで落ちるので、厳密一致ではなく
	// 許容差で比べる。古い方（2 時間前）を返していれば必ず外れる幅にしてある。
	got := latestSubagentMTime(dir + ".jsonl")
	if diff := got.Sub(newer); diff > 2*time.Second || diff < -2*time.Second {
		t.Fatalf("latestSubagentMTime() = %v, want the newest entry (%v, older one was %v)", got, newer, older)
	}
}

// TestApplyTranscriptStatNoGrowthWhenSizeAndMtimeUnchanged は停滞検知そのものが
// 死んでいないことの確認。サイズもサブエージェントの mtime も変わらなければ
// TranscriptGrewAt は進めない。**ここが緩むと Codex の 38 分無音を二度と拾えなくなる。**
func TestApplyTranscriptStatNoGrowthWhenSizeAndMtimeUnchanged(t *testing.T) {
	dir := t.TempDir()
	subagentDir := filepath.Join(dir, "subagents")
	if err := os.MkdirAll(subagentDir, 0o755); err != nil {
		t.Fatalf("MkdirAll(subagents) failed: %v", err)
	}
	agentFile := filepath.Join(subagentDir, "agent-1.jsonl")
	if err := os.WriteFile(agentFile, []byte("{}"), 0o644); err != nil {
		t.Fatalf("WriteFile(agent-1.jsonl) failed: %v", err)
	}
	fixedMTime := time.Now().Add(-time.Hour)
	if err := os.Chtimes(agentFile, fixedMTime, fixedMTime); err != nil {
		t.Fatalf("Chtimes(agent-1.jsonl) failed: %v", err)
	}

	transcriptPath := dir + ".jsonl" // sessionDir + ".jsonl" = transcriptPath の対応関係
	subagentAt := latestSubagentMTime(transcriptPath)
	if subagentAt.IsZero() {
		t.Fatal("latestSubagentMTime() returned zero value; test fixture is set up wrong")
	}

	s := newTestServer()
	registerTestSession(s, 1, "claude")

	s.applyTranscriptStat(1, transcriptPath, 100, subagentAt) // 初回観測
	const marker = "2020-01-01T00:00:00Z"
	s.sessionsMu.Lock()
	s.sessions[1].TranscriptGrewAt = marker
	s.sessionsMu.Unlock()

	// 2 回目: サイズもサブエージェントの mtime も 1 回目から変わっていない。
	s.applyTranscriptStat(1, transcriptPath, 100, subagentAt)
	s.sessionsMu.Lock()
	got := s.sessions[1].TranscriptGrewAt
	s.sessionsMu.Unlock()
	if got != marker {
		t.Fatalf("TranscriptGrewAt = %q with unchanged size and subagent mtime, want %q (stall detection must still fire)", got, marker)
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
