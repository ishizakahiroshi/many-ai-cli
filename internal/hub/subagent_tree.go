package hub

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

// subagentReadBudget bounds how much of a provider's own subagent records
// (transcripts, meta files, event logs) one reader call may look at. Every
// subagent:* reader in subagentReaderByKey must honor it. The Claude numbers
// (子 plan plan_subagent-tree-popup_c2_hub-core-claude.md 内部 C2 の完了条件)
// are head 64KB / tail 128KB / 50 nodes; Codex and Grok (親 plan C3/C4) reuse
// this same struct, sized to their own completion conditions.
type subagentReadBudget struct {
	HeadBytes int64
	TailBytes int64
	MaxNodes  int
}

// defaultSubagentReadBudget is the shared budget every subagent:* reader
// starts from (head 64KB / tail 128KB / 50 nodes — 子 plan C2 の完了条件).
func defaultSubagentReadBudget() subagentReadBudget {
	return subagentReadBudget{HeadBytes: 64 * 1024, TailBytes: 128 * 1024, MaxNodes: 50}
}

// subagentReader is the "鍵 → 読み取り関数" contract every registered
// subagent:* key satisfies (docs/local/plan_subagent-tree-popup.md 方針 2).
//
//   - transcriptPath is the resolved location of the *parent's own* record.
//     Each key's own reader decides what that path means and how to derive
//     it (for subagent:claude-v1 it is the parent's Claude transcript path,
//     resolved with agent_log_handler.go's claudeTranscriptPath /
//     findClaudeTranscript — 子 plan C1 の実装内容). Path resolution is not
//     part of this signature because it differs per provider (親 plan 方針 2).
//   - since is the turn-start cutoff a session last settled at (親 plan 方針
//     4, computed by 親 C3 in input_gate.go).
//   - prior/next carry an opaque, reader-owned state across polls for one
//     session (e.g. per-child read offsets, a cached child directory
//     listing) — the caller (親 C3's polling loop) stores whatever `next`
//     comes back and passes it in as `prior` on the following poll, untyped.
//     A reader receives prior == nil on a session's first poll and type-
//     asserts its own concrete state struct out of it (falling back to that
//     struct's zero value when the assertion fails, e.g. after a Hub
//     restart) rather than accepting `any` state further down its own call
//     stack.
//   - A nil returned tree means "no change this poll, or no children"; the
//     caller keeps whatever it last broadcast for this session.
type subagentReader func(transcriptPath string, since time.Time, prior any, budget subagentReadBudget) (tree *proto.SubagentTree, next any, err error)

// subagentReaderByKey is the Hub-side "鍵 → 読み取り関数" table (親 plan 方針
// 2, provider.EffectiveDefinition.Adapters.Subagents 経由で選ばれる — Hub は
// providerDefinition(id) で引いた定義のこのキーだけを見て、provider 名では
// 分岐しない). Every subagent: key in provider.DefaultAdapterCatalog() must
// have an entry here; the table ↔ catalog consistency test
// (internal/hub/subagent_tree_test.go) and the polling/dispatch code that
// calls this table are 子 plan
// plan_subagent-tree-popup_c2_hub-core-claude.md 内部 C3 の完了条件 — the C1
// that seeded this map only decided the table's shape and the Claude entry's
// stub.
var subagentReaderByKey = map[string]subagentReader{
	"subagent:claude-v1": readClaudeSubagentTree,
	"subagent:codex-v1":  readCodexSubagentTree,
	"subagent:grok-v1":   readGrokSubagentTree,
}

// --- 子 plan plan_subagent-tree-popup_c2_hub-core-claude.md 内部 C3:
// ターン開始時刻・ポーリング・配信・再接続時の送り直し -----------------------

// subagentPollInterval is the self-rescheduling tick interval for
// runSubagentTreePoll (子 plan C3 作業内容「ポーリング…間隔は3秒」), the same
// timer-chain shape workflow_task_detail.go uses for its own poll
// (workflowTaskDetailPollInterval).
const subagentPollInterval = 3 * time.Second

// subagentPathResolveRetryInterval bounds how often runSubagentTreePoll may
// call a subagentParentPathResolver for one session while it has no cached,
// still-existing parent path (親 plan C10・R6). Whether the session has never
// resolved a path yet, or its cached path just disappeared, an actual
// resolver call (which, unlike the per-tick os.Stat on a cached path, may
// walk directories) happens at most once per this interval.
const subagentPathResolveRetryInterval = 30 * time.Second

// subagentParentInfo carries only the session fields a subagentParentPathResolver
// needs. It is built under sessionsMu and then read outside the lock (親 plan
// 前提「ロックを持ったまま ReadDir/Read しない」— transcript_stall.go と同じ方針).
type subagentParentInfo struct {
	CWD            string
	ClaudeDir      string
	HomeDir        string
	AgentSessionID string
	StartedAt      string
	// CodexHome/NativeLogPath feed resolveCodexSubagentParentPath (親 plan
	// C3), mirroring agentChatTranscriptPathForSnapshot's codex branch: the
	// Stop-hook-set NativeLogPath wins when present, otherwise the resolver
	// falls back to CodexHome + CWD + StartedAt (findCodexRolloutLog).
	CodexHome     string
	NativeLogPath string
	// GrokHome feeds resolveGrokSubagentParentPath (親 plan C4), mirroring
	// agent_log_handler.go's grok branch: grokHomeDir(GrokHome, HomeDir) +
	// CWD + StartedAt via findGrokChatHistory (the same session-directory
	// resolution grok_history_handler.go already uses for /api/grok-history).
	GrokHome string
}

// subagentParentPathResolver derives the transcriptPath argument a
// subagentReader needs for one session. Path resolution differs per key
// exactly as subagentReader's doc comment says ("Path resolution is not part
// of this signature because it differs per provider"), so each key registers
// its own resolver here — the same key → function shape subagentReaderByKey
// uses, so a future key (parent plan C3/C4: codex/grok) adds one map entry to
// each table without touching the dispatch loop below.
type subagentParentPathResolver func(info subagentParentInfo) (string, bool)

var subagentPathResolverByKey = map[string]subagentParentPathResolver{
	"subagent:claude-v1": resolveClaudeSubagentParentPath,
	"subagent:codex-v1":  resolveCodexSubagentParentPath,
	"subagent:grok-v1":   resolveGrokSubagentParentPath,
}

// resolveClaudeSubagentParentPath finds the parent's own Claude transcript
// path for the subagent:claude-v1 reader, reusing the exact resolution Hub
// already uses for agentLogForSession's claude branch
// (agent_log_handler.go's claudeTranscriptPath / findClaudeTranscript — 子
// plan C1 の指示). It is keyed off the provider *definition's*
// adapters.subagents value (checked by the caller, runSubagentTreePoll), not
// off ses.Provider == "claude": a definition whose id is not literally
// "claude" but whose adapters.subagents selects subagent:claude-v1 still
// resolves through this same path shape (子 plan C3 完了条件「provider 名が
// claude ではない定義でも…」). agentLogForSession itself is not reused
// directly because it branches on ses.Provider, which is exactly the
// provider-name branching 親 plan 方針2 says not to repeat here.
func resolveClaudeSubagentParentPath(info subagentParentInfo) (string, bool) {
	root := strings.TrimSpace(info.ClaudeDir)
	if root == "" {
		root = filepath.Join(info.HomeDir, ".claude")
	}
	if info.CWD == "" || root == "" {
		return "", false
	}
	if info.AgentSessionID != "" {
		return claudeTranscriptPath(root, info.CWD, info.AgentSessionID)
	}
	startedAt, err := time.Parse(time.RFC3339, info.StartedAt)
	if err != nil {
		return "", false
	}
	return findClaudeTranscript(root, info.CWD, startedAt)
}

// resolveCodexSubagentParentPath finds the parent's own Codex rollout path
// for the subagent:codex-v1 reader, reusing the exact codex-branch shape
// agentChatTranscriptPathForSnapshot already uses (agent_chat_handler.go:
// Stop-hook-set NativeLogPath first, falling back to findCodexRolloutLog —
// 親 plan C3 「親の rollout のパスは…で得る」). Like
// resolveClaudeSubagentParentPath, it is keyed off the provider
// *definition's* adapters.subagents value, not off ses.Provider == "codex".
// NativeLogPath is not re-validated with codexTranscriptPathAllowed here: it
// already was, once, at the single place that ever sets it
// (setCodexNativeLogPath in agent_log_handler.go) — the same trust boundary
// agentChatTranscriptPathForSnapshot's own codex branch relies on without a
// second check.
func resolveCodexSubagentParentPath(info subagentParentInfo) (string, bool) {
	root := strings.TrimSpace(info.CodexHome)
	if root == "" {
		if strings.TrimSpace(info.HomeDir) == "" {
			return "", false
		}
		root = filepath.Join(info.HomeDir, ".codex")
	}
	if info.CWD == "" {
		return "", false
	}
	if info.NativeLogPath != "" && isExistingFile(info.NativeLogPath) {
		return info.NativeLogPath, true
	}
	startedAt, err := time.Parse(time.RFC3339, info.StartedAt)
	if err != nil {
		return "", false
	}
	return findCodexRolloutLog(root, info.CWD, startedAt)
}

// resolveGrokSubagentParentPath finds the parent's own Grok session record
// for the subagent:grok-v1 reader, reusing the exact resolution Hub already
// uses for agentLogForSession's grok branch (agent_log_handler.go:
// grokHomeDir + findGrokChatHistory — the same cwd-directory-name /
// active_sessions.json+UUIDv7 matching / GrokHome-as-subscription-root rules
// grok_history_handler.go implements for /api/grok-history — 親 plan C4
// 「まず...特定方法を実ファイルで確かめる」の3点はすべてこの既存関数がすでに
// 解決済みだった、2026-09-23 実測). Like the claude/codex resolvers, it is
// keyed off the provider *definition's* adapters.subagents value, not off
// ses.Provider == "grok". findGrokChatHistory returns the child's own
// chat_history.jsonl path; readGrokSubagentTree only needs its containing
// session directory (chat_history.jsonl, updates.jsonl and the subagents/
// directory are siblings there — confirmed 2026-09-23 against real
// ~/.many-ai-cli/subscriptions/grok/*/sessions data), so this resolver
// returns the sibling updates.jsonl path instead: that actually is the
// parent's own record subagent lifecycle events are read from (unlike
// chat_history.jsonl, which this reader never opens).
func resolveGrokSubagentParentPath(info subagentParentInfo) (string, bool) {
	root := grokHomeDir(info.GrokHome, info.HomeDir)
	if root == "" || info.CWD == "" {
		return "", false
	}
	startedAt, err := time.Parse(time.RFC3339, info.StartedAt)
	if err != nil {
		return "", false
	}
	historyPath, ok := findGrokChatHistory(root, info.CWD, startedAt)
	if !ok {
		return "", false
	}
	return filepath.Join(filepath.Dir(historyPath), "updates.jsonl"), true
}

// subagentTurnStartAdvanceLocked implements 親 plan 方針4's turn-start
// gating, called from input_gate.go's handleInput at the point a confirmed
// user turn reaches the provider (the same "ライブ限定の正確な境界" comment
// documents). The "current batch" cutoff (subagentTurnStartedAt) only
// advances when the session's most recently built tree has zero running
// children (subagentRunningCount, updated by
// subagentTreeForBroadcastLocked's running count each poll — "unknown"
// children are never counted as running there, so a session with only
// unknown children also advances here, matching 子 plan C3 完了条件
// 「unknownの子だけが残っている状態でのユーザー入力では、区切りの時刻が進む」).
// While at least one child is running, the cutoff stays put so a child that
// finishes between two user turns does not drop out of the tree (親 plan 方
// 針4「子が走っている途中で親に『まだ進んでる？』と聞いても区切らない」).
// **呼び出し側は sessionsMu を保持していること**（ここでファイルは読まない・
// 保持している値を読むだけ）。
func subagentTurnStartAdvanceLocked(ses *session, now time.Time) {
	if ses == nil || ses.subagentRunningCount != 0 {
		return
	}
	ses.subagentTurnStartedAt = now
}

// subagentFallbackTurnStart picks the cutoff restartSubagentTreePollLocked
// falls back to when a session has never had subagentTurnStartedAt set (R7).
// ses.StartedAt is the same ISO-8601 field session_update's own StartedAt
// wire value comes from (server.go:172). Falling back to now when it cannot
// be parsed is deliberately conservative: an empty/unparsable StartedAt must
// not fall back to a zero cutoff, which would make every child in the
// parent's entire history match "since" (親 plan 方針4 が効かなくなる).
func subagentFallbackTurnStart(ses *session, now time.Time) time.Time {
	if ses == nil {
		return now
	}
	if t, err := time.Parse(time.RFC3339, ses.StartedAt); err == nil {
		return t
	}
	return now
}

// scheduleSubagentTreePollLocked (re)schedules the next poll tick for one
// session, stopping any timer already in flight and bumping the generation
// guard first (same shape as startWorkflowTaskDetailLocked /
// runWorkflowTaskDetailResolve's reschedule). Callers: input_gate.go's
// handleInput (first confirmed user turn) and
// restartSubagentTreePollLocked below (reattach resume). **呼び出し側は
// sessionsMu を保持していること。**
func (s *Server) scheduleSubagentTreePollLocked(id int, ses *session, delay time.Duration) {
	if ses == nil {
		return
	}
	if ses.subagentTimer != nil {
		ses.subagentTimer.Stop()
		ses.subagentTimer = nil
	}
	ses.subagentGeneration++
	generation := ses.subagentGeneration
	expected := ses
	ses.subagentTimer = time.AfterFunc(delay, func() { s.runSubagentTreePoll(id, expected, generation) })
}

// restartSubagentTreePollLocked resumes the subagent-tree poll chain for a
// session recreated by reattach (子 plan C3 「再接続時の送り直し」). Unlike
// restartReattachAsyncStateLocked's three Claude-only chains, this one is not
// gated on ses.Provider == "claude": eligibility (config / registered
// adapter key / a resolvable parent path) is re-evaluated reactively on the
// very first tick, exactly as it is for a brand-new session's first poll —
// see runSubagentTreePoll's doc comment. **呼び出し側は sessionsMu を
// 保持していること。**
//
// 親 plan C10（R7）: reattach で作られた新しいセッションが、区切りの時刻
// subagentTurnStartedAt を一度も持ったことが無い（=ゼロ値のまま。最初の確定
// ユーザー入力より前に reattach された場合に起きる — applyReattachPreservedStateLocked
// は旧セッションの値をそのままコピーするだけで、旧セッション自身がまだ
// subagentTurnStartAdvanceLocked を一度も通っていなければゼロのまま渡ってくる）
// と、poll が「区切りより前の全履歴」を対象にしてしまう（親 plan 方針4が効かない）。
// ゼロのままなら、このセッションの起動時刻（StartedAt）を区切りの代わりに使う。
func (s *Server) restartSubagentTreePollLocked(id int, ses *session, now time.Time) {
	if ses == nil {
		return
	}
	if ses.subagentTurnStartedAt.IsZero() {
		ses.subagentTurnStartedAt = subagentFallbackTurnStart(ses, now)
	}
	s.scheduleSubagentTreePollLocked(id, ses, 0)
}

// subagentTreeEnabled reads the config toggle the same way
// workflowTaskDetailEnabled does (子 plan C1 が追加した
// Workflow.SubagentTreeEnabled, 既定 true).
func (s *Server) subagentTreeEnabled() bool {
	if s == nil || s.cfg == nil {
		return false
	}
	return s.snapshotCfg().Workflow.SubagentTreeEnabled
}

// runSubagentTreePoll is the self-perpetuating poll tick for one session's
// subagent tree — the same self-rescheduling chain shape as
// runWorkflowTaskDetailPoll (workflow_task_detail.go), just keyed by the
// provider definition's adapters.subagents value instead of a fixed
// provider name. Eligibility (子 plan C3 完了条件の3つの起動条件: config /
// registered adapter key / resolvable parent path) is re-checked every tick
// rather than once at start — when any of the three fails, this tick simply
// does not reschedule itself, which is indistinguishable on the wire from
// "the timer never started" (ses.subagentTimer stays nil either way).
func (s *Server) runSubagentTreePoll(id int, expected *session, generation uint64) {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses != expected || ses.subagentGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	ses.subagentTimer = nil
	providerID := ses.Provider
	since := ses.subagentTurnStartedAt
	prior := ses.subagentReaderState
	cachedPath := ses.subagentParentPath
	cachedFor := ses.subagentParentPathFor
	lastPathAttempt := ses.subagentParentPathAttemptedAt
	info := subagentParentInfo{
		CWD: ses.CWD, ClaudeDir: ses.ClaudeDir, HomeDir: ses.HomeDir,
		AgentSessionID: ses.AgentSessionID, StartedAt: ses.StartedAt,
		CodexHome: ses.CodexHome, NativeLogPath: ses.NativeLogPath,
		GrokHome: ses.GrokHome,
	}
	s.sessionsMu.Unlock()

	// 起動条件 1/3: 設定 off。config.yaml は書き手が手書きしうるので、この
	// チェックは毎ティック再評価する（起動時に一度だけ判定して固定にはしない）。
	if !s.subagentTreeEnabled() {
		return
	}
	def, ok := s.providerDefinition(providerID)
	var key string
	if ok {
		key = def.Adapters.Subagents
	}
	reader, hasReader := subagentReaderByKey[key]
	// 起動条件 2/3: 定義が adapters.subagents を持たない、または対応表に無い鍵。
	if key == "" || !hasReader {
		return
	}

	// ロックを持ったままファイルを読まない（前提節）。パス解決も実読み取りも
	// ここ、ロックの外で行う。
	now := time.Now()
	var (
		tree             *proto.SubagentTree
		nextState        any = prior
		attempted        bool
		resolvedPath     string
		pathResolved     bool
		cachedPathStale  bool
		resolveAttempted bool
	)
	if resolver, hasResolver := subagentPathResolverByKey[key]; hasResolver {
		// 親 plan C10（R6）: 覚えたパスがまだ有効なら、resolver（ディレクトリ
		// 探索を伴いうる）を呼ばずに使い回す。isExistingFile は os.Stat 1 回
		// だけの安価な確認で、これは毎ティック行ってよい。resolver を実際に
		// 呼び直すのは (a) 覚えたパスが消えていたとき（stat で判明）と
		// (b) まだ一度も解決できていないとき（この (b) は 30 秒に 1 回まで。
		// (a) で再解決を試みた直後の lastPathAttempt もこの間隔に含まれるため、
		// 消えたパスへ resolver を毎ティック呼び直すことにもならない）。
		// 解決に使った入力（AgentSessionID・NativeLogPath など）が変わったら、覚えた
		// パスのファイルがまだ残っていても使わず、30 秒を待たずに解決し直す。
		inputsChanged := cachedPath != "" && cachedFor != info
		cachedValid := cachedPath != "" && !inputsChanged && isExistingFile(cachedPath)
		switch {
		case cachedValid:
			resolvedPath, pathResolved = cachedPath, true
		case inputsChanged || now.Sub(lastPathAttempt) >= subagentPathResolveRetryInterval:
			resolveAttempted = true
			resolvedPath, pathResolved = resolver(info)
		}
		if cachedPath != "" && !cachedValid {
			cachedPathStale = true
		}
		if pathResolved {
			// 起動条件 3/3 が満たせたときだけ、実際に reader を呼ぶ。
			var readErr error
			tree, nextState, readErr = reader(resolvedPath, since, prior, defaultSubagentReadBudget())
			if readErr != nil {
				// 読めないときは出さない（親 plan 方針5）。前回の状態を保持して
				// 次のティックで再試行する。「reader が実行され、その結果として
				// 空を返した」わけではないので、下の attempted は立てない。
				nextState = prior
				tree = nil
			} else {
				attempted = true
			}
		}
		// 起動条件 3/3 が満たせない（transcript がまだ無い等）場合は恒久停止に
		// せず、次のティックで再度パス解決を試みる（attempted は立てない）。
	}

	s.sessionsMu.Lock()
	ses = s.sessions[id]
	if ses == nil || ses != expected || ses.subagentGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	ses.subagentReaderState = nextState
	switch {
	case pathResolved:
		ses.subagentParentPath = resolvedPath
		ses.subagentParentPathFor = info
	case cachedPathStale:
		// 覚えたパスが消えていた: 捨てる。次のティックが「まだ解決できていない」
		// 側の分岐（30 秒レート制限つき）に入れるようにする。
		ses.subagentParentPath = ""
	}
	if resolveAttempted {
		ses.subagentParentPathAttemptedAt = now
	}
	out, shouldBroadcast := s.subagentTreeForBroadcastLocked(ses, key, tree, attempted, now)
	ses.subagentGeneration++
	nextGeneration := ses.subagentGeneration
	nextExpected := ses
	ses.subagentTimer = time.AfterFunc(subagentPollInterval, func() { s.runSubagentTreePoll(id, nextExpected, nextGeneration) })
	s.sessionsMu.Unlock()
	if shouldBroadcast {
		s.broadcast(proto.Message{Type: "subagent_tree", SessionID: id, SubagentTree: out})
	}
}

// subagentTreeForBroadcastLocked applies one poll's result to ses and decides
// whether it must go out over the wire. **呼び出し側は sessionsMu を
// 保持していること。**
//
//   - tree != nil: a freshly read, non-empty tree. Always replaces
//     ses.subagentTree/subagentRunningCount; broadcasts only if its signature
//     differs from the last one sent (子 plan C3 完了条件「木が変わらない間は
//     subagent_tree を送らない。変わったときは1回だけ送る」).
//   - tree == nil && attempted: the reader ran to completion and reported
//     zero matching children this poll (e.g. since advanced past every child
//     in view — 親 plan 方針4). If ses previously held a tree, this is a
//     genuine "cleared" transition: broadcast one empty tree so the UI drops
//     the chip, then forget it (子 plan C3 完了条件「子が0件になったときは、
//     空の木を1回だけ送って Web 側の表示を消す」).
//   - tree == nil && !attempted: the reader was never actually invoked this
//     poll (unresolved path) or it errored. This is NOT evidence that the
//     children went away — leave ses's last-known tree exactly as it was and
//     broadcast nothing, so a transient read failure never wrongly clears a
//     display of still-running children (親 plan 方針5「読めないときは出さ
//     ない」).
func (s *Server) subagentTreeForBroadcastLocked(ses *session, key string, tree *proto.SubagentTree, attempted bool, now time.Time) (*proto.SubagentTree, bool) {
	switch {
	case tree != nil:
		running := 0
		for _, n := range tree.Nodes {
			if n.State == "running" {
				running++
			}
		}
		ses.subagentTree = tree
		ses.subagentRunningCount = running
		sig := subagentTreeSignature(tree)
		if sig == ses.subagentBroadcastSignature {
			return nil, false
		}
		ses.subagentBroadcastSignature = sig
		return tree, true
	case attempted && ses.subagentTree != nil:
		ses.subagentTree = nil
		ses.subagentRunningCount = 0
		empty := &proto.SubagentTree{Provider: key, UpdatedAt: now.UnixMilli()}
		sig := subagentTreeSignature(empty)
		if sig == ses.subagentBroadcastSignature {
			return nil, false
		}
		ses.subagentBroadcastSignature = sig
		return empty, true
	default:
		return nil, false
	}
}

// subagentTreeSignature hashes the wire-relevant fields of a SubagentTree,
// the same change-detection role workflowProgressSignature plays for
// WorkflowProgress.
//
// LastActivityAt IS included (親 plan C10・R4; this reverses 子 plan C3 の
// 「シグネチャから除いた項目」判断, which excluded it for the same reason
// workflowProgressSignature excludes ElapsedSec/Metrics). Leaving it out meant
// a still-running child whose tool/target never changed was never
// re-broadcast, so the UI's "最後の動き N 秒前" kept climbing stale between
// genuine tree changes. subagentPollInterval (3s) already caps how often this
// can fire, so including it costs at most one subagent_tree send per poll per
// running child — unlike WorkflowProgress.ElapsedSec, which recomputes every
// tick purely from the clock and would otherwise broadcast on every single
// poll with nothing else new.
func subagentTreeSignature(t *proto.SubagentTree) string {
	if t == nil {
		return ""
	}
	var b strings.Builder
	fmt.Fprintf(&b, "%s|%d", t.Provider, t.Omitted)
	for _, n := range t.Nodes {
		fmt.Fprintf(&b, "|N:%s:%s:%d:%s:%s:%s:%s:%d:%d:%d:%d:%s:%s",
			n.ID, n.ParentID, n.Depth, n.Label, n.AgentType, n.Model, n.State,
			n.StartedAt, n.LastActivityAt, n.FinishedAt, n.ToolCalls, n.LastToolName, n.LastToolSummary)
	}
	sum := sha256.Sum256([]byte(b.String()))
	return hex.EncodeToString(sum[:])
}

// cloneSubagentTree defensively copies a SubagentTree (same idiom as
// cloneWorkflowProgress) so a reattach snapshot never aliases the Nodes slice
// between the old session (about to be discarded) and the replacement.
func cloneSubagentTree(t *proto.SubagentTree) *proto.SubagentTree {
	if t == nil {
		return nil
	}
	cp := *t
	cp.Nodes = append([]proto.SubagentNode(nil), t.Nodes...)
	return &cp
}
