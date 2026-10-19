package hub

// subagent_source_grok.go implements subagentReader for "subagent:grok-v1"
// (親 plan docs/local/plan_subagent-tree-popup.md C4, corrected by C9 after the
// 敵対レビュー found R1/R2 below). It reads the files Grok Build itself writes
// for a subagent run — the parent session's own updates.jsonl, its
// subagents/<id>/meta.json, and (C9) the *sibling* session directory the
// child's own child_session_id names — never the parent's PTY output (親 plan
// 方針 1).
//
// C9 修正の 2 点（敵対レビュー R1・R2、2026-09-23）:
//
//   - R1 (状態判定): C4 の実装は「子の events.jsonl の mtime/size が前回の poll
//     から伸びたか」（Grew）で running を決めていた。これだと、長い間ツール
//     呼び出しが無い（伸びない）走行中の子が数 poll ごとに running/unknown を
//     行き来する。C9 は状態を「起動の信号（subagent_spawned）を見たら
//     running、完了の信号（subagent_finished／meta.json の status）を見たら
//     done/failed」に作り直した。一度 running と判定された子（Started
//     latch、下記 grokSubagentChildCache.Started）は、完了の信号が来るまで
//     running のまま保つ — 記録が伸びない・spawn イベントが親の読み取り窓
//     （2MB/500 行）から外れて再検出できなくても、です（Started はキャッシュ
//     に一度立てば cached.Started として毎 poll 引き継がれる）。
//   - R2 (読み元): C4 の実装は今の作業（LastToolName/LastToolSummary）を
//     `subagents/<id>/events.jsonl` から読んでいたが、**そのパスは実在しな
//     い**（実測: `subagents/<id>/` は meta.json 30 件・output.json 27 件のみ
//     で、events.jsonl は 0 件）。実物は `subagent_spawned` イベント自身が持つ
//     `child_session_id` が指す、親のセッションディレクトリの**兄弟**
//     `sessions/<cwd>/<child_session_id>/events.jsonl`（C9 実装時に 21/21 件
//     実在を確認）。child_session_id が無い・その兄弟ディレクトリが無い子
//     （実測 30 件中 9 件）では、今の作業を出さないだけで、状態は上の signal
//     ベースの規則で決まる（方針5）。
//
// C9 実装時に初めて実サンプルが取れた events.jsonl の形（2026-09-23、21 個の
// 完了済み子の兄弟ディレクトリ・計 74,471 行、キー名とイベント型名・件数のみ
// 確認。本文/引数は一切読んでいない）は、C4 の file doc comment が仮定してい
// た「updates.jsonl と同じ JSON-RPC 封筒（params.update.sessionUpdate:
// "tool_call" / rawInput）」とは**まったく別の、フラットな形**だった:
//
//	{"ts":"<RFC3339Nano>","type":"tool_started","tool_name":"read_file"}
//	{"ts":"<RFC3339Nano>","type":"tool_completed","tool_call_id":"...","duration_ms":123,"tool_name":"read_file","outcome":"success"}
//
// 実測した type の内訳（74,471 行中）: phase_changed 69,839／permission_
// resolved 999／permission_requested 999／tool_started 999／tool_completed
// 996／first_token 299／loop_started 299／turn_started 21／turn_ended 20。
// tool_name を持つのは tool_started/tool_completed だけで、値は
// run_terminal_command／read_file／list_dir／grep／web_fetch／write／
// search_replace／get_command_or_subagent_output の 8 種のみ（実測範囲）。
// この 2 レコード種のキーは ts/type/tool_name（tool_started）と ts/type/
// tool_call_id/duration_ms/tool_name/outcome（tool_completed）だけで、
// Claude/Codex の rawInput/input に相当する「対象」フィールドが**存在しない**
// ため、grokSubagentReadLastTool は LastToolName だけを返し、LastToolSummary
// は常に空文字にする（方針5 "読めないときは出さない" — 無いキーは出せない。
// 未確認の形に対する防御ではなく、確認した上で無いと分かった）。
//
// 以下、C4 で確認済みのまま変わっていない事実:
//
//   - 完了した子の目印は subagents/<subagent_id>/meta.json（30/30 件でディレ
//     クトリ名 == meta.json 自身の "subagent_id"、meta.json の
//     "parent_session_id" == 含む親セッションディレクトリ自身の名前）。
//     status は "completed" と "cancelled" の 2 値のみ実測（"completed" 以外
//     を失敗扱い）。started_at/completed_at は RFC3339Nano。
//   - meta.json が無い子の身元は、親自身の updates.jsonl の
//     {"timestamp":<epoch秒>,"method":"_x.ai/session/update","params":{"sessionId":...,"update":{"sessionUpdate":"subagent_spawned"|"subagent_finished",...}}}
//     行から取る。subagent_spawned は subagent_id/subagent_type/description/
//     model/child_session_id を持つ（C9 で child_session_id を新たに使うよ
//     うになった）。subagent_finished は subagent_id/status/tool_calls を持
//     つ。output/error（子自身の結果/失敗詳細）はどちらも decode 構造体に
//     一切宣言しない（方針3）。
//   - 入れ子は公式に 1 段までなので、すべてのノードが Depth=1/ParentID=""
//     （親 plan C4 作業内容）。

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"time"

	"many-ai-cli/internal/proto"
)

const (
	// grokSubagentParentReadBytesMax/RecordsMax bound the tail scan of the
	// *parent's* updates.jsonl used to resolve subagent_spawned/
	// subagent_finished signals for every still-in-flight child in one poll.
	// Mirrors claudeSubagentParentReadBytesMax/RecordsMax and
	// codexSubagentParentReadBytesMax/RecordsMax's identical role. Independent
	// of subagentReadBudget's HeadBytes/TailBytes, which bound each child's
	// own sibling events.jsonl tail read instead.
	grokSubagentParentReadBytesMax   = 2 * 1024 * 1024
	grokSubagentParentReadRecordsMax = 500
)

// grokSubagentMeta is the intentionally narrow decode shape for a completed
// child's subagents/<id>/meta.json. "prompt" and "error" — the child's own
// task text and failure detail — are deliberately never declared here (方針
// 3), so json.Unmarshal drops them on the floor before they ever reach a Go
// value this reader can touch.
type grokSubagentMeta struct {
	ParentSessionID  string `json:"parent_session_id"`
	SubagentType     string `json:"subagent_type"`
	Description      string `json:"description"`
	Status           string `json:"status"`
	StartedAt        string `json:"started_at"`
	CompletedAt      string `json:"completed_at"`
	ToolCalls        int    `json:"tool_calls"`
	EffectiveModelID string `json:"effective_model_id"`
}

// grokUpdateEnvelope is the narrow decode of one updates.jsonl line far
// enough to reach params.update (see file doc comment for the confirmed
// shape). Timestamp is epoch *seconds* (confirmed: Int64, 10 digits, 216/216
// samples) — every caller below multiplies by 1000 itself. This envelope is
// specific to updates.jsonl; the sibling events.jsonl this reader also reads
// (C9) has an entirely different, flat shape (grokSubagentEventRecord below).
type grokUpdateEnvelope struct {
	Timestamp int64  `json:"timestamp"`
	Method    string `json:"method"`
	Params    struct {
		SessionID string          `json:"sessionId"`
		Update    json.RawMessage `json:"update"`
	} `json:"params"`
}

// grokUpdateKind sniffs params.update.sessionUpdate without decoding the rest
// of the (per-kind-shaped) update object.
type grokUpdateKind struct {
	SessionUpdate string `json:"sessionUpdate"`
}

// grokSubagentSpawnUpdate is a "subagent_spawned" update's own fields (30/30
// real samples had all five, including child_session_id — the key C9 added
// use of, to resolve the sibling events.jsonl path below).
type grokSubagentSpawnUpdate struct {
	SubagentID     string `json:"subagent_id"`
	SubagentType   string `json:"subagent_type"`
	Description    string `json:"description"`
	Model          string `json:"model"`
	ChildSessionID string `json:"child_session_id"`
}

// grokSubagentFinishUpdate is a "subagent_finished" update's own fields.
// "output"/"error" (the child's own result/failure text) are deliberately
// never declared here (方針 3).
type grokSubagentFinishUpdate struct {
	SubagentID string `json:"subagent_id"`
	Status     string `json:"status"`
	ToolCalls  int    `json:"tool_calls"`
}

// grokSubagentEventRecord is the narrow decode of one line of a child's own
// sibling-session events.jsonl — a flat record shape confirmed 2026-09-23
// (C9, 21 real samples/74,471 lines) to be entirely unrelated to
// updates.jsonl's JSON-RPC envelope. Only "tool_started"/"tool_completed"
// carry "tool_name"; the other seven observed Type values (phase_changed,
// permission_requested, permission_resolved, first_token, loop_started,
// turn_started, turn_ended) are skipped by readers below without being
// decoded any further. There is no argument/target field in this schema at
// all (confirmed: tool_started's only other key is "ts"; tool_completed's are
// "ts"/"tool_call_id"/"duration_ms"/"outcome") — so LastToolSummary can never
// be built from it (方針 3 is moot here: there is nothing to allow-list).
type grokSubagentEventRecord struct {
	Type     string `json:"type"`
	ToolName string `json:"tool_name"`
}

// grokSubagentSpawnInfo is one child's identity as known from a
// "subagent_spawned" update (or carried forward in cache once seen once).
type grokSubagentSpawnInfo struct {
	Label          string
	AgentType      string
	Model          string
	ChildSessionID string
	StartedAtMs    int64
}

// grokSubagentFinishInfo is one child's terminal signal as known from a
// "subagent_finished" update, used only when meta.json has not appeared yet
// (a resolved-but-not-yet-flushed race — see file doc comment).
type grokSubagentFinishInfo struct {
	Failed       bool
	ToolCalls    int
	FinishedAtMs int64
}

// grokSubagentChildCache is what this reader remembers per child (keyed by
// subagent_id) across polls, via subagentReader's prior/next contract. Done
// latches true once meta.json (or a "subagent_finished" update) resolves a
// child's terminal state — from then on this reader never re-reads that
// child's meta.json or re-scans updates.jsonl on its behalf, mirroring
// claudeSubagentChildCache/codexSubagentChildCache's "meta fields never
// change once resolved" precedent.
//
// Started latches true the first time this reader has seen an explicit start
// signal (a live "subagent_spawned" update, or Done via meta.json) for this
// child, and — once true — is carried forward from cache every later poll
// regardless of whether the sibling events.jsonl has grown (C9 / R1 fix: this
// replaces the old Grew-driven running/unknown decision entirely).
type grokSubagentChildCache struct {
	Label           string
	AgentType       string
	Model           string
	ChildSessionID  string
	StartedAt       int64
	FinishedAt      int64
	Failed          bool
	Done            bool
	Started         bool
	ToolCalls       int
	ToolCallsKnown  bool
	LastToolName    string
	LastToolSummary string
	LastActivityAt  int64
	EventsModTime   time.Time
	EventsSize      int64
}

// grokSubagentState is the concrete prior/next state readGrokSubagentTree
// asserts out of subagentReader's `any` parameter/return.
type grokSubagentState struct {
	Children map[string]*grokSubagentChildCache
}

// readGrokSubagentTree is the subagent:grok-v1 entry in subagentReaderByKey
// (internal/hub/subagent_tree.go). transcriptPath is the parent's own
// updates.jsonl path, already resolved by the caller via
// resolveGrokSubagentParentPath (親 plan C4). subagents/ and every sibling
// child session directory are resolved relative to that same file's own
// directory (親 plan C9 作業内容).
func readGrokSubagentTree(transcriptPath string, since time.Time, prior any, budget subagentReadBudget) (*proto.SubagentTree, any, error) {
	state, _ := prior.(grokSubagentState)
	if state.Children == nil {
		state.Children = map[string]*grokSubagentChildCache{}
	}

	sessionDir := filepath.Dir(transcriptPath)
	parentSessionID := filepath.Base(sessionDir)
	// sessionsRoot is the cwd-named directory sessionDir itself lives under —
	// every child_session_id names a *sibling* of sessionDir there, never a
	// path under subagents/ (R2 — subagents/<id>/events.jsonl does not exist).
	sessionsRoot := filepath.Dir(sessionDir)
	subDir := filepath.Join(sessionDir, "subagents")
	entries, err := os.ReadDir(subDir)
	if err != nil {
		// 「まだ子がいない」の通常状態（方針5: 読めないときは出さない）。
		return nil, state, nil
	}

	// 全員すでに Done (meta.json 既読) なら、比較的重い updates.jsonl の走査を
	// この poll では省く（子 plan C4 作業内容「一度つながった子のパスは覚えて
	// おき…」と同じ「確定済みは読み直さない」規律）。
	needSignals := false
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		if cached := state.Children[e.Name()]; cached == nil || !cached.Done {
			needSignals = true
			break
		}
	}
	var spawned map[string]grokSubagentSpawnInfo
	var finished map[string]grokSubagentFinishInfo
	if needSignals {
		spawned, finished = grokSubagentScanUpdates(transcriptPath)
	}

	nodes := make(map[string]proto.SubagentNode, len(entries))
	nextChildren := make(map[string]*grokSubagentChildCache, len(state.Children))

	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		id := e.Name()
		if id == "" {
			continue
		}
		childDir := filepath.Join(subDir, id)

		if cached := state.Children[id]; cached != nil && cached.Done {
			nextChildren[id] = cached
			nodes[id] = grokSubagentNodeFromCache(id, cached)
			continue
		}

		if meta, metaModTime, ok := grokReadSubagentMeta(filepath.Join(childDir, "meta.json")); ok {
			if meta.ParentSessionID != "" && meta.ParentSessionID != parentSessionID {
				// 想定外の親子（子 plan 作業内容「meta に想定外の親子が出たら、
				// その子は木に入れない」）。
				continue
			}
			entry := &grokSubagentChildCache{
				Label: meta.Description, AgentType: meta.SubagentType, Model: meta.EffectiveModelID,
				StartedAt:      claudeSubagentParseTimestamp(meta.StartedAt),
				FinishedAt:     claudeSubagentParseTimestamp(meta.CompletedAt),
				Failed:         meta.Status != "completed",
				Done:           true,
				Started:        true,
				ToolCalls:      meta.ToolCalls,
				ToolCallsKnown: true,
				LastActivityAt: metaModTime.UnixMilli(),
			}
			nextChildren[id] = entry
			nodes[id] = grokSubagentNodeFromCache(id, entry)
			continue
		}

		// meta.json 未着手 = 走行中、または直近に終わった（subagent_finished
		// イベントのみ既知）。
		cached := state.Children[id]
		sp, haveSpawn := spawned[id]
		entry := &grokSubagentChildCache{}
		switch {
		case haveSpawn:
			// 起動の信号を今 poll で見た（C9 / R1: これが Started を立てる唯一
			// の直接証拠。以後は完了の信号が来るまでこの latch を保つ）。
			entry.Label, entry.AgentType, entry.Model = sp.Label, sp.AgentType, sp.Model
			entry.ChildSessionID = sp.ChildSessionID
			entry.StartedAt = sp.StartedAtMs
			entry.Started = true
		case cached != nil:
			// 前回までに起動の信号を見て Started 済み。今 poll の走査窓に
			// spawn イベントが無く（親の記録が伸びて 2MB/500 行の外へ出た
			// 等）再検出できなくても、running のまま保つ（C9 / R1）。
			entry.Label, entry.AgentType, entry.Model = cached.Label, cached.AgentType, cached.Model
			entry.ChildSessionID = cached.ChildSessionID
			entry.StartedAt = cached.StartedAt
			entry.Started = cached.Started
		default:
			// この poll の走査窓に spawn イベントが無く、前回までの cache も
			// 無い＝ラベルを安全に組めない（方針5）。
			continue
		}

		entry.LastActivityAt = entry.StartedAt
		if cached != nil {
			entry.LastActivityAt = cached.LastActivityAt
			entry.LastToolName = cached.LastToolName
			entry.LastToolSummary = cached.LastToolSummary
			entry.EventsModTime = cached.EventsModTime
			entry.EventsSize = cached.EventsSize
		}
		if entry.ChildSessionID != "" {
			// R2 の直し方: subagents/<id>/events.jsonl (実在しない) ではなく、
			// 親のセッションディレクトリの兄弟を見る。
			eventsPath := filepath.Join(sessionsRoot, entry.ChildSessionID, "events.jsonl")
			if info, statErr := os.Stat(eventsPath); statErr == nil {
				unchanged := entry.EventsModTime.Equal(info.ModTime()) && entry.EventsSize == info.Size()
				if !unchanged {
					name, summary := grokSubagentReadLastTool(eventsPath, budget.TailBytes)
					entry.LastToolName = name
					entry.LastToolSummary = summary
					entry.EventsModTime = info.ModTime()
					entry.EventsSize = info.Size()
				}
				entry.LastActivityAt = info.ModTime().UnixMilli()
			}
			// 兄弟ディレクトリ/ファイルが無い子（実測 30 件中 9 件）は、今の
			// ツールを出さないだけで、状態は上で決めた Started/Done で決まる
			// （親 plan C9 作業内容・方針5）。
		}

		if fin, ok := finished[id]; ok {
			entry.Done = true
			entry.Failed = fin.Failed
			entry.FinishedAt = fin.FinishedAtMs
			entry.ToolCalls = fin.ToolCalls
			entry.ToolCallsKnown = true
		}
		nextChildren[id] = entry
		nodes[id] = grokSubagentNodeFromCache(id, entry)
	}
	state.Children = nextChildren

	if len(nodes) == 0 {
		return nil, state, nil
	}

	sinceMs := since.UnixMilli()
	for id, node := range nodes {
		if node.State != "running" && node.StartedAt < sinceMs {
			delete(nodes, id)
		}
	}

	omitted := 0
	for len(nodes) > budget.MaxNodes {
		victim := claudeSubagentPickDropVictim(nodes)
		if victim == "" {
			break
		}
		delete(nodes, victim)
		omitted++
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
		Provider:  "subagent:grok-v1",
		Nodes:     ordered,
		Omitted:   omitted,
		UpdatedAt: time.Now().UnixMilli(),
	}, state, nil
}

// grokSubagentNodeFromCache builds the wire node for one child from its
// cache entry. Every Grok subagent is a direct child of the polled session
// (official 1-level nesting — file doc comment), so Depth/ParentID are always
// 1/"". State comes from Done/Failed/Started only (C9 / R1) — never from
// whether the sibling events.jsonl happened to grow this poll.
func grokSubagentNodeFromCache(id string, entry *grokSubagentChildCache) proto.SubagentNode {
	state := "unknown"
	switch {
	case entry.Done && entry.Failed:
		state = "failed"
	case entry.Done:
		state = "done"
	case entry.Started:
		state = "running"
	}
	node := proto.SubagentNode{
		ID: id, Depth: 1, Label: entry.Label, AgentType: entry.AgentType, Model: entry.Model,
		State: state, StartedAt: entry.StartedAt, FinishedAt: entry.FinishedAt,
		LastActivityAt:  entry.LastActivityAt,
		LastToolName:    entry.LastToolName,
		LastToolSummary: entry.LastToolSummary,
	}
	if entry.ToolCallsKnown {
		node.ToolCalls = entry.ToolCalls
	}
	return node
}

// grokReadSubagentMeta reads one child's subagents/<id>/meta.json in full
// (small, structural file — same unconditional-read precedent Claude's own
// meta.json reader follows) and returns its own mtime alongside the narrow
// decode, so callers can use "which file did we last look at" as
// LastActivityAt the same way Claude/Codex use their child transcript's own
// mtime.
func grokReadSubagentMeta(path string) (grokSubagentMeta, time.Time, bool) {
	info, statErr := os.Stat(path)
	if statErr != nil {
		return grokSubagentMeta{}, time.Time{}, false
	}
	data, err := os.ReadFile(path) // #nosec G304 -- path built from the local Grok subagents dir, not user input.
	if err != nil {
		return grokSubagentMeta{}, time.Time{}, false
	}
	var meta grokSubagentMeta
	if json.Unmarshal(data, &meta) != nil {
		return grokSubagentMeta{}, time.Time{}, false
	}
	return meta, info.ModTime(), true
}

// grokSubagentScanUpdates reads a bounded tail page of the parent's own
// updates.jsonl and resolves whatever subagent_spawned/subagent_finished
// signals it can find, keyed by subagent_id (see file doc comment). It never
// reads a "tool_call" update here (that update kind lives in updates.jsonl,
// not a child's own sibling events.jsonl, and is out of scope for this
// reader), and never decodes "output"/"error" at all.
func grokSubagentScanUpdates(transcriptPath string) (map[string]grokSubagentSpawnInfo, map[string]grokSubagentFinishInfo) {
	spawned := map[string]grokSubagentSpawnInfo{}
	finished := map[string]grokSubagentFinishInfo{}
	if transcriptPath == "" {
		return spawned, finished
	}
	budget := agentChatReadBudget{
		MaxBytes:   grokSubagentParentReadBytesMax,
		MaxRecords: grokSubagentParentReadRecordsMax,
		Deadline:   time.Now().Add(agentChatReadTimeBudget),
	}
	records, _, err := readAgentChatTailPageWithBudget(transcriptPath, grokSubagentParentReadRecordsMax, -1, budget)
	if err != nil {
		return spawned, finished
	}
	for _, rec := range records { // newest-first; first (most recent) signal per child wins
		var env grokUpdateEnvelope
		if json.Unmarshal(rec.line, &env) != nil || len(env.Params.Update) == 0 {
			continue
		}
		var kind grokUpdateKind
		if json.Unmarshal(env.Params.Update, &kind) != nil {
			continue
		}
		switch kind.SessionUpdate {
		case "subagent_spawned":
			var sp grokSubagentSpawnUpdate
			if json.Unmarshal(env.Params.Update, &sp) != nil || sp.SubagentID == "" {
				continue
			}
			if _, exists := spawned[sp.SubagentID]; exists {
				continue
			}
			spawned[sp.SubagentID] = grokSubagentSpawnInfo{
				Label: sp.Description, AgentType: sp.SubagentType, Model: sp.Model,
				ChildSessionID: sp.ChildSessionID,
				StartedAtMs:    env.Timestamp * 1000,
			}
		case "subagent_finished":
			var fi grokSubagentFinishUpdate
			if json.Unmarshal(env.Params.Update, &fi) != nil || fi.SubagentID == "" {
				continue
			}
			if _, exists := finished[fi.SubagentID]; exists {
				continue
			}
			finished[fi.SubagentID] = grokSubagentFinishInfo{
				Failed: fi.Status != "completed", ToolCalls: fi.ToolCalls,
				FinishedAtMs: env.Timestamp * 1000,
			}
		}
	}
	return spawned, finished
}

// grokSubagentReadLastTool reads at most tailBytes from the end of a child's
// own sibling-session events.jsonl (sessions/<cwd>/<child_session_id>/
// events.jsonl — resolved by the caller, never subagents/<id>/events.jsonl;
// see file doc comment / C9 R2) and returns the most recent tool_started/
// tool_completed record's own "tool_name". This confirmed real schema
// (2026-09-23, 21 samples/74,471 lines) carries no argument/target field at
// all, so summary is always "" — there is nothing to allow-list (方針 3/5).
func grokSubagentReadLastTool(path string, tailBytes int64) (name, summary string) {
	if tailBytes <= 0 {
		tailBytes = defaultSubagentReadBudget().TailBytes
	}
	budget := agentChatReadBudget{
		MaxBytes:   tailBytes,
		MaxRecords: agentChatPageRecordsMax,
		Deadline:   time.Now().Add(agentChatReadTimeBudget),
	}
	records, _, err := readAgentChatTailPageWithBudget(path, agentChatPageRecordsMax, -1, budget)
	if err != nil {
		return "", ""
	}
	for _, rec := range records { // newest-first
		var ev grokSubagentEventRecord
		if json.Unmarshal(rec.line, &ev) != nil {
			continue
		}
		if ev.ToolName == "" {
			continue
		}
		if ev.Type != "tool_started" && ev.Type != "tool_completed" {
			continue
		}
		return ev.ToolName, ""
	}
	return "", ""
}
