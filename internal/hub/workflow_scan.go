package hub

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"regexp"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"many-ai-cli/internal/proto"
)

const (
	workflowVTTailLines        = 200
	workflowVTScanDebounce     = 500 * time.Millisecond
	workflowVTHeartbeat        = 7 * time.Second
	workflowVTMissingSettleMax = 3
	// 同一シグネチャ × output-idle がこの回数（heartbeat 周期）続いたら settle。
	// TS 側 SETTLE_MISS_LIMIT のフレーム凍結ヒューリスティックの Hub 版で、
	// scrollback に残った stale サマリーが missing-signal settle を永久に
	// ブロックする問題（敵対レビュー 2026-08-05 F1）への対処。
	workflowVTFrozenSettleMax = 3
)

var (
	workflowCSIRe     = regexp.MustCompile(`\x1b\[[0-9;?]*[ -/]*[@-~]`)
	workflowOSCRe     = regexp.MustCompile(`\x1b\][^\x07]*(?:\x07|\x1b\\)`)
	workflowTreeRe    = regexp.MustCompile(`^[\s│├└─╰╭╮╯┃┣┗┏┓┛┆┊▕▏▸▹‣•·]+`)
	workflowHeaderRe  = regexp.MustCompile(`(?i)\bworkflows?\b`)
	workflowSummaryRe = regexp.MustCompile(`(?i)([0-9]{1,4})\s*/\s*([0-9]{1,4})\s+agents?\b`)
	workflowWaitingRe = regexp.MustCompile(`(?i)\bwaiting for\s+([0-9]{1,3})\s+dynamic\s+workflows?\s+to\s+finish\b`)
	workflowPercentRe = regexp.MustCompile(`([0-9]{1,3})\s*%`)
	workflowTimeRe    = regexp.MustCompile(`(?i)([0-9]+)\s*([hms])`)
	workflowMetricsRe = regexp.MustCompile(`\s{2,}`)
)

const (
	workflowSpinnerGlyphs = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏⣾⣽⣻⢿⡿⣟⣯⣷◐◓◑◒◜◝◞◟"
	workflowDoneGlyphs    = "✓✔"
	workflowFailedGlyphs  = "✗✘"
	workflowPendingGlyphs = "○◌◯"
)

// workflowCounts is deliberately duplicated per source in session. C3 writes
// journalCounts while this file writes vtCounts; composing the wire snapshot
// must never make one observation overwrite the other.
type workflowCounts struct {
	Detected       bool
	Started        int
	Done           int
	Total          int
	Running        int
	Failed         int
	Pending        int
	WaitingDynamic int
	Percent        int
	Settled        bool
	SettledBy      string
	ObservedAt     time.Time
}

func stripWorkflowANSI(line string) string {
	line = workflowOSCRe.ReplaceAllString(line, "")
	return workflowCSIRe.ReplaceAllString(line, "")
}

func stripWorkflowTree(line string) string {
	return workflowTreeRe.ReplaceAllString(stripWorkflowANSI(line), "")
}

func truncateWorkflowText(s string, maxRunes int) string {
	if maxRunes <= 0 {
		return ""
	}
	runes := []rune(s)
	if len(runes) <= maxRunes {
		return s
	}
	return string(runes[:maxRunes])
}

func workflowHeaderName(line string) (string, bool) {
	hasGear := strings.Contains(line, "⚙")
	if !hasGear && !workflowHeaderRe.MatchString(line) {
		return "", false
	}
	name := strings.ReplaceAll(line, "⚙", "")
	name = workflowHeaderRe.ReplaceAllString(name, "")
	name = strings.TrimSpace(strings.TrimLeft(name, ":："))
	name = strings.Join(strings.Fields(name), " ")
	switch strings.ToLower(name) {
	case "running", "done", "complete", "completed", "in progress":
		name = ""
	}
	name = truncateWorkflowText(name, 60)
	return name, true
}

func workflowSummary(line string) (done, total, elapsed int, tokens string, ok bool) {
	m := workflowSummaryRe.FindStringSubmatch(line)
	if m == nil {
		return 0, 0, 0, "", false
	}
	done, _ = strconv.Atoi(m[1])
	total, _ = strconv.Atoi(m[2])
	if total <= 0 || done < 0 || done > total {
		return 0, 0, 0, "", false
	}
	tail := line[strings.Index(line, m[0])+len(m[0]):]
	elapsedFound := false
	for _, segment := range strings.FieldsFunc(tail, func(r rune) bool { return r == '·' || r == '•' }) {
		segment = strings.TrimSpace(segment)
		if strings.HasPrefix(segment, "↓") {
			tokens = segment
			continue
		}
		matches := workflowTimeRe.FindAllStringSubmatch(segment, -1)
		if len(matches) == 0 || elapsedFound {
			continue
		}
		for _, tm := range matches {
			v, _ := strconv.Atoi(tm[1])
			switch strings.ToLower(tm[2]) {
			case "h":
				elapsed += v * 3600
			case "m":
				elapsed += v * 60
			case "s":
				elapsed += v
			}
		}
		elapsedFound = true
	}
	return done, total, elapsed, tokens, true
}

func workflowAgent(line string) (proto.WfAgent, bool) {
	first, size := utf8.DecodeRuneInString(line)
	if first == utf8.RuneError || size == 0 {
		return proto.WfAgent{}, false
	}
	state := ""
	switch {
	case strings.ContainsRune(workflowSpinnerGlyphs, first) || first == '●':
		state = "running"
	case strings.ContainsRune(workflowDoneGlyphs, first):
		state = "done"
	case strings.ContainsRune(workflowFailedGlyphs, first):
		state = "failed"
	case strings.ContainsRune(workflowPendingGlyphs, first):
		state = "pending"
	default:
		return proto.WfAgent{}, false
	}
	rest := line[size:]
	if rest != "" && rest[0] != ' ' && rest[0] != '\t' {
		return proto.WfAgent{}, false
	}
	rest = strings.TrimSpace(rest)
	label, metrics := rest, ""
	if loc := workflowMetricsRe.FindStringIndex(rest); loc != nil {
		label = strings.TrimSpace(rest[:loc[0]])
		metrics = strings.TrimSpace(rest[loc[1]:])
	}
	label = strings.Join(strings.Fields(label), " ")
	if label == "" || len(label) > 200 {
		return proto.WfAgent{}, false
	}
	return proto.WfAgent{Label: label, State: state, Metrics: metrics}, true
}

func workflowWaiting(lines []string) int {
	for i := len(lines) - 1; i >= 0; i-- {
		m := workflowWaitingRe.FindStringSubmatch(stripWorkflowANSI(lines[i]))
		if m == nil {
			continue
		}
		n, _ := strconv.Atoi(m[1])
		if n > 0 {
			return n
		}
	}
	return 0
}

// parseWorkflowVT is a pure parser. The caller supplies plain VT mirror lines;
// ANSI stripping is repeated defensively so synthetic/raw fixtures are safe.
func parseWorkflowVT(lines []string) *proto.WorkflowProgress {
	if len(lines) == 0 {
		return nil
	}
	waiting := workflowWaiting(lines)
	start, name := -1, ""
	for i := len(lines) - 1; i >= 0; i-- {
		line := stripWorkflowTree(lines[i])
		if _, _, _, _, ok := workflowSummary(line); ok {
			start = i
			break
		}
		// The waiting sentinel contains the word "workflow", but it is not a
		// heading. Treating it as one would hide a valid tree immediately above.
		if workflowWaitingRe.MatchString(line) {
			continue
		}
		if n, ok := workflowHeaderName(line); ok {
			start, name = i, n
			break
		}
	}
	if start < 0 {
		if waiting > 0 {
			return &proto.WorkflowProgress{Detected: true, Source: "vt-summary", WaitingDynamic: waiting}
		}
		return nil
	}

	p := &proto.WorkflowProgress{Name: name, WaitingDynamic: waiting}
	var current *proto.WfPhase
	pendingTitle := ""
	explicitPercent := -1
	hasSummary := false
	for i := start; i < len(lines); i++ {
		line := stripWorkflowTree(lines[i])
		if line == "" {
			continue
		}
		if done, total, elapsed, tokens, ok := workflowSummary(line); ok {
			p.Done, p.Total, p.Running = done, total, total-done
			p.ElapsedSec, p.TokensRaw = elapsed, tokens
			hasSummary = true
			continue
		}
		if i == start {
			continue
		}
		if agent, ok := workflowAgent(line); ok {
			if current == nil || pendingTitle != "" {
				p.Phases = append(p.Phases, proto.WfPhase{Title: pendingTitle})
				current = &p.Phases[len(p.Phases)-1]
				pendingTitle = ""
			}
			current.Agents = append(current.Agents, agent)
			continue
		}
		if m := workflowPercentRe.FindStringSubmatch(line); m != nil {
			v, _ := strconv.Atoi(m[1])
			if v <= 100 {
				explicitPercent = v
				continue
			}
		}
		pendingTitle = strings.Join(strings.Fields(line), " ")
		pendingTitle = truncateWorkflowText(pendingTitle, 80)
	}

	if hasSummary {
		p.Source = "vt-summary"
	} else {
		p.Source = "vt-tree"
		for _, phase := range p.Phases {
			for _, agent := range phase.Agents {
				p.Total++
				switch agent.State {
				case "running":
					p.Running++
				case "done":
					p.Done++
				case "failed":
					p.Failed++
				case "pending":
					p.Pending++
				}
			}
		}
	}
	if p.Total == 0 {
		if waiting > 0 {
			p.Detected = true
			p.Source = "vt-summary"
			return p
		}
		return nil
	}
	p.Detected = true
	if hasSummary {
		p.Percent = p.Done * 100 / p.Total
	} else if explicitPercent >= 0 {
		p.Percent = explicitPercent
	} else {
		p.Percent = (p.Done + p.Failed) * 100 / p.Total
	}
	return p
}

func workflowCountsFromProgress(p *proto.WorkflowProgress, observedAt time.Time) workflowCounts {
	if p == nil {
		return workflowCounts{}
	}
	return workflowCounts{Detected: p.Detected, Done: p.Done, Total: p.Total, Running: p.Running,
		Failed: p.Failed, Pending: p.Pending, WaitingDynamic: p.WaitingDynamic, Percent: p.Percent,
		Settled: p.Settled, SettledBy: p.SettledBy, ObservedAt: observedAt}
}

// composeWorkflowProgress keeps field-level authority explicit. VT owns total,
// running/pending and tree detail; once C3 links a journal, its result count and
// settle decision take precedence without mutating vtCounts.
func composeWorkflowProgress(vt *proto.WorkflowProgress, vtCounts, journalCounts workflowCounts) *proto.WorkflowProgress {
	p := cloneWorkflowProgress(vt)
	if p == nil {
		if !journalCounts.Detected {
			return nil
		}
		p = &proto.WorkflowProgress{
			Detected:       true,
			Total:          vtCounts.Total,
			Running:        vtCounts.Running,
			Failed:         vtCounts.Failed,
			Pending:        vtCounts.Pending,
			WaitingDynamic: vtCounts.WaitingDynamic,
		}
	}
	if !journalCounts.Detected {
		return p
	}
	p.Detected = true
	p.Source = "journal"
	p.Done = journalCounts.Done
	if p.Total > 0 && p.Done > p.Total {
		// journal は VT フレームより先に result を観測し得る。合成ビューが
		// 「6/5 完了」にならないよう表示値だけ丸める（journalCounts は生のまま）。
		p.Done = p.Total
	}
	if p.Total == 0 {
		p.Total = journalCounts.Total
		if journalCounts.Started > p.Total {
			p.Total = journalCounts.Started
		}
	}
	if p.Total > 0 {
		p.Percent = p.Done * 100 / p.Total
		if p.Percent > 100 {
			p.Percent = 100
		}
	}
	// A linked but incomplete journal suppresses VT settle. C3 can later set
	// journalCounts.SettledBy to journal or timeout; both outrank vt.
	p.Settled = journalCounts.Settled
	p.SettledBy = journalCounts.SettledBy
	return p
}

func workflowProgressSignature(p *proto.WorkflowProgress) string {
	if p == nil {
		return ""
	}
	var b strings.Builder
	fmt.Fprintf(&b, "%t|%s|%s|%d|%d|%d|%d|%d|%d|%d|%t|%s", p.Detected, p.Source, p.Name,
		p.Done, p.Total, p.Running, p.Failed, p.Pending, p.WaitingDynamic, p.Percent, p.Settled, p.SettledBy)
	for _, phase := range p.Phases {
		b.WriteString("|P:")
		b.WriteString(phase.Title)
		for _, agent := range phase.Agents {
			// Metrics frequently contain elapsed/token columns. They are payload,
			// not change-signature material; including them would recreate the
			// per-frame broadcast spam that the heartbeat design avoids.
			fmt.Fprintf(&b, "|A:%s:%s", agent.State, agent.Label)
		}
	}
	sum := sha256.Sum256([]byte(b.String()))
	return hex.EncodeToString(sum[:])
}

func cloneWorkflowProgress(p *proto.WorkflowProgress) *proto.WorkflowProgress {
	if p == nil {
		return nil
	}
	cp := *p
	cp.Phases = make([]proto.WfPhase, len(p.Phases))
	for i := range p.Phases {
		cp.Phases[i] = p.Phases[i]
		cp.Phases[i].Agents = append([]proto.WfAgent(nil), p.Phases[i].Agents...)
	}
	return &cp
}

func workflowScanDue(ses *session, requested time.Time) time.Time {
	due := requested
	if debounce := ses.workflowLastScanAt.Add(workflowVTScanDebounce); debounce.After(due) {
		due = debounce
	}
	if ses.vtResizeDebounceUntil.After(due) {
		due = ses.vtResizeDebounceUntil
	}
	return due
}

// queueWorkflowVTScanLocked schedules the earliest needed scan. Repeated PTY
// chunks move no work onto the hot path, while the retained timer guarantees a
// final scan after the last chunk in a debounce window.
func (s *Server) queueWorkflowVTScanLocked(id int, ses *session, requested time.Time) {
	if ses == nil || ses.Provider != "claude" {
		return
	}
	due := workflowScanDue(ses, requested)
	if ses.workflowScanTimer != nil && !due.Before(ses.workflowScanDue) {
		return
	}
	if ses.workflowScanTimer != nil {
		ses.workflowScanTimer.Stop()
	}
	ses.workflowScanGeneration++
	generation := ses.workflowScanGeneration
	ses.workflowScanDue = due
	delay := time.Until(due)
	if delay < 0 {
		delay = 0
	}
	expected := ses
	ses.workflowScanTimer = time.AfterFunc(delay, func() { s.runWorkflowVTScan(id, expected, generation) })
}

func (s *Server) runWorkflowVTScan(id int, expected *session, generation uint64) {
	now := time.Now()
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses != expected || ses.Provider != "claude" || ses.workflowScanGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	ses.workflowScanTimer = nil
	due := workflowScanDue(ses, now)
	if due.After(now) {
		s.queueWorkflowVTScanLocked(id, ses, due)
		s.sessionsMu.Unlock()
		return
	}
	ses.workflowLastScanAt = now
	var lines []string
	if ses.vt != nil {
		lines = ses.vt.TailLinesWithScrollback(workflowVTTailLines)
	}
	s.sessionsMu.Unlock()

	parsed := parseWorkflowVT(lines)
	s.applyWorkflowVTScan(id, expected, parsed, now)
}

func (s *Server) applyWorkflowVTScan(id int, expected *session, parsed *proto.WorkflowProgress, now time.Time) {
	journalEnabled := s.workflowJournalEnabled()
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses != expected || ses.Provider != "claude" {
		s.sessionsMu.Unlock()
		return
	}
	hasSignal := parsed != nil && (parsed.Detected || parsed.WaitingDynamic > 0)
	ses.workflowVTHasSignal = hasSignal
	if hasSignal {
		ses.workflowMissingScans = 0
		parsed = cloneWorkflowProgress(parsed)
		if parsed.Source == "vt-summary" && parsed.Total > 0 && parsed.Done == parsed.Total && parsed.WaitingDynamic == 0 {
			parsed.Settled = true
			parsed.SettledBy = "vt"
		}
		baseSig := workflowProgressSignature(parsed)
		sameFrame := baseSig == ses.workflowVTSignature
		// 経過時間の前進はライブ描画の証拠。stale scrollback フレームは経過が
		// 凍っているので、これで「同じ絵の再パース」と「実走行の再描画」を区別する。
		liveEvidence := parsed.ElapsedSec > ses.workflowElapsedBase
		if sameFrame && !parsed.Settled && !liveEvidence &&
			ses.workflowVTProgress != nil && ses.workflowVTProgress.Settled {
			// Sticky settle: 変化のないフレームは新情報を運ばない。ここで上書き
			// すると scrollback の stale サマリーが無関係な PTY 出力のたびに
			// settled を剥がして heartbeat を再開させる（F1 の再発経路）。
		} else {
			if !sameFrame || liveEvidence {
				ses.workflowElapsedBase = parsed.ElapsedSec
				ses.workflowElapsedObservedAt = now
			}
			ses.workflowVTSignature = baseSig
			ses.workflowVTProgress = parsed
			ses.vtCounts = workflowCountsFromProgress(parsed, now)
			if journalEnabled {
				s.startWorkflowJournalLocked(id, ses, now)
			}
		}
		// Frozen settle: 同一フレームのまま output-idle が続く＝実質終了。
		// 実走行中は PTY 出力が続き OutputIdle=false のため誤発火しない。
		if !sameFrame || liveEvidence || !ses.Activity.OutputIdle {
			ses.workflowFrozenScans = 0
		} else if ses.workflowVTProgress != nil && !ses.workflowVTProgress.Settled {
			ses.workflowFrozenScans++
			if ses.workflowFrozenScans >= workflowVTFrozenSettleMax {
				frozen := cloneWorkflowProgress(ses.workflowVTProgress)
				frozen.Running = 0
				frozen.Pending = 0
				frozen.WaitingDynamic = 0
				frozen.Settled = true
				frozen.SettledBy = "vt"
				ses.workflowVTProgress = frozen
				ses.vtCounts = workflowCountsFromProgress(frozen, now)
			}
		}
	} else if ses.workflowVTProgress != nil && !ses.workflowVTProgress.Settled {
		if ses.Activity.OutputIdle {
			ses.workflowMissingScans++
		} else {
			ses.workflowMissingScans = 0
		}
		if ses.workflowMissingScans >= workflowVTMissingSettleMax {
			parsed = cloneWorkflowProgress(ses.workflowVTProgress)
			parsed.Running = 0
			parsed.Pending = 0
			parsed.WaitingDynamic = 0
			parsed.Settled = true
			parsed.SettledBy = "vt"
			ses.workflowVTProgress = parsed
			ses.vtCounts = workflowCountsFromProgress(parsed, now)
		}
	}

	out, shouldBroadcast, shouldNotify := s.workflowProgressForBroadcastLocked(ses, now)
	if out == nil {
		s.sessionsMu.Unlock()
		return
	}
	if !out.Settled {
		s.queueWorkflowVTScanLocked(id, ses, now.Add(workflowVTHeartbeat))
	}
	s.sessionsMu.Unlock()
	if shouldBroadcast {
		s.broadcast(proto.Message{Type: "workflow_progress", SessionID: id, WorkflowProgress: out})
	}
	if shouldNotify {
		s.notifyWorkflowCompletionPush(id, out)
	}
}

// finalizeWorkflowOnSessionEnd force-settles a still-tracked workflow when its
// session terminates (session_end / wrapper 切断). A dead PTY can never emit the
// final frame while the stale VT mirror keeps the last summary visible, so
// without this the unsettled heartbeat re-queues itself indefinitely
// （敵対レビュー 2026-08-05 F1/F11）. Idempotent; completion push is deliberately
// suppressed（死んだセッションへ「完了」通知を出さない）.
func (s *Server) finalizeWorkflowOnSessionEnd(id int) {
	now := time.Now()
	var out *proto.WorkflowProgress
	shouldBroadcast := false
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	if ses.workflowScanTimer != nil {
		ses.workflowScanTimer.Stop()
		ses.workflowScanTimer = nil
	}
	ses.workflowScanGeneration++
	if ses.workflowJournalTimer != nil {
		ses.workflowJournalTimer.Stop()
		ses.workflowJournalTimer = nil
	}
	ses.workflowJournalGeneration++
	ses.workflowJournalRunning = false
	ses.workflowJournalDormant = false
	ses.workflowJournalDormantVTSignature = ""
	ses.workflowJournalPendingAssociation = false
	s.stopWorkflowTaskDetailLocked(ses)
	changed := false
	if ses.workflowVTProgress != nil && !ses.workflowVTProgress.Settled {
		p := cloneWorkflowProgress(ses.workflowVTProgress)
		p.Running = 0
		p.Pending = 0
		p.WaitingDynamic = 0
		p.Settled = true
		p.SettledBy = "timeout"
		ses.workflowVTProgress = p
		ses.vtCounts = workflowCountsFromProgress(p, now)
		changed = true
	}
	if ses.journalCounts.Detected && !ses.journalCounts.Settled {
		ses.journalCounts.Settled = true
		ses.journalCounts.SettledBy = "timeout"
		changed = true
	}
	if changed {
		// shouldNotify は意図的に破棄する（内部で notified 済みへ倒れるため、
		// 以後の経路からも完了 push は出ない）。
		out, shouldBroadcast, _ = s.workflowProgressForBroadcastLocked(ses, now)
	}
	s.sessionsMu.Unlock()
	if shouldBroadcast && out != nil {
		s.broadcast(proto.Message{Type: "workflow_progress", SessionID: id, WorkflowProgress: out})
	}
}

// workflowProgressForBroadcastLocked is the single C2/C3 composition path. It
// applies field authority, heartbeat/diff policy, and one-shot completion
// transitions while sessionsMu is held.
func (s *Server) workflowProgressForBroadcastLocked(ses *session, now time.Time) (*proto.WorkflowProgress, bool, bool) {
	out := composeWorkflowProgress(ses.workflowVTProgress, ses.vtCounts, ses.journalCounts)
	if out == nil {
		return nil, false, false
	}
	// internal C3: when the tasks-output taskId resolved and at least one poll
	// succeeded, swap in the richer Phases/WfAgent tree it derived. This never
	// touches Done/Total/Running/Percent/Settled/SettledBy above, which stay
	// owned by composeWorkflowProgress's existing VT/journal authority (D1).
	applyWorkflowTaskDetailOverlay(out, ses.taskDetailProgress)
	// Give the first journal association poll a chance to outrank a final VT
	// frame. If no unambiguous journal exists, the poll releases this guard and
	// VT-only settle proceeds as the documented degradation path.
	if ses.workflowJournalPendingAssociation && out.SettledBy == "vt" {
		out.Settled = false
		out.SettledBy = ""
	}
	if !out.Settled && ses.workflowElapsedBase > 0 && !ses.workflowElapsedObservedAt.IsZero() {
		out.ElapsedSec = ses.workflowElapsedBase + int(now.Sub(ses.workflowElapsedObservedAt)/time.Second)
	}
	sig := workflowProgressSignature(out)
	changed := sig != ses.workflowBroadcastSignature
	heartbeat := !out.Settled && now.Sub(ses.workflowLastBroadcastAt) >= workflowVTHeartbeat
	shouldBroadcast := changed || ses.workflowLastBroadcastAt.IsZero() || heartbeat
	if shouldBroadcast {
		ses.workflowBroadcastSignature = sig
		ses.workflowLastBroadcastAt = now
	}
	shouldNotify := false
	if out.Settled {
		if !ses.workflowCompletionNotified || ses.workflowCompletionSignature != sig {
			ses.workflowCompletionNotified = true
			ses.workflowCompletionSignature = sig
			shouldNotify = true
		}
	} else if ses.workflowCompletionNotified {
		ses.workflowCompletionNotified = false
		ses.workflowCompletionSignature = ""
	}
	return out, shouldBroadcast, shouldNotify
}
