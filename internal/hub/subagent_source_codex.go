package hub

// subagent_source_codex.go implements subagentReader for "subagent:codex-v1"
// (親 plan docs/local/plan_subagent-tree-popup.md C3). It reads the rollout
// JSONL files Codex itself writes under CODEX_HOME/sessions/YYYY/MM/DD/ — the
// parent's own rollout for completion signals, and each spawned child's own
// separate rollout file for its identity and current activity — never the
// parent's PTY output (親 plan 方針 1).
//
// Key names/shapes below were confirmed 2026-09-23 against real
// ~/.codex/sessions and ~/.many-ai-cli/subscriptions/codex/*/sessions rollout
// files (key names, enum values, and record counts only — no prompt, tool
// result, or reply body was read or left this investigation; see this C's
// "実装時の確定" note in the parent plan for the raw counts):
//
//   - A rollout's own thread id is session_meta's top-level payload.id
//     (present in every session_meta sampled). A spawned child's own
//     session_meta additionally carries
//     payload.source.subagent.thread_spawn{agent_nickname, agent_role,
//     parent_thread_id, depth} (96/96 subagent session_metas sampled had all
//     four keys). parent_thread_id was confirmed, by direct id lookup, to
//     always match some other rollout's own payload.id — for a depth==1
//     child that is the actual top-level (non-subagent) session; for the one
//     depth==2 sample found, it was instead a depth==1 *sibling's* own id,
//     not the top-level session's id. So child discovery cannot filter on
//     "parent_thread_id == the root's own id" alone (that misses every
//     grandchild+) — it must instead collect every subagent-shaped rollout
//     found in the scanned window and connect it to the root by walking
//     parent_thread_id chains (codexSubagentConnectedToRoot below), the same
//     transitive-connection idea 子 plan C2's claudeSubagentDropOrphans
//     applies after the fact for Claude's parentAgentId chains.
//   - Parent-side state signal (修正 C8, 親 plan
//     docs/local/plan_subagent-tree-popup.md §C8 — replaces the original C3
//     "did the child's rollout grow since the last poll" running/unknown
//     fallback, which flickered to unknown every poll a child went quiet
//     while waiting on a long shell command, and dropped children whose
//     growth had gone stale before a display-range cutoff (指摘 R1)): the
//     parent's own rollout carries event_msg records shaped
//     {"type":"event_msg","payload":{"type":"item_completed","item":{"type":"SubAgentActivity","kind":...,"agent_thread_id":...},"completed_at_ms":...}}.
//     kind is one of started/interacted/completed/interrupted (206 samples
//     read, see this C's "実装時の確定" note in the parent plan). No kind
//     value distinct from "interrupted" was observed standing for failure,
//     so "interrupted" is kept as the failed signal (already true of the
//     pre-C8 code). agent_thread_id was confirmed, in every one of those 206
//     samples, to equal a child's own payload.id.
//     "started" is now the authoritative launch signal: once seen for a
//     child (in any poll, however long ago), that child's cached
//     SeenStarted latches true and the child is reported "running" on every
//     later poll until a "completed"/"interrupted" record is seen for it —
//     never merely because its own rollout stopped growing. "completed"
//     (-> done) and "interrupted" (-> failed) are likewise latched the first
//     time they are seen (codexSubagentChildCache.Terminal) and never
//     revisited afterward, so a child does not un-finish just because its
//     finishing record later scrolls outside a later poll's bounded tail
//     scan of the parent rollout. "interacted" contributes to neither
//     latch (親 plan says only started/completed decide state; interacted
//     already implies discovery + activity, which the other two signals
//     already cover). A child whose own "started" record already sits
//     outside the parent's bounded read window the first time this Hub ever
//     sees it (e.g. a Hub restart, or a very long parent transcript) falls
//     back to codexSubagentUnseenSignalWindow: running if its own rollout
//     was modified within that window at the moment of first sight, unknown
//     otherwise — decided once, at first sight, and then frozen the same
//     way Terminal is (親 plan「修正の C の共通の決まり」).
//   - Current tool: the child's OWN rollout tail (never the parent's, never
//     read past subagentReadBudget.TailBytes), scanned for response_item
//     records. Two payload.type shapes carry a tool call:
//     "custom_tool_call" (payload.name/payload.input — the only name
//     observed in 2,215 real samples was "exec", the shell tool, with
//     payload.input a plain JSON string holding the command itself, not an
//     object) and "function_call" (payload.name/payload.arguments, a
//     JSON-encoded object string — the only names observed, 138 real
//     samples, were inter-agent coordination calls: send_message/wait/
//     wait_agent/list_agents/spawn_agent). LastToolName is always
//     payload.name. LastToolSummary is populated only for custom_tool_call:
//     its input *is* the allow-listed value (there is no further key to
//     filter down to, unlike Claude's command/pattern/path/file_path
//     object), clamped the same way Claude's summary is (方針 3). For
//     function_call, every real argument key sampled (message, target,
//     yield_time_ms, max_tokens, cell_id, timeout_ms, terminate, fork_turns,
//     task_name) is either inter-agent chat content ("message") or a bare
//     control number — none is an allow-listed command/pattern/path shape —
//     so LastToolSummary is left empty for function_call, per 方針 5 ("読め
//     ないときは出さない" applied to "no allow-listed key exists here", the
//     same call Claude's claudeSubagentToolSummary makes for a tool_use
//     input with no allow-listed key present).

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

const (
	// codexSubagentSessionMetaLineMax bounds a single rollout's first-line
	// read during discovery (子 plan C3 完了条件「子の探索で読むのは各
	// rollout の先頭行だけ」— this is that read, bounded the same way
	// agent_log_handler.go's readCodexSessionMeta already bounds its own
	// first-line-only read of the exact same file shape).
	codexSubagentSessionMetaLineMax = 4 * 1024 * 1024

	// codexSubagentParentReadBytesMax/RecordsMax bound the tail scan of the
	// *parent's* rollout used to resolve SubAgentActivity completion signals
	// for every child in one poll — independent of subagentReadBudget's
	// HeadBytes/TailBytes, which bound the *child* rollout reads instead
	// (mirrors claudeSubagentParentReadBytesMax/RecordsMax's same split).
	codexSubagentParentReadBytesMax   = 2 * 1024 * 1024
	codexSubagentParentReadRecordsMax = 500

	// codexSubagentMaxDayDirs bounds how many sessions/YYYY/MM/DD day
	// directories one poll's discovery walk will ever consider (from the
	// parent's own start date through today), so an unusually long-running
	// session cannot force an unbounded directory walk.
	codexSubagentMaxDayDirs = 32

	// codexSubagentUnseenSignalWindow bounds the "起動も完了も一度も見てい
	// ない子" fallback (親 plan「修正の C の共通の決まり」, 修正 C8): matches
	// web/src/app/longproc.ts's STALL_SEC = 600 — the same "still recent
	// enough to call it running without a direct SubAgentActivity signal"
	// window. Used only the first time a child is ever observed and never
	// re-evaluated afterward (see codexSubagentChildCache.FirstObservationDecided).
	codexSubagentUnseenSignalWindow = 10 * time.Minute
)

// codexRolloutSessionMeta is the intentionally narrow decode shape for a
// rollout's first line (type=="session_meta"). Only structural/id fields are
// read; the same line also carries base_instructions, dynamic_tools schemas,
// and other large fields this reader never even declares, let alone decodes.
type codexRolloutSessionMeta struct {
	Type    string `json:"type"`
	Payload struct {
		ID        string             `json:"id"`
		Timestamp string             `json:"timestamp"`
		Source    codexRolloutSource `json:"source"`
	} `json:"payload"`
}

type codexRolloutSource struct {
	Subagent struct {
		ThreadSpawn *codexThreadSpawn `json:"thread_spawn"`
	} `json:"subagent"`
}

// codexThreadSpawn is payload.source.subagent.thread_spawn, present on a
// spawned child's own session_meta only (a non-subagent, top-level session's
// source.subagent is absent entirely).
type codexThreadSpawn struct {
	AgentNickname  string `json:"agent_nickname"`
	AgentRole      string `json:"agent_role"`
	ParentThreadID string `json:"parent_thread_id"`
	Depth          int    `json:"depth"`
}

// codexSubagentChildCache is what this reader remembers per discovered
// subagent-shaped rollout (keyed by its own thread id) across polls, via
// subagentReader's prior/next contract. Meta fields never change after Codex
// writes the file's first line, so only the rollout's own size/mtime need
// re-checking each poll. Entries here are *candidates*: connectivity to the
// current session's own root id is recomputed every poll by
// codexSubagentConnectedToRoot, not assumed at discovery time, because a
// depth>=2 child's parent_thread_id names another child, not the root (see
// file doc comment).
type codexSubagentChildCache struct {
	Path            string
	Nickname        string
	Role            string
	ParentThreadID  string
	Depth           int
	StartedAt       int64 // epoch ms, from this child's own session_meta payload.timestamp
	JSONLModTime    time.Time
	JSONLSize       int64
	LastToolName    string
	LastToolSummary string
	// Grew is recomputed every poll (true iff the rollout's size/mtime
	// changed since the previous poll, or this is the first poll this Hub
	// process has seen it). It exists only to decide whether
	// codexSubagentReadLastTool needs to re-scan this child's own tail this
	// poll (transcript_stall.go's "first observation counts as grew"
	// convention). Since 修正 C8 it no longer feeds running/unknown state —
	// that used to flicker to "unknown" the moment a child's own rollout
	// stopped growing for a poll or two (指摘 R1); see SeenStarted below for
	// the replacement.
	Grew bool

	// --- 修正 C8 (親 plan §C8): state is decided from SubAgentActivity
	// signals, latched, and never re-derived from Grew. ---

	// SeenStarted becomes true the first time this reader observes a
	// SubAgentActivity item_completed record with kind=="started" for this
	// child, in any poll, and never reverts to false. A child with
	// SeenStarted true is reported "running" until Terminal latches, no
	// matter how long its own rollout goes without a new line.
	SeenStarted bool
	// Terminal/Failed/FinishedAt latch the first time this reader sees a
	// completed ("interrupted", see file doc comment)/failed
	// SubAgentActivity record for this child, and never change afterward —
	// even once that record scrolls outside a later poll's bounded tail
	// scan of the parent rollout (codexSubagentParentReadBytesMax/
	// RecordsMax). A done/failed child must not revert to running/unknown
	// just because its finishing signal aged out of that window.
	Terminal   bool
	Failed     bool
	FinishedAt int64
	// FirstObservationDecided/FirstObservationRunning implement the "起動も
	// 完了も一度も見ていない子" fallback (親 plan「修正の C の共通の決ま
	// り」): decided exactly once, the first poll this child is connected to
	// the current root and neither SeenStarted nor Terminal is true yet,
	// from whether its own rollout was modified within
	// codexSubagentUnseenSignalWindow of that moment — covering a child
	// whose "started" SubAgentActivity record already sits outside the
	// parent's bounded read window by the time this Hub first sees it. The
	// decision is frozen the same way Terminal is, so it cannot flicker poll
	// to poll either.
	FirstObservationDecided bool
	FirstObservationRunning bool
}

// codexSubagentState is the concrete prior/next state readCodexSubagentTree
// asserts out of subagentReader's `any` parameter/return.
type codexSubagentState struct {
	Children map[string]*codexSubagentChildCache
	// DayDirs remembers, per sessions/YYYY/MM/DD directory already scanned,
	// the ModTime last observed there — so 「一度つながった子のパスは覚えて
	// おき、毎回ディレクトリを全部見直さない。見直すのは sessions/ 配下の
	// 当日ディレクトリの mtime が変わったときだけにする」holds
	// (codexSubagentScanDayDirs below): a past (non-today) day directory is
	// scanned at most once, ever, no matter how its mtime changes later;
	// only today's directory is ever rescanned, and then only when its
	// ModTime differs from the value recorded here.
	DayDirs map[string]time.Time
}

// codexSubagentParentSignal is what one poll's bounded scan of the parent
// rollout found for one child's SubAgentActivity records (see file doc
// comment). It is intentionally not "the" signal but an accumulation over
// every matching record found *this poll*: Started is a plain OR (any
// "started" record seen this poll sets it, regardless of order), while
// Terminal/Failed/FinishedAt latch onto the first (i.e., since records are
// scanned newest-first, the most recent) completed/interrupted record seen.
// The caller (readCodexSubagentTree) folds both into the child's persistent
// codexSubagentChildCache, which is what actually survives across polls —
// this struct itself is rebuilt from scratch on every call and never stored.
type codexSubagentParentSignal struct {
	Started    bool
	Terminal   bool
	Failed     bool  // meaningful only when Terminal
	FinishedAt int64 // epoch ms, from the record's own completed_at_ms
}

// codexEventMsgItemCompleted decodes a parent rollout "event_msg" record far
// enough to reach a SubAgentActivity item_completed's kind and target child.
type codexEventMsgItemCompleted struct {
	Type    string `json:"type"`
	Payload struct {
		Type          string `json:"type"`
		CompletedAtMs int64  `json:"completed_at_ms"`
		Item          struct {
			Type          string `json:"type"`
			Kind          string `json:"kind"`
			AgentThreadID string `json:"agent_thread_id"`
		} `json:"item"`
	} `json:"payload"`
}

// codexResponseItemToolCall decodes a child rollout "response_item" record
// far enough to reach whichever of the two tool-call shapes it might be (see
// file doc comment for why both are read and why only one is summarized).
type codexResponseItemToolCall struct {
	Type    string `json:"type"`
	Payload struct {
		Type      string          `json:"type"`
		Name      string          `json:"name"`
		Input     json.RawMessage `json:"input"`     // custom_tool_call only
		Arguments json.RawMessage `json:"arguments"` // function_call only; deliberately never decoded further (方針 3)
	} `json:"payload"`
}

// readCodexSubagentTree is the subagent:codex-v1 entry in
// subagentReaderByKey (internal/hub/subagent_tree.go). transcriptPath is the
// parent's own Codex rollout path, already resolved by the caller via
// resolveCodexSubagentParentPath (親 plan C3 「親の rollout のパスは...で得
// る」).
func readCodexSubagentTree(transcriptPath string, since time.Time, prior any, budget subagentReadBudget) (*proto.SubagentTree, any, error) {
	state, _ := prior.(codexSubagentState)
	if state.Children == nil {
		state.Children = map[string]*codexSubagentChildCache{}
	}
	if state.DayDirs == nil {
		state.DayDirs = map[string]time.Time{}
	}

	parentMeta, ok := readCodexRolloutSessionMeta(transcriptPath)
	if !ok || parentMeta.Payload.ID == "" {
		return nil, state, nil
	}
	rootID := parentMeta.Payload.ID

	sessionsRoot, ok := codexSessionsRootFromRolloutPath(transcriptPath)
	if !ok {
		return nil, state, nil
	}

	now := time.Now()
	fromTime := now
	if parentStartedAt := claudeSubagentParseTimestamp(parentMeta.Payload.Timestamp); parentStartedAt > 0 {
		fromTime = time.UnixMilli(parentStartedAt)
	}
	codexSubagentScanDayDirs(sessionsRoot, fromTime, now, state)

	if len(state.Children) == 0 {
		return nil, state, nil
	}
	connected := codexSubagentConnectedToRoot(rootID, state.Children)
	if len(connected) == 0 {
		return nil, state, nil
	}

	nodes := make(map[string]proto.SubagentNode, len(connected))
	for id := range connected {
		cache := state.Children[id]
		info, statErr := os.Stat(cache.Path)
		if statErr != nil {
			// rollout がロテーション等で消えている。この poll では出さない
			// （方針 5）。エントリ自体は state に残し、戻ってくれば拾い直す。
			continue
		}
		unchanged := cache.JSONLModTime.Equal(info.ModTime()) && cache.JSONLSize == info.Size()
		if unchanged {
			cache.Grew = false
		} else {
			name, summary, _ := codexSubagentReadLastTool(cache.Path, budget.TailBytes)
			cache.LastToolName = name
			cache.LastToolSummary = summary
			cache.JSONLModTime = info.ModTime()
			cache.JSONLSize = info.Size()
			cache.Grew = true // "first sight" もここに含む（transcript_stall.go と同じ規約）
		}

		node := proto.SubagentNode{
			ID:              id,
			Depth:           cache.Depth,
			Label:           codexSubagentLabel(cache),
			StartedAt:       cache.StartedAt,
			LastActivityAt:  info.ModTime().UnixMilli(),
			LastToolName:    cache.LastToolName,
			LastToolSummary: cache.LastToolSummary,
		}
		if cache.ParentThreadID != rootID {
			node.ParentID = cache.ParentThreadID
		}
		nodes[id] = node
	}
	if len(nodes) == 0 {
		return nil, state, nil
	}

	signals := codexSubagentScanParentSignals(transcriptPath, nodes)
	for id, node := range nodes {
		cache := state.Children[id]
		if sig, ok := signals[id]; ok {
			if sig.Started {
				cache.SeenStarted = true
			}
			if sig.Terminal && !cache.Terminal {
				cache.Terminal = true
				cache.Failed = sig.Failed
				cache.FinishedAt = sig.FinishedAt
			}
		}
		switch {
		case cache.Terminal && cache.Failed:
			node.State = "failed"
			node.FinishedAt = cache.FinishedAt
		case cache.Terminal:
			node.State = "done"
			node.FinishedAt = cache.FinishedAt
		case cache.SeenStarted:
			node.State = "running"
		default:
			// 起動も完了も一度も見ていない子（親 plan「修正の C の共通の決
			// まり」）。最初にこの子を見た poll でだけ判定し、以後は固定する。
			if !cache.FirstObservationDecided {
				cache.FirstObservationDecided = true
				cache.FirstObservationRunning = now.Sub(time.UnixMilli(node.LastActivityAt)) <= codexSubagentUnseenSignalWindow
			}
			if cache.FirstObservationRunning {
				node.State = "running"
			} else {
				node.State = "unknown"
			}
		}
		nodes[id] = node
	}

	claudeSubagentDropOrphans(nodes) // 生成時点では孤児は無いはずだが、Claude 側と同じ形にそろえる

	sinceMs := since.UnixMilli()
	for id, node := range nodes {
		if node.State != "running" && node.StartedAt < sinceMs {
			delete(nodes, id)
		}
	}
	claudeSubagentDropOrphans(nodes) // since フィルタで親が落ちた孫を連鎖で落とす

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
	for _, n := range nodes {
		ordered = append(ordered, n)
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
		Provider:  "subagent:codex-v1",
		Nodes:     ordered,
		Omitted:   omitted,
		UpdatedAt: time.Now().UnixMilli(),
	}, state, nil
}

// codexSubagentLabel picks agent_nickname, falling back to agent_role (子
// plan C3 作業内容「名前: agent_nickname、無ければ agent_role」).
func codexSubagentLabel(cache *codexSubagentChildCache) string {
	if cache.Nickname != "" {
		return cache.Nickname
	}
	return cache.Role
}

// codexSubagentConnectedToRoot returns the ids in children that are
// transitively reachable from rootID by following ParentThreadID links (see
// file doc comment for why a depth>=2 child's parent_thread_id names another
// child, not the root, so a single equality check against rootID is not
// enough). This also naturally excludes any unrelated subagent rollout
// discovered in the same sessions/ day directories that happens to belong to
// a different root session entirely.
func codexSubagentConnectedToRoot(rootID string, children map[string]*codexSubagentChildCache) map[string]bool {
	connected := map[string]bool{}
	changed := true
	for changed {
		changed = false
		for id, c := range children {
			if connected[id] {
				continue
			}
			if c.ParentThreadID == rootID || connected[c.ParentThreadID] {
				connected[id] = true
				changed = true
			}
		}
	}
	return connected
}

// codexSubagentScanDayDirs (re)scans sessions/YYYY/MM/DD directories from
// fromTime's date through now's date (inclusive, capped at
// codexSubagentMaxDayDirs), adding any newly discovered subagent-shaped
// rollout to state.Children. A day directory already recorded in
// state.DayDirs is skipped entirely unless it is today's directory and its
// ModTime has changed since the value recorded there (子 plan C3 作業内容).
func codexSubagentScanDayDirs(sessionsRoot string, fromTime, now time.Time, state codexSubagentState) {
	today := now.Local()
	for _, d := range codexSubagentDayDirDates(fromTime, now) {
		dayDir := filepath.Join(sessionsRoot, fmt.Sprintf("%04d", d.Year()), fmt.Sprintf("%02d", d.Month()), fmt.Sprintf("%02d", d.Day()))
		info, statErr := os.Stat(dayDir)
		if statErr != nil {
			continue // まだ/もう存在しない日ディレクトリ。何もしない。
		}
		isToday := d.Year() == today.Year() && d.Month() == today.Month() && d.Day() == today.Day()
		if lastSeen, known := state.DayDirs[dayDir]; known {
			if !isToday || lastSeen.Equal(info.ModTime()) {
				continue
			}
		}
		entries, readErr := os.ReadDir(dayDir)
		if readErr != nil {
			continue
		}
		for _, e := range entries {
			if e.IsDir() || !strings.HasSuffix(e.Name(), ".jsonl") {
				continue
			}
			path := filepath.Join(dayDir, e.Name())
			if codexSubagentAlreadyKnown(state.Children, path) {
				continue
			}
			meta, ok := readCodexRolloutSessionMeta(path)
			if !ok || meta.Payload.Source.Subagent.ThreadSpawn == nil {
				continue
			}
			ts := meta.Payload.Source.Subagent.ThreadSpawn
			id := meta.Payload.ID
			if id == "" || ts.ParentThreadID == "" {
				continue
			}
			depth := ts.Depth
			if depth <= 0 {
				depth = 1
			}
			state.Children[id] = &codexSubagentChildCache{
				Path:           path,
				Nickname:       ts.AgentNickname,
				Role:           ts.AgentRole,
				ParentThreadID: ts.ParentThreadID,
				Depth:          depth,
				StartedAt:      claudeSubagentParseTimestamp(meta.Payload.Timestamp),
			}
		}
		state.DayDirs[dayDir] = info.ModTime()
	}
}

// codexSubagentAlreadyKnown reports whether path has already been classified
// (as a child or otherwise) in a previous scan, so its first line is not
// re-read every time its containing day directory changes.
func codexSubagentAlreadyKnown(children map[string]*codexSubagentChildCache, path string) bool {
	for _, c := range children {
		if c.Path == path {
			return true
		}
	}
	return false
}

// codexSubagentDayDirDates returns the sequence of local calendar dates
// (truncated to midnight) from from's date through to's date inclusive,
// capped at codexSubagentMaxDayDirs entries.
func codexSubagentDayDirDates(from, to time.Time) []time.Time {
	from = from.Local()
	to = to.Local()
	if to.Before(from) {
		from, to = to, from
	}
	start := time.Date(from.Year(), from.Month(), from.Day(), 0, 0, 0, 0, from.Location())
	end := time.Date(to.Year(), to.Month(), to.Day(), 0, 0, 0, 0, to.Location())
	var dates []time.Time
	for d := start; !d.After(end) && len(dates) < codexSubagentMaxDayDirs; d = d.AddDate(0, 0, 1) {
		dates = append(dates, d)
	}
	return dates
}

// codexSessionsRootFromRolloutPath walks up from a rollout file's own path
// to find its ancestor "sessions" directory (CODEX_HOME/sessions), without
// requiring CodexHome as a separate input — the rollout path already encodes
// it, the same way it is laid out by findCodexRolloutLog
// (CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl).
func codexSessionsRootFromRolloutPath(path string) (string, bool) {
	dir := filepath.Dir(path)
	for i := 0; i < 8; i++ {
		if filepath.Base(dir) == "sessions" {
			return dir, true
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	return "", false
}

// readCodexRolloutSessionMeta reads only the first line of a rollout JSONL
// file (子 plan C3 完了条件「子の探索で読むのは各 rollout の先頭行だけ」),
// mirroring agent_log_handler.go's readCodexSessionMeta but decoding the
// additional id/source.subagent fields this reader needs.
func readCodexRolloutSessionMeta(path string) (codexRolloutSessionMeta, bool) {
	f, err := os.Open(path) // #nosec G304 -- path built from the local Codex sessions dir, not user input.
	if err != nil {
		return codexRolloutSessionMeta{}, false
	}
	defer func() { _ = f.Close() }()
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 64*1024), codexSubagentSessionMetaLineMax)
	if !sc.Scan() {
		return codexRolloutSessionMeta{}, false
	}
	var meta codexRolloutSessionMeta
	if err := json.Unmarshal(sc.Bytes(), &meta); err != nil || meta.Type != "session_meta" {
		return codexRolloutSessionMeta{}, false
	}
	return meta, true
}

// codexSubagentScanParentSignals reads a bounded tail page of the parent's
// own rollout and resolves whatever SubAgentActivity started/completion
// signals it can find this poll, keyed by the child's own thread id
// (item.agent_thread_id — see file doc comment). It never reads
// response_item/tool-call bodies: only
// event_msg/item_completed/SubAgentActivity's kind, agent_thread_id, and
// completed_at_ms. The caller is responsible for latching these into the
// child's persistent cache (修正 C8) — this function itself remembers
// nothing across calls.
func codexSubagentScanParentSignals(transcriptPath string, nodes map[string]proto.SubagentNode) map[string]codexSubagentParentSignal {
	signals := map[string]codexSubagentParentSignal{}
	if transcriptPath == "" {
		return signals
	}
	budget := agentChatReadBudget{
		MaxBytes:   codexSubagentParentReadBytesMax,
		MaxRecords: codexSubagentParentReadRecordsMax,
		Deadline:   time.Now().Add(agentChatReadTimeBudget),
	}
	records, _, err := readAgentChatTailPageWithBudget(transcriptPath, codexSubagentParentReadRecordsMax, -1, budget)
	if err != nil {
		return signals
	}
	for _, rec := range records { // newest-first
		var line codexEventMsgItemCompleted
		if json.Unmarshal(rec.line, &line) != nil {
			continue
		}
		if line.Type != "event_msg" || line.Payload.Type != "item_completed" || line.Payload.Item.Type != "SubAgentActivity" {
			continue
		}
		id := line.Payload.Item.AgentThreadID
		if id == "" {
			continue
		}
		if _, known := nodes[id]; !known {
			continue // この poll で見えていない子の信号は無視
		}
		acc := signals[id]
		switch line.Payload.Item.Kind {
		case "started":
			acc.Started = true
		case "completed":
			if !acc.Terminal {
				acc.Terminal = true
				acc.FinishedAt = line.Payload.CompletedAtMs
			}
		case "interrupted":
			if !acc.Terminal {
				acc.Terminal = true
				acc.Failed = true
				acc.FinishedAt = line.Payload.CompletedAtMs
			}
			// "interacted" はどちらの latch にも寄与しない（ファイル doc
			// comment 参照。起動は started、完了は completed/interrupted の
			// みで決める）。
		}
		signals[id] = acc
	}
	return signals
}

// codexSubagentReadLastTool reads at most tailBytes from the end of a
// child's own rollout and returns the most recent response_item tool call's
// name and an allow-listed, truncated summary (see file doc comment for why
// only custom_tool_call gets a summary). bytesRead is the actual number of
// bytes the underlying tail read touched (agentChatReadStats.BytesRead),
// returned so tests can count the read-budget bound directly (R10) instead
// of inferring it from whether a tool call outside the tail window was found
// — readCodexSubagentTree itself ignores this return value.
func codexSubagentReadLastTool(path string, tailBytes int64) (name, summary string, bytesRead int64) {
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
		return "", "", stats.BytesRead
	}
	for _, rec := range records { // newest-first
		var line codexResponseItemToolCall
		if json.Unmarshal(rec.line, &line) != nil || line.Type != "response_item" {
			continue
		}
		switch line.Payload.Type {
		case "custom_tool_call":
			return line.Payload.Name, codexSubagentSummaryFromInput(line.Payload.Input), stats.BytesRead
		case "function_call":
			return line.Payload.Name, "", stats.BytesRead
		}
	}
	return "", "", stats.BytesRead
}

// codexSubagentSummaryFromInput decodes a custom_tool_call's input exactly
// as far as confirming it is the plain JSON string real "exec" calls always
// carry (2,215/2,215 real samples), then clamps it with the same helper
// Claude's summary uses (claudeSubagentClampSummary — 100 runes, matching
// web/src/app/workflow-modal.ts's clampForLine). Any other shape (e.g. input
// decoding as a JSON object) is not decoded further and yields no summary,
// per 方針 5 — no allow-listed key extraction is defined for that shape
// because it has never been observed.
func codexSubagentSummaryFromInput(raw json.RawMessage) string {
	if len(raw) == 0 {
		return ""
	}
	var s string
	if json.Unmarshal(raw, &s) != nil {
		return ""
	}
	return claudeSubagentClampSummary(s)
}
