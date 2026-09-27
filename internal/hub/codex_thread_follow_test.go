package hub

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// 2026-09-25 の実測を写した時刻。21:38 に始まったセッションが 21:54 で止まり、
// 22:01 に同じプロセスの中で新しい会話が始まった。
var (
	codexFollowCWD        = `C:\workspace\many-ai-cli`
	codexFollowStart      = time.Date(2026, 9, 25, 21, 38, 7, 0, time.Local)
	codexFollowLastWrite  = time.Date(2026, 9, 25, 21, 54, 16, 0, time.Local)
	codexFollowNewStart   = time.Date(2026, 9, 25, 22, 1, 18, 0, time.Local)
	codexFollowNewWritten = time.Date(2026, 9, 25, 22, 52, 46, 0, time.Local)
	codexFollowNow        = time.Date(2026, 9, 25, 22, 53, 0, 0, time.Local)
)

type codexFollowRollout struct {
	name, cwd, threadSource, parent string
	start, lastWrite                time.Time
}

func writeCodexFollowRollout(t *testing.T, codexHome string, r codexFollowRollout) string {
	t.Helper()
	day := r.start.Local()
	dir := filepath.Join(codexHome, "sessions", day.Format("2006"), day.Format("01"), day.Format("02"))
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	meta := codexSessionMeta{Type: "session_meta"}
	meta.Payload.CWD = r.cwd
	meta.Payload.Timestamp = r.start.Format(time.RFC3339Nano)
	meta.Payload.ThreadSource = r.threadSource
	meta.Payload.ParentThreadID = r.parent
	line, err := json.Marshal(meta)
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, r.name)
	if err := os.WriteFile(path, append(line, '\n'), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(path, r.lastWrite, r.lastWrite); err != nil {
		t.Fatal(err)
	}
	return path
}

// writeCodexFollowPair は開始時の会話と、22:01 に切り替えた後の会話を置く。
func writeCodexFollowPair(t *testing.T, codexHome string) (oldPath, newPath string) {
	t.Helper()
	oldPath = writeCodexFollowRollout(t, codexHome, codexFollowRollout{
		name: "rollout-old.jsonl", cwd: codexFollowCWD, threadSource: "user",
		start: codexFollowStart, lastWrite: codexFollowLastWrite,
	})
	newPath = writeCodexFollowRollout(t, codexHome, codexFollowRollout{
		name: "rollout-new.jsonl", cwd: codexFollowCWD, threadSource: "user",
		start: codexFollowNewStart, lastWrite: codexFollowNewWritten,
	})
	return oldPath, newPath
}

// 開始時刻だけで探すと、切り替えた後の会話は窓の外になる。本 bugfix の出発点。
func TestFindCodexRolloutLogStaysOnFirstConversationAfterSwitch(t *testing.T) {
	codexHome := t.TempDir()
	oldPath, _ := writeCodexFollowPair(t, codexHome)
	got, ok := findCodexRolloutLog(codexHome, codexFollowCWD, codexFollowStart)
	if !ok || got != oldPath {
		t.Fatalf("findCodexRolloutLog = %q, %v; want the first conversation %q", got, ok, oldPath)
	}
}

func TestPickCodexThreadSwitchFollowsNewConversation(t *testing.T) {
	codexHome := t.TempDir()
	oldPath, newPath := writeCodexFollowPair(t, codexHome)
	got, ambiguous := pickCodexThreadSwitch(codexHome, codexFollowCWD, oldPath, nil, codexFollowNow)
	if got != newPath || ambiguous {
		t.Fatalf("pickCodexThreadSwitch = %q, %v; want %q, false", got, ambiguous, newPath)
	}
}

// Codex のサブエージェントは同じ cwd に rollout を作る。乗り換え先にしない。
func TestPickCodexThreadSwitchIgnoresSubagentConversation(t *testing.T) {
	codexHome := t.TempDir()
	oldPath := writeCodexFollowRollout(t, codexHome, codexFollowRollout{
		name: "rollout-old.jsonl", cwd: codexFollowCWD, threadSource: "user",
		start: codexFollowStart, lastWrite: codexFollowLastWrite,
	})
	writeCodexFollowRollout(t, codexHome, codexFollowRollout{
		name: "rollout-subagent.jsonl", cwd: codexFollowCWD, threadSource: "subagent", parent: "01a0d892",
		start: codexFollowNewStart, lastWrite: codexFollowNewWritten,
	})
	if got, ambiguous := pickCodexThreadSwitch(codexHome, codexFollowCWD, oldPath, nil, codexFollowNow); got != "" || ambiguous {
		t.Fatalf("pickCodexThreadSwitch = %q, %v; want no switch for a subagent rollout", got, ambiguous)
	}
}

// 今の会話がまだ書かれ続けているなら、後から始まった会話は並行して動く別の Codex のもの。
func TestPickCodexThreadSwitchIgnoresConversationWhileCurrentIsStillWritten(t *testing.T) {
	codexHome := t.TempDir()
	oldPath := writeCodexFollowRollout(t, codexHome, codexFollowRollout{
		name: "rollout-old.jsonl", cwd: codexFollowCWD, threadSource: "user",
		start: codexFollowStart, lastWrite: codexFollowNewWritten,
	})
	writeCodexFollowRollout(t, codexHome, codexFollowRollout{
		name: "rollout-other.jsonl", cwd: codexFollowCWD, threadSource: "user",
		start: codexFollowNewStart, lastWrite: codexFollowNewWritten,
	})
	if got, ambiguous := pickCodexThreadSwitch(codexHome, codexFollowCWD, oldPath, nil, codexFollowNow); got != "" || ambiguous {
		t.Fatalf("pickCodexThreadSwitch = %q, %v; want no switch while the current rollout is still written", got, ambiguous)
	}
}

func TestPickCodexThreadSwitchIgnoresOtherCWD(t *testing.T) {
	codexHome := t.TempDir()
	oldPath := writeCodexFollowRollout(t, codexHome, codexFollowRollout{
		name: "rollout-old.jsonl", cwd: codexFollowCWD, threadSource: "user",
		start: codexFollowStart, lastWrite: codexFollowLastWrite,
	})
	writeCodexFollowRollout(t, codexHome, codexFollowRollout{
		name: "rollout-other-cwd.jsonl", cwd: `C:\workspace\other`, threadSource: "user",
		start: codexFollowNewStart, lastWrite: codexFollowNewWritten,
	})
	if got, ambiguous := pickCodexThreadSwitch(codexHome, codexFollowCWD, oldPath, nil, codexFollowNow); got != "" || ambiguous {
		t.Fatalf("pickCodexThreadSwitch = %q, %v; want no switch for another cwd", got, ambiguous)
	}
}

func TestPickCodexThreadSwitchPeers(t *testing.T) {
	codexHome := t.TempDir()
	oldPath, newPath := writeCodexFollowPair(t, codexHome)

	// 他のセッションが既に読んでいる会話は、そのセッションのもの。
	claimed := []codexThreadPeer{{paths: []string{newPath}, startedAt: codexFollowNewStart}}
	if got, ambiguous := pickCodexThreadSwitch(codexHome, codexFollowCWD, oldPath, claimed, codexFollowNow); got != "" || ambiguous {
		t.Fatalf("claimed by peer: got %q, %v; want no switch", got, ambiguous)
	}

	// パスをまだ決めていないセッションが開始時刻で拾う見込みの会話も、そのセッションのもの。
	unresolved := []codexThreadPeer{{startedAt: codexFollowNewStart.Add(-2 * time.Second)}}
	if got, ambiguous := pickCodexThreadSwitch(codexHome, codexFollowCWD, oldPath, unresolved, codexFollowNow); got != "" || ambiguous {
		t.Fatalf("claimable by unresolved peer: got %q, %v; want no switch", got, ambiguous)
	}

	// 誰のものでもない会話があり、同じ場所で他の Codex が動いているなら決められない。
	other := []codexThreadPeer{{paths: []string{filepath.Join(codexHome, "elsewhere.jsonl")}, startedAt: codexFollowStart}}
	if got, ambiguous := pickCodexThreadSwitch(codexHome, codexFollowCWD, oldPath, other, codexFollowNow); got != "" || !ambiguous {
		t.Fatalf("unclaimed with peer: got %q, %v; want ambiguous", got, ambiguous)
	}
}

func registerCodexFollowSession(s *Server, id int, codexHome, transcript string) *session {
	ses := registerTestSession(s, id, "codex")
	ses.CodexHome = codexHome
	ses.CWD = codexFollowCWD
	ses.StartedAt = codexFollowStart.Format(time.RFC3339)
	ses.State = "running"
	ses.agentChatPath = transcript
	return ses
}

// 乗り換えは NativeLogPath に入り、トランスクリプトの解決がそれを選ぶ。
// 承認マーカーの供給元はトランスクリプトのまま（新しい会話から読む）。
func TestFollowCodexThreadSwitchRepointsTranscript(t *testing.T) {
	codexHome := t.TempDir()
	oldPath, newPath := writeCodexFollowPair(t, codexHome)
	s := newTestServer()
	ses := registerCodexFollowSession(s, 1, codexHome, oldPath)

	if !s.followCodexThreadSwitch(1, oldPath, time.Now()) {
		t.Fatal("followCodexThreadSwitch = false, want a switch to the new conversation")
	}
	if ses.NativeLogPath != newPath {
		t.Fatalf("NativeLogPath = %q, want %q", ses.NativeLogPath, newPath)
	}
	if got, ok := agentChatTranscriptPathForSnapshot(s.agentChatSnapshot(1)); !ok || got != newPath {
		t.Fatalf("transcript path = %q, %v; want %q", got, ok, newPath)
	}
	if !approvalMarkerSourceIsTranscriptLocked(ses) {
		t.Fatal("approval marker source left the transcript after an unambiguous switch")
	}
	// 間隔の内側では探し直さない。
	if s.followCodexThreadSwitch(1, newPath, time.Now()) {
		t.Fatal("followCodexThreadSwitch ran again inside codexThreadFollowInterval")
	}
}

// 取り違えを否定できないときは乗り換えず、承認マーカーの供給元を VT ミラーへ戻す。
func TestFollowCodexThreadSwitchAmbiguousFallsBackToVT(t *testing.T) {
	codexHome := t.TempDir()
	oldPath, _ := writeCodexFollowPair(t, codexHome)
	peerPath := writeCodexFollowRollout(t, codexHome, codexFollowRollout{
		name: "rollout-peer.jsonl", cwd: codexFollowCWD, threadSource: "user",
		start: codexFollowStart.Add(time.Minute), lastWrite: codexFollowNewWritten,
	})
	s := newTestServer()
	ses := registerCodexFollowSession(s, 1, codexHome, oldPath)
	registerCodexFollowSession(s, 2, codexHome, peerPath)

	if !approvalMarkerSourceIsTranscriptLocked(ses) {
		t.Fatal("precondition: transcript should be the approval marker source")
	}
	if s.followCodexThreadSwitch(1, oldPath, time.Now()) {
		t.Fatal("followCodexThreadSwitch switched although another codex session shares the cwd")
	}
	if ses.NativeLogPath != "" {
		t.Fatalf("NativeLogPath = %q, want untouched", ses.NativeLogPath)
	}
	if approvalMarkerSourceIsTranscriptLocked(ses) {
		t.Fatal("ambiguous switch kept the stale transcript as the approval marker source")
	}
}
