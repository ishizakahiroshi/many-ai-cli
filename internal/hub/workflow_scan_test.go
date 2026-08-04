package hub

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
	"unicode/utf8"
)

func workflowFixture(t *testing.T, name string) []string {
	t.Helper()
	b, err := os.ReadFile(filepath.Join("testdata", "workflow_vt", name))
	if err != nil {
		t.Fatal(err)
	}
	return strings.Split(strings.ReplaceAll(string(b), "\r\n", "\n"), "\n")
}

func TestWorkflowScanSummaryRunning(t *testing.T) {
	p := parseWorkflowVT(workflowFixture(t, "summary_running.txt"))
	if p == nil || !p.Detected {
		t.Fatal("summary fixture was not detected")
	}
	if p.Source != "vt-summary" || p.Done != 1 || p.Total != 5 || p.Running != 4 {
		t.Fatalf("unexpected summary counts: %+v", p)
	}
	if p.ElapsedSec != 4*60+21 || p.TokensRaw != "↓ 501.0k" || p.WaitingDynamic != 1 {
		t.Fatalf("unexpected summary metadata: %+v", p)
	}
}

func TestWorkflowScanTreeAndMetrics(t *testing.T) {
	p := parseWorkflowVT(workflowFixture(t, "tree_running.txt"))
	if p == nil || p.Source != "vt-tree" || p.Name != "synthetic-review" {
		t.Fatalf("unexpected tree result: %+v", p)
	}
	if p.Done != 1 || p.Running != 1 || p.Pending != 1 || p.Total != 3 || len(p.Phases) != 2 {
		t.Fatalf("unexpected tree counts: %+v", p)
	}
	if got := p.Phases[0].Agents[0]; got.Label != "inspect:alpha" || got.Metrics != "12s · ↓ 8.0k" {
		t.Fatalf("agent label/metrics not separated: %+v", got)
	}

	view := parseWorkflowVT(workflowFixture(t, "workflows_view.txt"))
	if view == nil || view.Total != 3 || view.Done != 1 || view.Running != 1 || view.Pending != 1 {
		t.Fatalf("/workflows-style fixture mismatch: %+v", view)
	}
}

func TestWorkflowScanDoneAndBrokenFrames(t *testing.T) {
	done := parseWorkflowVT(workflowFixture(t, "summary_done.txt"))
	if done == nil || done.Done != 5 || done.Total != 5 || done.Percent != 100 {
		t.Fatalf("done summary mismatch: %+v", done)
	}
	if got := parseWorkflowVT(workflowFixture(t, "broken_resize.txt")); got != nil {
		t.Fatalf("broken resize frame must not be detected: %+v", got)
	}
	if got := parseWorkflowVT([]string{"ordinary output", "✓ completed a normal edit"}); got != nil {
		t.Fatalf("non-workflow frame must not be detected: %+v", got)
	}
}

func TestWorkflowScanWaitingOnlyIsDetectedSignal(t *testing.T) {
	p := parseWorkflowVT([]string{"Waiting for 2 dynamic workflows to finish"})
	if p == nil || !p.Detected || p.Source != "vt-summary" || p.WaitingDynamic != 2 {
		t.Fatalf("waiting-only frame was not promoted to a workflow signal: %+v", p)
	}
}

func TestWorkflowScanHeaderAndWaitingWithoutAgentsIsDetectedSignal(t *testing.T) {
	p := parseWorkflowVT([]string{
		"⚙ workflow synthetic-background",
		"Waiting for 1 dynamic workflow to finish",
	})
	if p == nil || !p.Detected || p.Source != "vt-summary" || p.Name != "synthetic-background" || p.WaitingDynamic != 1 {
		t.Fatalf("header + waiting frame was not promoted to a workflow signal: %+v", p)
	}
}

func TestWorkflowScanUnicodeTruncationKeepsValidUTF8(t *testing.T) {
	headerName := strings.Repeat("進", 70)
	p := parseWorkflowVT([]string{
		"⚙ workflow " + headerName,
		"  " + strings.Repeat("検証", 50),
		"    ⠋ 合成エージェント",
	})
	if p == nil || !utf8.ValidString(p.Name) || len([]rune(p.Name)) != 60 {
		t.Fatalf("workflow name was not safely rune-truncated: %+v", p)
	}
	if len(p.Phases) != 1 || !utf8.ValidString(p.Phases[0].Title) || len([]rune(p.Phases[0].Title)) != 80 {
		t.Fatalf("phase title was not safely rune-truncated: %+v", p.Phases)
	}
}

func TestWorkflowScanTailIncludesScrollback(t *testing.T) {
	vt := newVTBuffer(100, 3)
	vt.Write([]byte("⚙ workflow synthetic-tail\r\n  ✓ agent-one\r\nline-a\r\nline-b\r\nline-c"))
	if got := parseWorkflowVT(vt.TailLines(3)); got != nil {
		t.Fatalf("screen-only tail unexpectedly retained workflow: %+v", got)
	}
	got := parseWorkflowVT(vt.TailLinesWithScrollback(workflowVTTailLines))
	if got == nil || got.Done != 1 || got.Total != 1 {
		t.Fatalf("scrollback tail did not retain workflow: %+v", got)
	}
}

func TestWorkflowScanProviderGuardAndTrailingDebounce(t *testing.T) {
	now := time.Now()
	codex := &session{Provider: "codex", vt: newVTBuffer(100, 5)}
	claude := &session{Provider: "claude", vt: newVTBuffer(100, 5), workflowLastScanAt: now}
	s := &Server{sessions: map[int]*session{1: codex, 2: claude}}

	s.sessionsMu.Lock()
	s.queueWorkflowVTScanLocked(1, codex, now)
	claude.vt.Write([]byte("◯ synthetic 0/2 agents done · 1s"))
	s.queueWorkflowVTScanLocked(2, claude, now)
	// The final frame arrives inside the debounce window. The already queued
	// trailing scan must observe this frame rather than dropping it.
	claude.vt.Write([]byte("\r✓ synthetic 2/2 agents done · 2s"))
	s.queueWorkflowVTScanLocked(2, claude, now.Add(20*time.Millisecond))
	s.sessionsMu.Unlock()

	if codex.workflowScanTimer != nil {
		t.Fatal("non-Claude provider scheduled a workflow scan")
	}
	time.Sleep(workflowVTScanDebounce + 150*time.Millisecond)
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if claude.vtCounts.Done != 2 || claude.vtCounts.Total != 2 || !claude.vtCounts.Settled {
		t.Fatalf("trailing scan did not observe final frame: %+v", claude.vtCounts)
	}
}

func TestWorkflowScanResizeGuardAndHeartbeat(t *testing.T) {
	now := time.Now()
	guardSes := &session{Provider: "claude", vt: newVTBuffer(100, 5), vtResizeDebounceUntil: now.Add(vtResizeDebounce)}
	guardSes.vt.Write([]byte("◯ synthetic 1/2 agents done · 1m"))
	if due := workflowScanDue(guardSes, now); due.Before(guardSes.vtResizeDebounceUntil) {
		t.Fatalf("scan due %v bypassed resize guard %v", due, guardSes.vtResizeDebounceUntil)
	}
	guardServer := &Server{sessions: map[int]*session{1: guardSes}}
	guardServer.sessionsMu.Lock()
	guardServer.queueWorkflowVTScanLocked(1, guardSes, now)
	guardServer.sessionsMu.Unlock()
	time.Sleep(vtResizeDebounce / 2)
	guardServer.sessionsMu.Lock()
	detectedDuringResize := guardSes.vtCounts.Detected
	guardServer.sessionsMu.Unlock()
	if detectedDuringResize {
		t.Fatal("workflow scan ran during resize debounce")
	}
	time.Sleep(vtResizeDebounce/2 + 100*time.Millisecond)
	guardServer.sessionsMu.Lock()
	detectedAfterResize := guardSes.vtCounts.Detected
	guardServer.sessionsMu.Unlock()
	if !detectedAfterResize {
		t.Fatal("workflow scan did not resume after resize debounce")
	}
	stopWorkflowTestTimer(guardServer, guardSes)

	ses := &session{Provider: "claude"}
	s := &Server{sessions: map[int]*session{1: ses}}
	p := parseWorkflowVT([]string{"◯ synthetic 1/2 agents done · 1m 0s"})
	s.applyWorkflowVTScan(1, ses, p, now)
	stopWorkflowTestTimer(s, ses)
	first := ses.workflowLastBroadcastAt
	s.applyWorkflowVTScan(1, ses, p, now.Add(workflowVTHeartbeat-time.Second))
	stopWorkflowTestTimer(s, ses)
	if !ses.workflowLastBroadcastAt.Equal(first) {
		t.Fatal("unchanged progress broadcast before heartbeat interval")
	}
	s.applyWorkflowVTScan(1, ses, p, now.Add(workflowVTHeartbeat))
	stopWorkflowTestTimer(s, ses)
	if !ses.workflowLastBroadcastAt.Equal(now.Add(workflowVTHeartbeat)) {
		t.Fatal("unchanged progress did not broadcast at heartbeat interval")
	}
}

func TestWorkflowScanMissingSignalSettlesOnlyWhenIdle(t *testing.T) {
	now := time.Now()
	ses := &session{Provider: "claude", Activity: SessionActivity{OutputIdle: false}}
	s := &Server{sessions: map[int]*session{1: ses}}
	p := parseWorkflowVT([]string{"◯ synthetic 1/2 agents done · 1m"})
	s.applyWorkflowVTScan(1, ses, p, now)
	stopWorkflowTestTimer(s, ses)
	s.applyWorkflowVTScan(1, ses, nil, now.Add(time.Second))
	stopWorkflowTestTimer(s, ses)
	if ses.workflowMissingScans != 0 {
		t.Fatal("non-idle missing scan contributed to settle")
	}
	ses.Activity.OutputIdle = true
	for i := 1; i <= workflowVTMissingSettleMax; i++ {
		s.applyWorkflowVTScan(1, ses, nil, now.Add(time.Duration(i+1)*time.Second))
		stopWorkflowTestTimer(s, ses)
	}
	if ses.workflowVTProgress == nil || !ses.workflowVTProgress.Settled || ses.workflowVTProgress.SettledBy != "vt" {
		t.Fatalf("idle disappearance did not VT-settle: %+v", ses.workflowVTProgress)
	}
}

func TestWorkflowScanCompletedSummaryWaitsForDynamicWorkflow(t *testing.T) {
	now := time.Now()
	ses := &session{Provider: "claude"}
	s := &Server{sessions: map[int]*session{1: ses}}
	p := parseWorkflowVT([]string{
		"✓ synthetic 2/2 agents done · 1m",
		"*Waiting for 1 dynamic workflow to finish",
	})
	s.applyWorkflowVTScan(1, ses, p, now)
	stopWorkflowTestTimer(s, ses)
	if ses.workflowVTProgress == nil || ses.workflowVTProgress.Settled {
		t.Fatalf("done summary settled while dynamic workflow remained: %+v", ses.workflowVTProgress)
	}
}

func TestWorkflowScanKeepsVTAndJournalCountsSeparate(t *testing.T) {
	vt := parseWorkflowVT([]string{
		"⚙ workflow synthetic",
		"  ✓ agent-one",
		"  ⠋ agent-two",
		"  ○ agent-three",
	})
	vt.Settled = true
	vt.SettledBy = "vt"
	vtCounts := workflowCountsFromProgress(vt, time.Now())
	journalCounts := workflowCounts{Detected: true, Started: 4, Done: 2}

	combined := composeWorkflowProgress(vt, vtCounts, journalCounts)
	if combined.Source != "journal" || combined.Done != 2 || combined.Total != 3 {
		t.Fatalf("field-level authority mismatch: %+v", combined)
	}
	if len(combined.Phases) != 1 || combined.Running != 1 || combined.Pending != 1 {
		t.Fatalf("VT-only detail was lost: %+v", combined)
	}
	if combined.Settled || combined.SettledBy != "" {
		t.Fatalf("incomplete journal did not suppress VT settle: %+v", combined)
	}
	if vtCounts.Done != 1 || journalCounts.Done != 2 {
		t.Fatal("source counts were mutated during composition")
	}
}

// 敵対レビュー 2026-08-05 F1: scrollback に残った stale サマリー（done<total）は
// hasSignal を立て続けるため missing-signal settle が発火せず、heartbeat が無期限
// に自己再予約していた。凍結判定（同一フレーム × output-idle × N 周）で settle し、
// 以後の stale 再パースでは settled を剥がさない（経過時間の前進 = 生存証拠が
// ある時だけ復活させる）ことを固定する。
func TestWorkflowFrozenStaleFrameSettlesSticksAndRevives(t *testing.T) {
	now := time.Now()
	ses := &session{Provider: "claude", Activity: SessionActivity{OutputIdle: true}}
	s := &Server{sessions: map[int]*session{1: ses}}
	stale := parseWorkflowVT([]string{"◐ wf 43/90 agents done · 5m 25s · ↓ 1.9m"})
	if stale == nil || stale.Done != 43 || stale.Total != 90 {
		t.Fatalf("synthetic stale frame did not parse: %+v", stale)
	}
	for i := 0; i <= workflowVTFrozenSettleMax; i++ {
		s.applyWorkflowVTScan(1, ses, cloneWorkflowProgress(stale), now.Add(time.Duration(i)*workflowVTHeartbeat))
		stopWorkflowTestTimer(s, ses)
	}
	if ses.workflowVTProgress == nil || !ses.workflowVTProgress.Settled || ses.workflowVTProgress.SettledBy != "vt" {
		t.Fatalf("frozen stale frame did not settle: %+v", ses.workflowVTProgress)
	}
	if ses.workflowVTProgress.Running != 0 || !ses.vtCounts.Settled {
		t.Fatalf("frozen settle left running counts: %+v / %+v", ses.workflowVTProgress, ses.vtCounts)
	}

	// Sticky: 同じ stale フレームの再パースは settled を剥がさない。
	s.applyWorkflowVTScan(1, ses, cloneWorkflowProgress(stale), now.Add(10*workflowVTHeartbeat))
	stopWorkflowTestTimer(s, ses)
	if !ses.workflowVTProgress.Settled {
		t.Fatal("stale re-parse revived a settled workflow")
	}

	// 経過時間が前進したフレームはライブ描画の証拠なので走行中へ復活する。
	live := parseWorkflowVT([]string{"◐ wf 43/90 agents done · 5m 26s · ↓ 1.9m"})
	s.applyWorkflowVTScan(1, ses, live, now.Add(11*workflowVTHeartbeat))
	stopWorkflowTestTimer(s, ses)
	if ses.workflowVTProgress.Settled {
		t.Fatal("advancing elapsed frame did not revive the workflow")
	}
}

// 敵対レビュー 2026-08-05 F1/F11: セッション終端（session_end / wrapper 切断）で
// 追跡タイマーを止め、未 settle の workflow を 1 回だけ強制 settle する。完了 push
// は出さない（notified 済みへ倒す）。
func TestWorkflowFinalizeOnSessionEndForcesSettleWithoutPush(t *testing.T) {
	partial := parseWorkflowVT([]string{"◐ wf 1/3 agents done · 10s"})
	ses := &session{
		Provider:           "claude",
		workflowVTProgress: partial,
		vtCounts:           workflowCountsFromProgress(partial, time.Now()),
		journalCounts:      workflowCounts{Detected: true, Started: 3, Done: 1},
	}
	s := &Server{sessions: map[int]*session{1: ses}}
	s.finalizeWorkflowOnSessionEnd(1)
	if ses.workflowVTProgress == nil || !ses.workflowVTProgress.Settled || ses.workflowVTProgress.SettledBy != "timeout" {
		t.Fatalf("session end did not settle VT progress: %+v", ses.workflowVTProgress)
	}
	if !ses.journalCounts.Settled || ses.journalCounts.SettledBy != "timeout" {
		t.Fatalf("session end did not settle journal counts: %+v", ses.journalCounts)
	}
	if ses.workflowScanTimer != nil || ses.workflowJournalTimer != nil || ses.workflowJournalRunning {
		t.Fatal("session end left workflow timers running")
	}
	if !ses.workflowCompletionNotified {
		t.Fatal("finalize must mark completion notified so no later push fires")
	}
	// 冪等: 2 回目は変更なしで安全に抜ける。
	s.finalizeWorkflowOnSessionEnd(1)
	if !ses.workflowVTProgress.Settled {
		t.Fatal("second finalize corrupted settled state")
	}
}

// 敵対レビュー 2026-08-05: journal が VT フレームより先に result を観測すると
// 合成ビューが「6/5 完了」になり得た。表示値のみクランプする。
func TestComposeWorkflowProgressClampsJournalDoneToVTTotal(t *testing.T) {
	vt := parseWorkflowVT([]string{"◐ wf 4/5 agents done · 1m 2s"})
	out := composeWorkflowProgress(vt, workflowCounts{}, workflowCounts{Detected: true, Started: 6, Done: 6, Total: 6})
	if out == nil || out.Done != 5 || out.Total != 5 {
		t.Fatalf("composed view not clamped to total: %+v", out)
	}
	if out.Percent != 100 {
		t.Fatalf("clamped view percent = %d, want 100", out.Percent)
	}
}

func stopWorkflowTestTimer(s *Server, ses *session) {
	s.sessionsMu.Lock()
	if ses.workflowScanTimer != nil {
		ses.workflowScanTimer.Stop()
		ses.workflowScanTimer = nil
		ses.workflowScanDue = time.Time{}
	}
	s.sessionsMu.Unlock()
}
