package hub

// subagent_source_claude.go implements subagentReader for "subagent:claude-v1"
// (子 plan plan_subagent-tree-popup_c2_hub-core-claude.md 内部 C2). It reads the
// files Claude Code itself writes for a Task/Agent-tool subagent run — never
// the parent's PTY output — and turns them into the read-only
// proto.SubagentTree the popup displays (親 plan
// docs/local/plan_subagent-tree-popup.md 方針 1〜3, 5, 6).
//
// Three separate ID spaces are in play here, confirmed against real session
// records on this machine (2026-09-23, keys/enum values only — no prompt or
// result body left this investigation, see this C's "実装時の確定" note in the
// child plan for the raw counts):
//
//   - toolUseID ("toolu_...") — the Anthropic content-block id. It is what
//     meta.json's "toolUseId" field holds, what the parent's own Agent
//     tool_use block's "id" is, what the parent's matching tool_result
//     block's "tool_use_id" is, and (confirmed by direct comparison of a real
//     record) what a task-notification's <tool-use-id> tag holds. This is the
//     key used to correlate a child with its parent-side completion signal.
//   - the short per-agent id embedded in "agent-<id>.meta.json" /
//     "agent-<id>.jsonl" (e.g. "a169fe025668a41ee") — confirmed equal to the
//     child transcript's own top-level "agentId" field and to a
//     task-notification's <task-id> tag, but *not* equal to toolUseID. A
//     spawnDepth==2 child's "parentAgentId" field was confirmed (6/6 real
//     samples) to match this short id on one of its siblings, not a
//     toolUseID. This is therefore the id space SubagentNode.ID/ParentID use
//     for tree shape.
//   - the workflow run id ("wf_...") used by subagents/workflows/ — out of
//     scope here; those are the existing Workflow chip's children and are
//     filtered out below by directory position and by agentType.
//
// Foreground vs. background (親 plan C2 "前景・背景の見分け方"): confirmed via
// the parent transcript's own tool_result for the Agent tool_use.
// toolUseResult.isAsync is present and true exactly when the child was
// launched in the background — that record's toolUseResult.status is then
// "async_launched" (a launch acknowledgement, not a completion) and carries
// none of the completion-only fields (content/toolStats/totalDurationMs/...).
// When isAsync is absent, toolUseResult.status is the real, final status
// ("completed" in every real sample read; no direct "failed" sample was
// available, so a defensive substring check is kept for that case too).
//
// A backgrounded child's completion instead arrives later as a
// {"type":"queue-operation","operation":"enqueue","content":"<task-notification>...</task-notification>..."}
// record injected into the *parent's own* transcript. The inner XML-ish body
// carries (among fields this reader does not read, e.g. <result>/<summary>,
// which are the child's own output and out of scope per 方針 3)
// <tool-use-id>...</tool-use-id> and <status>completed|failed</status> — both
// confirmed against real records (285 notifications sampled, status only ever
// "completed" or "failed").
//
// Neither of those parent-side signals fires for a child the current
// parent-scan window (claudeSubagentParentReadBytesMax/RecordsMax tail of the
// parent, freshly resolved or reused per the caching described below) has no
// evidence for at all — e.g. an old Hub restart with no prior state, or a
// launch far enough back that the parent transcript has since scrolled it out
// of the tail window. For those, and only those, this reader falls back to
// "was the child's own transcript modified within the last 10 minutes"
// (claudeSubagentFreshnessWindow — same length as web/src/app/longproc.ts's
// STALL_SEC=600) at the moment it is first seen with no signal at all.
//
// State is a strict two-signal machine (C7, replacing an earlier "did the
// child's transcript grow since the last poll (3s ago)" rule that flickered a
// still-running child to "unknown" the instant it went quiet for one poll —
// e.g. while waiting on a long test or build — see this C's "実装時の確定" in
// the parent plan for the real-record evidence this replaced it with): a
// launch signal (a background isAsync launch ack, a foreground Agent
// tool_use with no result yet, or the 10-minute freshness fallback above)
// sets running; only an explicit completion signal (done/failed) ever clears
// it — inactivity never does. Both bits live per child in
// claudeSubagentChildCache and are carried forward across polls via
// subagentReader's prior/next contract, specifically so a launch or
// completion record that has since scrolled out of a later poll's
// parent-scan window cannot un-set what an earlier poll already established.
//
// The parent transcript itself is only re-scanned when its own size or mtime
// changed since the last poll (R6): claudeSubagentState caches the last
// scan's resolved signals keyed by toolUseID. claudeSubagentScanParentFn (a
// package var, not a bare function call) is the seam
// subagent_source_claude_test.go uses to count how many times a scan actually
// ran. claudeSubagentReadHead / claudeSubagentReadLastTool similarly report
// the exact byte count of every bounded child-transcript read they perform
// through claudeSubagentReadObserver, so the 64KB head / 128KB tail budgets
// are checked by direct measurement (R10) rather than inferred from
// downstream behavior.

import (
	"bytes"
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

const (
	// claudeSubagentParentReadBytesMax/RecordsMax bound the tail scan of the
	// *parent's* transcript used to resolve completion signals for every
	// child in one poll. Mirrors workflow_task_detail.go's
	// workflowTaskDetailReadBytesMax/RecordsMax (same shape of problem: find a
	// handful of marker records without reading a transcript that can reach
	// tens of MB). This is independent of subagentReadBudget's HeadBytes/
	// TailBytes, which bound the *child* jsonl reads instead (子 plan C2 完了条件
	// "10MB の合成 jsonl で...先頭 64KB と末尾 128KB を超えない" is about the child
	// file specifically).
	claudeSubagentParentReadBytesMax   = 2 * 1024 * 1024
	claudeSubagentParentReadRecordsMax = 500

	// claudeSubagentSummaryMax matches web/src/app/workflow-modal.ts'
	// clampForLine's default so LastToolSummary reads the same on both sides.
	claudeSubagentSummaryMax = 100

	claudeSubagentWorkflowAgentType = "workflow-subagent"

	// claudeSubagentFreshnessWindow is how recently a child's own jsonl must
	// have been modified, with zero parent-side launch/completion evidence
	// ever seen for it, to still be reported "running" instead of "unknown"
	// (C7 新しい規則). Same length as web/src/app/longproc.ts's STALL_SEC=600.
	claudeSubagentFreshnessWindow = 10 * time.Minute
)

var (
	claudeTaskNotificationRe          = regexp.MustCompile(`(?s)<task-notification>(.*?)</task-notification>`)
	claudeTaskNotificationToolUseIDRe = regexp.MustCompile(`<tool-use-id>\s*([^<\s]+)\s*</tool-use-id>`)
	claudeTaskNotificationStatusRe    = regexp.MustCompile(`<status>\s*([^<\s]+)\s*</status>`)
)

// claudeSubagentScanParentFn indirects claudeSubagentScanParent so
// subagent_source_claude_test.go can count how many times the parent
// transcript is actually scanned (C7 R6: "親の記録のサイズか更新時刻が前回から
// 変わったときだけ行う。変わっていなければ前回の信号を使う"). Production code
// never reassigns this var.
var claudeSubagentScanParentFn = claudeSubagentScanParent

// claudeSubagentReadObserver, when non-nil, receives the byte count of every
// bounded child-transcript read claudeSubagentReadHead/claudeSubagentReadLastTool
// perform ("head" or "tail" as kind). Left nil in production and never read
// there; subagent_source_claude_test.go sets it to directly measure that the
// head/tail budgets are honored (C7 R10), rather than inferring it from
// downstream behavior.
var claudeSubagentReadObserver func(kind string, n int64)

func claudeSubagentObserveRead(kind string, n int64) {
	if claudeSubagentReadObserver != nil {
		claudeSubagentReadObserver(kind, n)
	}
}

// claudeSubagentMeta is the intentionally narrow decode shape for an
// agent-<id>.meta.json file. Only structural/id fields are read; free-text
// fields Claude Code writes here beyond "description" (itself just the
// short label, not a prompt) are not decoded at all.
type claudeSubagentMeta struct {
	AgentType     string `json:"agentType"`
	Description   string `json:"description"`
	ToolUseID     string `json:"toolUseId"`
	SpawnDepth    int    `json:"spawnDepth"`
	ParentAgentID string `json:"parentAgentId,omitempty"`
	Model         string `json:"model,omitempty"`
}

// claudeSubagentChildCache is what this reader remembers per child (keyed by
// the short agent id) across polls, entirely in-memory via subagentReader's
// prior/next contract. Meta fields never change after Claude Code writes the
// file (this reader reads agent-*.meta.json exactly once per child, ever),
// so only the jsonl's own size/mtime need to be re-checked each poll; a
// child whose jsonl mtime+size are unchanged since the last poll is served
// entirely from cache with zero file reads that poll.
type claudeSubagentChildCache struct {
	Meta            claudeSubagentMeta
	JSONLModTime    time.Time
	JSONLSize       int64
	StartedAt       int64
	LastToolName    string
	LastToolSummary string
	ToolCalls       int
	ToolCallsKnown  bool

	// EverRunning and Resolved implement the C7 "起動の信号〜完了の信号" state
	// machine. Both are sticky: once true they stay true for the lifetime of
	// this child (across polls, regardless of whether the current poll's
	// parent-scan window still contains the original evidence, and
	// regardless of whether the child's own jsonl keeps growing). EverRunning
	// is set by any launch signal (isAsync launch ack, a foreground Agent
	// tool_use with no result yet, or — only when neither has ever been seen
	// — the claudeSubagentFreshnessWindow fallback). Resolved/Failed/
	// FinishedAt are set only by an explicit completion signal
	// (task-notification or a synchronous, non-async tool_result) and, once
	// set, are never recomputed.
	EverRunning bool
	Resolved    bool
	Failed      bool
	FinishedAt  int64
}

// claudeSubagentState is the concrete prior/next state readClaudeSubagentTree
// asserts out of subagentReader's `any` parameter/return (see
// internal/hub/subagent_tree.go's subagentReader doc for why that boundary is
// untyped).
type claudeSubagentState struct {
	Children map[string]*claudeSubagentChildCache

	// ParentModTime/ParentSize/ParentSignals cache the last parent-transcript
	// scan (C7 R6: "親の記録のサイズか更新時刻が前回から変わったときだけ行う").
	// ParentSignals nil means "never scanned yet", which always forces a
	// (re)scan — mirrors claudeSubagentChildCache's own cached==nil-means-
	// first-sight rule.
	ParentModTime time.Time
	ParentSize    int64
	ParentSignals map[string]claudeSubagentParentSignal
}

// claudeSubagentParentSignal is one child's parent-side evidence, resolved
// from the parent transcript's assistant/tool_result/task-notification
// records within the current poll's scan window (see file doc comment).
// Launched records a launch-only signal (起動); Resolved/Failed/FinishedAt
// record a completion signal (完了). A completion signal always wins: once
// Resolved is true for a toolUseID, later (chronologically older, since this
// scan runs newest-first) launch evidence for the same id is a no-op rather
// than a downgrade.
type claudeSubagentParentSignal struct {
	Launched   bool
	Resolved   bool
	Failed     bool
	FinishedAt int64 // epoch ms, from the signal record's own "timestamp" field
}

// claudeSubagentQueueOperationLine decodes a queue-operation transcript
// record far enough to find a task-notification payload. Every other
// queue-operation (e.g. plain user input being queued) has the same shape
// but content that never matches claudeTaskNotificationRe, so no separate
// operation-kind filtering beyond "operation == enqueue" is needed.
type claudeSubagentQueueOperationLine struct {
	Type      string `json:"type"`
	Operation string `json:"operation"`
	Timestamp string `json:"timestamp"`
	Content   string `json:"content"`
}

// claudeSubagentResultLine decodes a parent transcript "user" record (a
// tool_result envelope) far enough to reach its toolUseResult side-channel.
// Deliberately its own type rather than widening the shared
// claudeTranscriptLine (agent_chat_parse.go): that type is used by every
// provider-neutral chat reader and must not grow a field only this reader
// needs.
type claudeSubagentResultLine struct {
	Timestamp     string          `json:"timestamp"`
	ToolUseResult json.RawMessage `json:"toolUseResult"`
	Message       struct {
		Content json.RawMessage `json:"content"`
	} `json:"message"`
}

// claudeSubagentToolUseResult is the narrow decode of toolUseResult: only the
// two fields that decide foreground-vs-background and completion state.
type claudeSubagentToolUseResult struct {
	IsAsync bool   `json:"isAsync"`
	Status  string `json:"status"`
}

// claudeSubagentToolInputKeys is the allow-listed subset of a tool_use's
// input object LastToolSummary may ever be built from (親 plan 方針 3). Any
// other key present in input is decoded into nothing and never reaches this
// struct, let alone the wire.
type claudeSubagentToolInputKeys struct {
	Command  string `json:"command"`
	Pattern  string `json:"pattern"`
	Path     string `json:"path"`
	FilePath string `json:"file_path"`
}

// readClaudeSubagentTree is the subagent:claude-v1 entry in
// subagentReaderByKey (internal/hub/subagent_tree.go). transcriptPath is the
// parent's own Claude transcript path, already resolved by the caller via
// agent_log_handler.go's claudeTranscriptPath/findClaudeTranscript (子 plan
// C1 "親の記録の場所").
func readClaudeSubagentTree(transcriptPath string, since time.Time, prior any, budget subagentReadBudget) (*proto.SubagentTree, any, error) {
	state, _ := prior.(claudeSubagentState)
	if state.Children == nil {
		state.Children = map[string]*claudeSubagentChildCache{}
	}

	subDir := subagentDirForTranscript(transcriptPath)
	if subDir == "" {
		return nil, state, nil
	}
	entries, err := os.ReadDir(subDir)
	if err != nil {
		// Absent/unreadable subagents dir is the normal "no children yet"
		// state, not an error worth surfacing (方針 5: 読めないときは出さない).
		return nil, state, nil
	}

	nodes := make(map[string]proto.SubagentNode, len(entries))
	nextChildren := make(map[string]*claudeSubagentChildCache, len(state.Children))

	for _, e := range entries {
		if e.IsDir() {
			// Notably skips the "workflows" subdirectory itself: its
			// contents belong to the existing Workflow chip, not this tree
			// (親 plan 方針 6, 子 plan C2 完了条件).
			continue
		}
		name := e.Name()
		const prefix, suffix = "agent-", ".meta.json"
		if !strings.HasPrefix(name, prefix) || !strings.HasSuffix(name, suffix) {
			continue
		}
		id := strings.TrimSuffix(strings.TrimPrefix(name, prefix), suffix)
		if id == "" {
			continue
		}

		cached := state.Children[id]
		var meta claudeSubagentMeta
		if cached != nil {
			meta = cached.Meta
		} else {
			data, err := os.ReadFile(filepath.Join(subDir, name)) // #nosec G304 -- path built from the local Claude subagents dir, not user input.
			if err != nil || json.Unmarshal(data, &meta) != nil {
				continue
			}
		}
		if meta.AgentType == claudeSubagentWorkflowAgentType {
			continue
		}
		if meta.Description == "" || meta.ToolUseID == "" {
			// 子 plan C2: "キーが欠けていたら、その子は飛ばす".
			continue
		}
		if meta.SpawnDepth <= 0 {
			meta.SpawnDepth = 1
		}

		childPath := filepath.Join(subDir, prefix+id+".jsonl")
		info, statErr := os.Stat(childPath)
		if statErr != nil {
			continue
		}

		entry := &claudeSubagentChildCache{Meta: meta}
		if cached != nil {
			// EverRunning/Resolved/Failed/FinishedAt are sticky (C7): carry
			// them forward regardless of whether the child's own jsonl
			// changed this poll. Only the signals merge below is allowed to
			// set them further; nothing here ever clears them.
			entry.EverRunning = cached.EverRunning
			entry.Resolved = cached.Resolved
			entry.Failed = cached.Failed
			entry.FinishedAt = cached.FinishedAt
		}
		unchanged := cached != nil && cached.JSONLModTime.Equal(info.ModTime()) && cached.JSONLSize == info.Size()
		if unchanged {
			entry.JSONLModTime = cached.JSONLModTime
			entry.JSONLSize = cached.JSONLSize
			entry.StartedAt = cached.StartedAt
			entry.LastToolName = cached.LastToolName
			entry.LastToolSummary = cached.LastToolSummary
			entry.ToolCalls = cached.ToolCalls
			entry.ToolCallsKnown = cached.ToolCallsKnown
		} else {
			startedAt, toolCalls, toolCallsKnown := claudeSubagentReadHead(childPath, budget.HeadBytes, info.Size())
			toolName, toolSummary := claudeSubagentReadLastTool(childPath, budget.TailBytes)
			entry.JSONLModTime = info.ModTime()
			entry.JSONLSize = info.Size()
			entry.StartedAt = startedAt
			entry.LastToolName = toolName
			entry.LastToolSummary = toolSummary
			entry.ToolCalls = toolCalls
			entry.ToolCallsKnown = toolCallsKnown
		}
		nextChildren[id] = entry

		node := proto.SubagentNode{
			ID:              id,
			Depth:           meta.SpawnDepth,
			Label:           meta.Description,
			AgentType:       meta.AgentType,
			Model:           meta.Model,
			StartedAt:       entry.StartedAt,
			LastActivityAt:  info.ModTime().UnixMilli(),
			LastToolName:    entry.LastToolName,
			LastToolSummary: entry.LastToolSummary,
		}
		if entry.ToolCallsKnown {
			node.ToolCalls = entry.ToolCalls
		}
		if meta.SpawnDepth >= 2 && meta.ParentAgentID != "" {
			node.ParentID = meta.ParentAgentID
		}
		nodes[id] = node
	}
	state.Children = nextChildren

	if len(nodes) == 0 {
		return nil, state, nil
	}

	signals := claudeSubagentResolveParentSignals(transcriptPath, &state)
	now := time.Now()
	for id, node := range nodes {
		cache := nextChildren[id]
		if !cache.Resolved {
			if sig, ok := signals[cache.Meta.ToolUseID]; ok {
				switch {
				case sig.Resolved:
					cache.Resolved = true
					cache.Failed = sig.Failed
					cache.FinishedAt = sig.FinishedAt
					cache.EverRunning = true
				case sig.Launched:
					cache.EverRunning = true
				}
			}
			if !cache.Resolved && !cache.EverRunning && now.Sub(cache.JSONLModTime) <= claudeSubagentFreshnessWindow {
				// C7 新しい規則: 起動も完了も一度も見ていない子は、記録の更新が
				// claudeSubagentFreshnessWindow 以内なら running とみなす
				// (親の記録の読み取り窓の外で起動した子のため). Sticky from
				// here on via EverRunning, same as a genuine launch signal.
				cache.EverRunning = true
			}
		}
		switch {
		case cache.Resolved && cache.Failed:
			node.State = "failed"
			node.FinishedAt = cache.FinishedAt
		case cache.Resolved:
			node.State = "done"
			node.FinishedAt = cache.FinishedAt
		case cache.EverRunning:
			node.State = "running"
		default:
			node.State = "unknown"
		}
		nodes[id] = node
	}

	// 孫の親が(そもそも欠落 or since/cap で)いなければ木に出さない (子 plan C2 完了
	// 条件 + 方針 5). Runs before *and* after the since/cap passes below so it
	// catches both "meta pointed at a parent that was never a valid node" and
	// "the parent existed but got filtered/dropped".
	claudeSubagentDropOrphans(nodes)

	sinceMs := since.UnixMilli()
	for id, node := range nodes {
		if node.State != "running" && node.StartedAt < sinceMs {
			delete(nodes, id)
		}
	}
	claudeSubagentDropOrphans(nodes)

	omitted := 0
	for len(nodes) > budget.MaxNodes {
		victim := claudeSubagentPickDropVictim(nodes)
		if victim == "" {
			break
		}
		delete(nodes, victim)
		omitted++
		before := len(nodes)
		claudeSubagentDropOrphans(nodes)
		omitted += before - len(nodes)
	}

	ordered := make([]proto.SubagentNode, 0, len(nodes))
	for _, node := range nodes {
		ordered = append(ordered, node)
	}
	sort.Slice(ordered, func(i, j int) bool {
		if ordered[i].StartedAt != ordered[j].StartedAt {
			return ordered[i].StartedAt < ordered[j].StartedAt
		}
		return ordered[i].ID < ordered[j].ID
	})

	if len(ordered) == 0 {
		return nil, state, nil
	}
	return &proto.SubagentTree{
		Provider:  "subagent:claude-v1",
		Nodes:     ordered,
		Omitted:   omitted,
		UpdatedAt: time.Now().UnixMilli(),
	}, state, nil
}

// claudeSubagentDropOrphans removes any node whose ParentID does not name a
// node still present in nodes, looping until stable so a cascade (dropping a
// depth-1 node also drops its depth-2 children) converges in one call.
func claudeSubagentDropOrphans(nodes map[string]proto.SubagentNode) {
	for {
		changed := false
		for id, n := range nodes {
			if n.ParentID == "" {
				continue
			}
			if _, ok := nodes[n.ParentID]; !ok {
				delete(nodes, id)
				changed = true
			}
		}
		if !changed {
			return
		}
	}
}

// claudeSubagentPickDropVictim chooses which node the 50-node cap drops next
// (子 plan C2 "落とすときは古い完了から落とす"). It prefers the oldest-finished
// done/failed node; only if none remain (every surviving node is still
// running/unknown, which should not happen once the cap is that low in
// practice) does it fall back to the oldest-started node of any state, so the
// cap is always honored.
func claudeSubagentPickDropVictim(nodes map[string]proto.SubagentNode) string {
	var victim string
	var bestFinished int64 = -1
	for id, n := range nodes {
		if n.State != "done" && n.State != "failed" {
			continue
		}
		if victim == "" || n.FinishedAt < bestFinished || (n.FinishedAt == bestFinished && id < victim) {
			victim, bestFinished = id, n.FinishedAt
		}
	}
	if victim != "" {
		return victim
	}
	var bestStarted int64 = -1
	for id, n := range nodes {
		if victim == "" || n.StartedAt < bestStarted || (n.StartedAt == bestStarted && id < victim) {
			victim, bestStarted = id, n.StartedAt
		}
	}
	return victim
}

// claudeSubagentResolveParentSignals returns the parent transcript's launch/
// completion signals, re-scanning only when the parent file's own size or
// mtime changed since the last poll (C7 R6). state.ParentSignals is reused
// as-is otherwise, so an unchanged parent never gets reopened.
func claudeSubagentResolveParentSignals(transcriptPath string, state *claudeSubagentState) map[string]claudeSubagentParentSignal {
	info, statErr := os.Stat(transcriptPath)
	if statErr == nil && state.ParentSignals != nil && state.ParentModTime.Equal(info.ModTime()) && state.ParentSize == info.Size() {
		return state.ParentSignals
	}
	signals := claudeSubagentScanParentFn(transcriptPath)
	state.ParentSignals = signals
	if statErr == nil {
		state.ParentModTime = info.ModTime()
		state.ParentSize = info.Size()
	} else {
		state.ParentModTime = time.Time{}
		state.ParentSize = 0
	}
	return signals
}

// claudeSubagentScanParent reads a bounded tail page of the parent's own
// transcript and resolves whatever launch/completion signals it can find,
// keyed by toolUseID (see file doc comment for why that id space, not the
// short per-agent id). It never reads body text: only the Agent tool_use's
// own id, tool_result.tool_use_id + toolUseResult.{isAsync,status}, and
// task-notification's <tool-use-id>/<status> tags.
func claudeSubagentScanParent(transcriptPath string) map[string]claudeSubagentParentSignal {
	signals := map[string]claudeSubagentParentSignal{}
	if transcriptPath == "" {
		return signals
	}
	budget := agentChatReadBudget{
		MaxBytes:   claudeSubagentParentReadBytesMax,
		MaxRecords: claudeSubagentParentReadRecordsMax,
		Deadline:   time.Now().Add(agentChatReadTimeBudget),
	}
	records, _, err := readAgentChatTailPageWithBudget(transcriptPath, claudeSubagentParentReadRecordsMax, -1, budget)
	if err != nil {
		return signals
	}
	for _, rec := range records {
		var sniff struct {
			Type string `json:"type"`
		}
		if json.Unmarshal(rec.line, &sniff) != nil {
			continue
		}
		switch sniff.Type {
		case "queue-operation":
			var qop claudeSubagentQueueOperationLine
			if json.Unmarshal(rec.line, &qop) != nil || qop.Operation != "enqueue" || qop.Content == "" {
				continue
			}
			claudeSubagentApplyNotification(qop.Content, claudeSubagentParseTimestamp(qop.Timestamp), signals)
		case "assistant":
			// 起動（表）(C7): the parent's own Agent tool_use block. Its mere
			// presence is launch evidence regardless of whether a result has
			// arrived yet; claudeSubagentMarkLaunched leaves an
			// already-resolved id alone (this scan is newest-first, so a
			// completion for this id may already have been recorded above).
			var line claudeTranscriptLine
			if json.Unmarshal(rec.line, &line) != nil {
				continue
			}
			var blocks []anthropicContentBlock
			if json.Unmarshal(line.Message.Content, &blocks) != nil {
				continue
			}
			for _, b := range blocks {
				if b.Type == "tool_use" && b.ID != "" {
					claudeSubagentMarkLaunched(signals, b.ID)
				}
			}
		case "user":
			var res claudeSubagentResultLine
			if json.Unmarshal(rec.line, &res) != nil || len(res.ToolUseResult) == 0 {
				continue
			}
			var blocks []anthropicContentBlock
			if json.Unmarshal(res.Message.Content, &blocks) != nil {
				continue
			}
			var toolUseID string
			for _, b := range blocks {
				if b.Type == "tool_result" && b.ToolUseID != "" {
					toolUseID = b.ToolUseID
					break
				}
			}
			if toolUseID == "" {
				continue
			}
			var tur claudeSubagentToolUseResult
			if json.Unmarshal(res.ToolUseResult, &tur) != nil {
				continue
			}
			if tur.IsAsync {
				// 起動（裏）(C7): a launch acknowledgement only
				// ("async_launched" in every real sample read), not a
				// completion — the real completion arrives later via a
				// task-notification queue-operation, handled above.
				claudeSubagentMarkLaunched(signals, toolUseID)
				continue
			}
			if done, failed := claudeSubagentClassifyStatus(tur.Status); done {
				claudeSubagentMarkResolved(signals, toolUseID, failed, claudeSubagentParseTimestamp(res.Timestamp))
			} else {
				// A synchronous tool_result exists but its status is not one
				// of the two recognized terminal values: 起動（表） evidence
				// without a completion signal this reader is willing to act
				// on (方針 5: 読めないときは出さない).
				claudeSubagentMarkLaunched(signals, toolUseID)
			}
		}
	}
	return signals
}

// claudeSubagentApplyNotification extracts a <task-notification> block's
// <tool-use-id>/<status> tags and folds them into signals as a 完了 (C7)
// signal. It never captures any of the notification's other tags (<result>,
// <summary>, ...): those are the child's own output text and out of scope
// (親 plan 方針 3).
func claudeSubagentApplyNotification(content string, ts int64, signals map[string]claudeSubagentParentSignal) {
	m := claudeTaskNotificationRe.FindStringSubmatch(content)
	if len(m) < 2 {
		return
	}
	inner := m[1]
	idMatch := claudeTaskNotificationToolUseIDRe.FindStringSubmatch(inner)
	statusMatch := claudeTaskNotificationStatusRe.FindStringSubmatch(inner)
	if len(idMatch) < 2 || len(statusMatch) < 2 {
		return
	}
	if done, failed := claudeSubagentClassifyStatus(statusMatch[1]); done {
		claudeSubagentMarkResolved(signals, idMatch[1], failed, ts)
	} else {
		claudeSubagentMarkLaunched(signals, idMatch[1])
	}
}

// claudeSubagentClassifyStatus maps a toolUseResult/task-notification status
// string to a resolved done/failed pair. "completed" is done; "fail"-containing
// values and the other terminal-but-unsuccessful values (a child the user
// stopped shows up as "killed" — 2 of 300 real task-notifications on
// 2026-09-23) are failed. Anything else (e.g. "async_launched") is "not a
// completion yet". Treating "killed" as a launch signal left a stopped child
// running forever, which also kept the turn cutoff from ever advancing.
func claudeSubagentClassifyStatus(status string) (done, failed bool) {
	st := strings.ToLower(strings.TrimSpace(status))
	switch {
	case st == "completed":
		return true, false
	case strings.Contains(st, "fail"),
		st == "killed", st == "stopped", st == "cancelled", st == "canceled",
		st == "aborted", st == "interrupted", st == "error", st == "timeout", st == "timed_out":
		return true, true
	default:
		return false, false
	}
}

// claudeSubagentMarkLaunched records a 起動 (launch-only) signal for
// toolUseID: a backgrounded async_launched acknowledgement, a foreground
// Agent tool_use block, or a synchronous tool_result whose status isn't a
// recognized terminal value. It never downgrades an already-resolved
// (完了/done/failed) signal — this scan runs newest-first, so launch
// evidence for an id that has since completed is expected, not a
// contradiction, and must not un-resolve it.
func claudeSubagentMarkLaunched(signals map[string]claudeSubagentParentSignal, toolUseID string) {
	if toolUseID == "" {
		return
	}
	sig := signals[toolUseID]
	if sig.Resolved {
		return
	}
	sig.Launched = true
	signals[toolUseID] = sig
}

// claudeSubagentMarkResolved records a terminal 完了 (done/failed) signal.
// Only the first one seen wins (this scan visits records newest-first, so
// "first seen" is "most recent"); once Resolved is set for a toolUseID,
// later (chronologically older, in scan order) resolution attempts for the
// same id are no-ops.
func claudeSubagentMarkResolved(signals map[string]claudeSubagentParentSignal, toolUseID string, failed bool, finishedAt int64) {
	if toolUseID == "" {
		return
	}
	if signals[toolUseID].Resolved {
		return
	}
	signals[toolUseID] = claudeSubagentParentSignal{Launched: true, Resolved: true, Failed: failed, FinishedAt: finishedAt}
}

// claudeSubagentReadHead reads at most headBytes from the start of a child
// transcript. When the whole file already fits in that budget it also counts
// every tool_use block while it is right there (ToolCalls "known" case); once
// a child's transcript outgrows headBytes this always returns
// toolCallsKnown=false for that child, for good, rather than issuing any
// further read beyond the declared head/tail budget to complete the count
// (子 plan C2 "初回に...数えきれていないので、ToolCalls を出さない").
func claudeSubagentReadHead(path string, headBytes int64, size int64) (startedAt int64, toolCalls int, toolCallsKnown bool) {
	if headBytes <= 0 {
		headBytes = defaultSubagentReadBudget().HeadBytes
	}
	if size <= headBytes {
		data, err := os.ReadFile(path) // #nosec G304 -- size already confirmed <= headBytes just above.
		if err != nil {
			return 0, 0, false
		}
		claudeSubagentObserveRead("head", int64(len(data)))
		first := true
		for _, ln := range bytes.Split(data, []byte("\n")) {
			if len(bytes.TrimSpace(ln)) == 0 {
				continue
			}
			var line claudeTranscriptLine
			if json.Unmarshal(ln, &line) != nil {
				continue
			}
			if first {
				startedAt = claudeSubagentParseTimestamp(line.Timestamp)
				first = false
			}
			if line.Type != "assistant" {
				continue
			}
			var blocks []anthropicContentBlock
			if json.Unmarshal(line.Message.Content, &blocks) != nil {
				continue
			}
			for _, b := range blocks {
				if b.Type == "tool_use" {
					toolCalls++
				}
			}
		}
		return startedAt, toolCalls, true
	}

	f, err := os.Open(path) // #nosec G304 -- path derived from the local Claude subagents dir, not user input.
	if err != nil {
		return 0, 0, false
	}
	defer func() { _ = f.Close() }()
	buf := make([]byte, headBytes)
	n, readErr := io.ReadFull(f, buf)
	if readErr != nil && readErr != io.ErrUnexpectedEOF {
		return 0, 0, false
	}
	claudeSubagentObserveRead("head", int64(n))
	data := buf[:n]
	if nl := bytes.IndexByte(data, '\n'); nl >= 0 {
		data = data[:nl]
	}
	var line claudeTranscriptLine
	if json.Unmarshal(data, &line) != nil {
		return 0, 0, false
	}
	return claudeSubagentParseTimestamp(line.Timestamp), 0, false
}

// claudeSubagentReadLastTool reads at most tailBytes from the end of a child
// transcript and returns the most recent assistant tool_use's name and an
// allow-listed, truncated summary of its input.
func claudeSubagentReadLastTool(path string, tailBytes int64) (name, summary string) {
	if tailBytes <= 0 {
		tailBytes = defaultSubagentReadBudget().TailBytes
	}
	budget := agentChatReadBudget{
		MaxBytes:   tailBytes,
		MaxRecords: agentChatPageRecordsMax,
		Deadline:   time.Now().Add(agentChatReadTimeBudget),
	}
	records, stats, err := readAgentChatTailPageWithBudget(path, agentChatPageRecordsMax, -1, budget)
	if err != nil {
		return "", ""
	}
	// BytesRead is an actual I/O count (readAgentChatRange's own doc: "Direct
	// file reads make BytesRead an actual I/O cap"), so this is a direct
	// measurement of the tail budget, not an inference (C7 R10).
	claudeSubagentObserveRead("tail", stats.BytesRead)
	for _, rec := range records { // newest-first
		var line claudeTranscriptLine
		if json.Unmarshal(rec.line, &line) != nil || line.Type != "assistant" {
			continue
		}
		var blocks []anthropicContentBlock
		if json.Unmarshal(line.Message.Content, &blocks) != nil {
			continue
		}
		for _, b := range blocks {
			if b.Type != "tool_use" {
				continue
			}
			return b.Name, claudeSubagentToolSummary(b.Input)
		}
	}
	return "", ""
}

// claudeSubagentToolSummary picks the first allow-listed key present (in the
// fixed priority order 親 plan 方針 3 names: command / pattern / path /
// file_path) out of a tool_use's input and clamps it. Any other key in input
// is never decoded into this struct at all, so it cannot leak here.
func claudeSubagentToolSummary(input json.RawMessage) string {
	if len(input) == 0 {
		return ""
	}
	var keys claudeSubagentToolInputKeys
	if json.Unmarshal(input, &keys) != nil {
		return ""
	}
	for _, v := range []string{keys.Command, keys.Pattern, keys.Path, keys.FilePath} {
		if v != "" {
			return claudeSubagentClampSummary(v)
		}
	}
	return ""
}

// claudeSubagentClampSummary mirrors web/src/app/workflow-modal.ts'
// clampForLine exactly (collapse whitespace runs to single spaces, trim, cap
// at claudeSubagentSummaryMax runes with a trailing "…") so the same value
// looks identical whichever side renders it.
func claudeSubagentClampSummary(raw string) string {
	s := strings.Join(strings.Fields(raw), " ")
	runes := []rune(s)
	if len(runes) <= claudeSubagentSummaryMax {
		return s
	}
	return string(runes[:claudeSubagentSummaryMax-1]) + "…"
}

// claudeSubagentParseTimestamp parses a Claude Code transcript record's
// "timestamp" field (RFC3339 with fractional seconds, e.g.
// "2026-09-23T12:34:56.789Z") into epoch ms. Returns 0 (not an error) on any
// unparseable/empty input, consistent with 方針 5's "読めないときは出さない":
// callers treat a zero StartedAt/FinishedAt as "unknown", not "epoch zero".
func claudeSubagentParseTimestamp(ts string) int64 {
	if ts == "" {
		return 0
	}
	if t, err := time.Parse(time.RFC3339Nano, ts); err == nil {
		return t.UnixMilli()
	}
	if t, err := time.Parse(time.RFC3339, ts); err == nil {
		return t.UnixMilli()
	}
	return 0
}
