package hub

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/provider"
)

// TestSubagentReaderTableMatchesCatalog は「Hub 側の対応表とカタログの
// subagent: の鍵が一致しているテスト」（子 plan C3 完了条件）。片方だけに
// 足すと落ちる: カタログにあって対応表に無い鍵、対応表にあってカタログに無い鍵
// のどちらも検出する。
func TestSubagentReaderTableMatchesCatalog(t *testing.T) {
	catalog := provider.DefaultAdapterCatalog()
	sawSubagentKey := false
	for key := range catalog.Keys {
		if !strings.HasPrefix(key, "subagent:") {
			continue
		}
		sawSubagentKey = true
		if _, ok := subagentReaderByKey[key]; !ok {
			t.Errorf("catalog key %q has no entry in subagentReaderByKey", key)
		}
	}
	if !sawSubagentKey {
		t.Fatal("DefaultAdapterCatalog has no subagent: keys to check subagentReaderByKey against")
	}
	for key := range subagentReaderByKey {
		if !catalog.Has(key) {
			t.Errorf("subagentReaderByKey key %q is not registered in DefaultAdapterCatalog", key)
		}
	}
}

// TestRunSubagentTreePollStartupGatesDoNotReschedule は子 plan C3 完了条件
// 「SubagentTreeEnabled が false のとき、adapters.subagents が空のとき、
// 対応表に無い鍵のときは、どれもタイマーが起動しない」を確認する。
// runSubagentTreePoll はティックのたびに ses.subagentTimer = nil を書いてから
// 再スケジュールを試みるので、呼び出し後も nil のままなら
// 「再スケジュールされなかった」＝実質「タイマーが起動しない」ことの直接の証拠になる。
func TestRunSubagentTreePollStartupGatesDoNotReschedule(t *testing.T) {
	t.Run("config disabled", func(t *testing.T) {
		s := newTestServer()
		s.cfg.Workflow.SubagentTreeEnabled = false
		ses := registerTestSession(s, 1, "claude")
		s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
		if ses.subagentTimer != nil {
			t.Fatal("timer was rescheduled although SubagentTreeEnabled=false")
		}
	})

	t.Run("adapters.subagents empty", func(t *testing.T) {
		// 実 provider（claude/codex）はどちらも本 plan で adapters.subagents を
		// 獲得済みなので、「持たない」ケースは実在の embedded 定義に頼らず、
		// TestRunSubagentTreePollDispatchesByDefinitionKeyNotProviderName と同じ
		// 形の合成定義（Adapters.Subagents を明示的に空のまま）で作る。
		definitions, diagnostics, err := provider.EmbeddedDefinitions()
		if err != nil {
			t.Fatal(err)
		}
		if len(diagnostics) != 0 {
			t.Fatalf("embedded diagnostics = %#v", diagnostics)
		}
		noSubagents := provider.Definition{
			SchemaVersion: provider.CurrentSchemaVersion,
			ID:            "no-subagents-provider",
			DisplayName:   "No Subagents Provider",
			Launch:        &provider.LaunchDefinition{Executable: "test"},
		}
		registry, buildDiagnostics := provider.Build(provider.Layers{
			Embedded: definitions,
			User:     []provider.Definition{noSubagents},
		}, provider.DefaultAdapterCatalog())
		if len(buildDiagnostics) != 0 {
			t.Fatalf("Build diagnostics = %#v", buildDiagnostics)
		}
		if def, ok := registry.Lookup("no-subagents-provider"); !ok || def.Adapters.Subagents != "" {
			t.Fatalf("test fixture definition unexpectedly has adapters.subagents: %+v", def)
		}

		s := newTestServer()
		s.cfg.Workflow.SubagentTreeEnabled = true
		s.providers = registry
		ses := registerTestSession(s, 1, "no-subagents-provider")
		s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
		if ses.subagentTimer != nil {
			t.Fatal("timer was rescheduled although the provider definition has no adapters.subagents")
		}
	})

	t.Run("key not in subagentReaderByKey", func(t *testing.T) {
		s := newTestServer()
		s.cfg.Workflow.SubagentTreeEnabled = true
		ses := registerTestSession(s, 1, "claude") // claude.json は subagent:claude-v1 を持つ
		saved := subagentReaderByKey["subagent:claude-v1"]
		delete(subagentReaderByKey, "subagent:claude-v1")
		defer func() { subagentReaderByKey["subagent:claude-v1"] = saved }()

		s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
		if ses.subagentTimer != nil {
			t.Fatal("timer was rescheduled although the selected key has no registered reader")
		}
	})
}

// TestRunSubagentTreePollDispatchesByDefinitionKeyNotProviderName は子 plan
// C3 完了条件「provider 名が claude ではない定義でも、adapters.subagents が
// subagent:claude-v1 なら Claude の読み取りで木が作られる」を確認する。
//
// 実 Claude reader（internal/hub/subagent_source_claude.go の
// readClaudeSubagentTree）は本テスト作成時点でまだ実装されていない（子 plan
// C2 が別セッションで並行実装中）ため、subagentReaderByKey /
// subagentPathResolverByKey の "subagent:claude-v1" エントリをテスト用の偽の
// 読み取り関数へ一時的に差し替えて確認する。これは「定義の鍵で読み取り関数が
// 選ばれ、provider 名では選ばれない」という配線そのものの検証であり、実物の
// Claude 読み取りの正しさは検証しない（別 C の完了条件）。
func TestRunSubagentTreePollDispatchesByDefinitionKeyNotProviderName(t *testing.T) {
	definitions, diagnostics, err := provider.EmbeddedDefinitions()
	if err != nil {
		t.Fatal(err)
	}
	if len(diagnostics) != 0 {
		t.Fatalf("embedded diagnostics = %#v", diagnostics)
	}
	custom := provider.Definition{
		SchemaVersion: provider.CurrentSchemaVersion,
		ID:            "my-claude-like",
		DisplayName:   "My Claude-like",
		Launch:        &provider.LaunchDefinition{Executable: "test"},
		Adapters:      provider.AdapterRefs{Subagents: "subagent:claude-v1"},
	}
	registry, buildDiagnostics := provider.Build(provider.Layers{
		Embedded: definitions,
		User:     []provider.Definition{custom},
	}, provider.DefaultAdapterCatalog())
	if len(buildDiagnostics) != 0 {
		t.Fatalf("Build diagnostics = %#v", buildDiagnostics)
	}
	def, ok := registry.Lookup("my-claude-like")
	if !ok || def.Adapters.Subagents != "subagent:claude-v1" {
		t.Fatalf("test fixture definition did not select subagent:claude-v1: %+v", def)
	}

	s := newTestServer()
	s.cfg.Workflow.SubagentTreeEnabled = true
	s.providers = registry
	ses := registerTestSession(s, 1, "my-claude-like")

	var gotPath string
	var gotSince time.Time
	savedReader := subagentReaderByKey["subagent:claude-v1"]
	savedResolver := subagentPathResolverByKey["subagent:claude-v1"]
	subagentReaderByKey["subagent:claude-v1"] = func(transcriptPath string, since time.Time, prior any, budget subagentReadBudget) (*proto.SubagentTree, any, error) {
		gotPath, gotSince = transcriptPath, since
		return &proto.SubagentTree{
			Provider: "subagent:claude-v1",
			Nodes:    []proto.SubagentNode{{ID: "child-1", State: "running", Label: "fake-child"}},
		}, "next-state", nil
	}
	subagentPathResolverByKey["subagent:claude-v1"] = func(info subagentParentInfo) (string, bool) {
		return "/fake/transcript.jsonl", true
	}
	defer func() {
		subagentReaderByKey["subagent:claude-v1"] = savedReader
		subagentPathResolverByKey["subagent:claude-v1"] = savedResolver
	}()

	turnStart := time.Now().Add(-time.Minute)
	s.sessionsMu.Lock()
	ses.subagentTurnStartedAt = turnStart
	s.sessionsMu.Unlock()

	s.runSubagentTreePoll(1, ses, ses.subagentGeneration)

	if gotPath != "/fake/transcript.jsonl" {
		t.Fatalf("fake reader received transcriptPath = %q, want the resolved fake path", gotPath)
	}
	if !gotSince.Equal(turnStart) {
		t.Fatalf("fake reader received since = %v, want %v", gotSince, turnStart)
	}
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if ses.subagentTree == nil || len(ses.subagentTree.Nodes) != 1 || ses.subagentTree.Nodes[0].ID != "child-1" {
		t.Fatalf("tree built via the fake claude-v1 reader was not applied to the session: %+v", ses.subagentTree)
	}
	if ses.subagentRunningCount != 1 {
		t.Fatalf("subagentRunningCount = %d, want 1", ses.subagentRunningCount)
	}
	if ses.subagentTimer == nil {
		t.Fatal("an eligible session must reschedule its next poll")
	}
	ses.subagentTimer.Stop()
}

// TestRunSubagentTreePollUsesRealClaudeReaderForNonClaudeProviderName is the
// end-to-end confirmation of 子 plan C3 完了条件「provider 名が claude では
// ない定義でも、adapters.subagents が subagent:claude-v1 なら Claude の読み取り
// で木が作られる」using the real reader
// (internal/hub/subagent_source_claude.go's readClaudeSubagentTree, landed by
// 子 plan C2 in a parallel session — writeClaudeJSONLLines /
// writeClaudeSubagentMetaFile / claudeSubagentMeta / claudeChildLine /
// claudeTextBlock / claudeParentToolUseLine are that C's fixture helpers,
// reused here as-is) rather than the fake stand-in
// TestRunSubagentTreePollDispatchesByDefinitionKeyNotProviderName used while
// the real reader did not exist yet. Both tests are kept: the fake-reader one
// isolates the dispatch mechanism itself, this one proves the real reader is
// reachable through a non-"claude" provider id.
func TestRunSubagentTreePollUsesRealClaudeReaderForNonClaudeProviderName(t *testing.T) {
	claudeDir := t.TempDir()
	cwd := filepath.Join(t.TempDir(), "repo")
	sessionID := "11111111-2222-4333-8444-555555555555"
	projectDir := filepath.Join(claudeDir, "projects", claudeProjectDirName(cwd))
	if err := os.MkdirAll(projectDir, 0o755); err != nil {
		t.Fatal(err)
	}
	parentPath := filepath.Join(projectDir, sessionID+".jsonl")
	writeClaudeJSONLLines(t, parentPath, claudeParentToolUseLine("toolu_c1"))

	subDir := filepath.Join(projectDir, sessionID, "subagents")
	if err := os.MkdirAll(subDir, 0o755); err != nil {
		t.Fatal(err)
	}
	writeClaudeSubagentMetaFile(t, subDir, "c1", claudeSubagentMeta{
		AgentType: "general-purpose", Description: "child one", ToolUseID: "toolu_c1", SpawnDepth: 1,
	})
	writeClaudeJSONLLines(t, filepath.Join(subDir, "agent-c1.jsonl"),
		claudeChildLine("assistant", "2026-09-23T10:00:00.000Z", []map[string]any{claudeTextBlock("hi")}))

	definitions, diagnostics, err := provider.EmbeddedDefinitions()
	if err != nil {
		t.Fatal(err)
	}
	if len(diagnostics) != 0 {
		t.Fatalf("embedded diagnostics = %#v", diagnostics)
	}
	custom := provider.Definition{
		SchemaVersion: provider.CurrentSchemaVersion,
		ID:            "my-claude-like",
		DisplayName:   "My Claude-like",
		Launch:        &provider.LaunchDefinition{Executable: "test"},
		Adapters:      provider.AdapterRefs{Subagents: "subagent:claude-v1"},
	}
	registry, buildDiagnostics := provider.Build(provider.Layers{
		Embedded: definitions,
		User:     []provider.Definition{custom},
	}, provider.DefaultAdapterCatalog())
	if len(buildDiagnostics) != 0 {
		t.Fatalf("Build diagnostics = %#v", buildDiagnostics)
	}

	s := newTestServer()
	s.cfg.Workflow.SubagentTreeEnabled = true
	s.providers = registry
	ses := registerTestSession(s, 1, "my-claude-like")
	s.sessionsMu.Lock()
	ses.CWD = cwd
	ses.ClaudeDir = claudeDir
	ses.AgentSessionID = sessionID
	s.sessionsMu.Unlock()

	s.runSubagentTreePoll(1, ses, ses.subagentGeneration)

	s.sessionsMu.Lock()
	tree := ses.subagentTree
	timer := ses.subagentTimer
	s.sessionsMu.Unlock()
	if timer != nil {
		timer.Stop()
	}
	if tree == nil || len(tree.Nodes) != 1 || tree.Nodes[0].ID != "c1" {
		t.Fatalf("real Claude reader dispatched via provider id %q (not \"claude\") did not build the expected tree: %+v", ses.Provider, tree)
	}
}

// TestSubagentTreeForBroadcastLockedChangeDetection は子 plan C3 完了条件
// 「木が変わらない間は subagent_tree を送らない。変わったときは1回だけ送る」を、
// 送信回数を数えて確認する。あわせて「子が0件になったときは、空の木を1回だけ
// 送って画面から消す」と、「読み取りに失敗した(attempted=false)ときは、直前まで
// 走行中だった子を勝手に消さない」（親 plan 方針5）も確認する。
func TestSubagentTreeForBroadcastLockedChangeDetection(t *testing.T) {
	s := newTestServer()
	ses := &session{ID: 1}
	now := time.Now()

	treeA := &proto.SubagentTree{Nodes: []proto.SubagentNode{{ID: "c1", State: "running"}}}
	out, broadcast := s.subagentTreeForBroadcastLocked(ses, "subagent:claude-v1", treeA, true, now)
	if !broadcast || out == nil {
		t.Fatal("first non-empty tree must broadcast")
	}
	if ses.subagentRunningCount != 1 {
		t.Fatalf("subagentRunningCount = %d, want 1", ses.subagentRunningCount)
	}

	// 同じ内容（新しいポインタだが同じフィールド値）の 2 回目のティック: 送らない。
	treeASame := &proto.SubagentTree{Nodes: []proto.SubagentNode{{ID: "c1", State: "running"}}}
	_, broadcast = s.subagentTreeForBroadcastLocked(ses, "subagent:claude-v1", treeASame, true, now.Add(time.Second))
	if broadcast {
		t.Fatal("unchanged tree must not broadcast again")
	}

	// 状態が変わった: 送る。
	treeB := &proto.SubagentTree{Nodes: []proto.SubagentNode{{ID: "c1", State: "done"}}}
	_, broadcast = s.subagentTreeForBroadcastLocked(ses, "subagent:claude-v1", treeB, true, now.Add(2*time.Second))
	if !broadcast {
		t.Fatal("a changed node state must broadcast")
	}
	if ses.subagentRunningCount != 0 {
		t.Fatalf("subagentRunningCount = %d, want 0 after the child finished", ses.subagentRunningCount)
	}

	// reader が読み取りに失敗した（attempted=false）: 直前の木をそのまま保持し、送らない。
	_, broadcast = s.subagentTreeForBroadcastLocked(ses, "subagent:claude-v1", nil, false, now.Add(3*time.Second))
	if broadcast {
		t.Fatal("a failed/unresolved read must not broadcast")
	}
	if ses.subagentTree == nil {
		t.Fatal("a failed/unresolved read must not clear the last-known tree")
	}

	// reader が実際に「子なし」を返した（attempted=true）: 空の木を1回だけ送る。
	out, broadcast = s.subagentTreeForBroadcastLocked(ses, "subagent:claude-v1", nil, true, now.Add(4*time.Second))
	if !broadcast || out == nil || len(out.Nodes) != 0 {
		t.Fatalf("the transition to zero children must broadcast one empty tree, got broadcast=%v out=%+v", broadcast, out)
	}
	if ses.subagentTree != nil {
		t.Fatal("session must forget the tree once it broadcasts the cleared state")
	}

	// 既に空の状態でさらに「子なし」が続く: もう送らない（子が1本も無いセッションの規律と同じ）。
	_, broadcast = s.subagentTreeForBroadcastLocked(ses, "subagent:claude-v1", nil, true, now.Add(5*time.Second))
	if broadcast {
		t.Fatal("repeated 'no children' must not re-broadcast an already-cleared tree")
	}
}

// TestSubagentTurnStartAdvanceLockedGating は子 plan C3 完了条件のうち turn-start
// ゲート 3 点を確認する:
//   - 走行中の子が1人でもいる間はユーザー入力が来ても区切りの時刻が動かない
//   - 走行中の子が0人になった後の最初のユーザー入力で区切りの時刻が進む
//   - unknown の子だけが残っている状態（subagentRunningCount は running だけを
//     数えるので 0）でのユーザー入力では区切りの時刻が進む
func TestSubagentTurnStartAdvanceLockedGating(t *testing.T) {
	base := time.Now().Add(-time.Hour)
	ses := &session{subagentTurnStartedAt: base, subagentRunningCount: 1}

	laterWhileRunning := base.Add(time.Minute)
	subagentTurnStartAdvanceLocked(ses, laterWhileRunning)
	if !ses.subagentTurnStartedAt.Equal(base) {
		t.Fatalf("turn start advanced while a child was still running: got %v, want %v", ses.subagentTurnStartedAt, base)
	}

	ses.subagentRunningCount = 0
	afterChildrenDone := base.Add(2 * time.Minute)
	subagentTurnStartAdvanceLocked(ses, afterChildrenDone)
	if !ses.subagentTurnStartedAt.Equal(afterChildrenDone) {
		t.Fatalf("turn start did not advance once running children reached 0: got %v, want %v", ses.subagentTurnStartedAt, afterChildrenDone)
	}

	// unknown だけが残っている木: running としては数えない
	// (subagentTreeForBroadcastLocked のカウントが証拠)。
	unknownOnly := &proto.SubagentTree{Nodes: []proto.SubagentNode{{ID: "c1", State: "unknown"}}}
	s := newTestServer()
	broadcastSes := &session{}
	s.subagentTreeForBroadcastLocked(broadcastSes, "subagent:claude-v1", unknownOnly, true, time.Now())
	if broadcastSes.subagentRunningCount != 0 {
		t.Fatalf("a tree with only 'unknown' children must count 0 running, got %d", broadcastSes.subagentRunningCount)
	}
	broadcastSes.subagentTurnStartedAt = base
	nextInput := base.Add(3 * time.Minute)
	subagentTurnStartAdvanceLocked(broadcastSes, nextInput)
	if !broadcastSes.subagentTurnStartedAt.Equal(nextInput) {
		t.Fatalf("turn start did not advance with only unknown children present: got %v, want %v", broadcastSes.subagentTurnStartedAt, nextInput)
	}
}

// TestScheduleSubagentTreePollLockedReplacesPriorTimer は再スケジュール時に
// generation が進み、古いタイマーが発火しても新しいものと衝突しないことを
// 確認する（他の 3 つの timer chain と同じガード形）。
func TestScheduleSubagentTreePollLockedReplacesPriorTimer(t *testing.T) {
	s := newTestServer()
	s.cfg.Workflow.SubagentTreeEnabled = false // ティックがすぐ止まるようにする
	ses := registerTestSession(s, 1, "claude")

	s.sessionsMu.Lock()
	s.scheduleSubagentTreePollLocked(1, ses, time.Hour) // まだ発火しない長い delay
	firstGen := ses.subagentGeneration
	s.scheduleSubagentTreePollLocked(1, ses, time.Hour) // 差し替え
	secondGen := ses.subagentGeneration
	s.sessionsMu.Unlock()

	if secondGen == firstGen {
		t.Fatal("rescheduling must bump the generation guard")
	}
	if ses.subagentTimer == nil {
		t.Fatal("scheduling must leave a live timer")
	}
	ses.subagentTimer.Stop()
}

// TestRunSubagentTreePollStopsWhenSessionRemoved は子 plan C3 完了条件
// 「セッションを消すとタイマーが止まる」を確認する。runSubagentTreePoll は
// ses が s.sessions から消えていれば早期リターンし、再スケジュールしない。
func TestRunSubagentTreePollStopsWhenSessionRemoved(t *testing.T) {
	s := newTestServer()
	s.cfg.Workflow.SubagentTreeEnabled = true
	ses := registerTestSession(s, 1, "claude")

	s.sessionsMu.Lock()
	delete(s.sessions, 1)
	s.sessionsMu.Unlock()

	s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
	if ses.subagentTimer != nil {
		t.Fatal("poll must not reschedule once the session is gone")
	}
}

// TestResolveClaudeSubagentParentPath は agentLogForSession の claude 分岐と
// 同じ解決規則を、provider 名ではなく渡された情報だけから行うことを確認する。
func TestResolveClaudeSubagentParentPath(t *testing.T) {
	if _, ok := resolveClaudeSubagentParentPath(subagentParentInfo{}); ok {
		t.Fatal("empty info must not resolve a path")
	}
	if _, ok := resolveClaudeSubagentParentPath(subagentParentInfo{CWD: "/repo"}); ok {
		t.Fatal("missing ClaudeDir/HomeDir must not resolve a path")
	}
	// AgentSessionID が無く StartedAt も不正な時刻形式: 解決できない。
	if _, ok := resolveClaudeSubagentParentPath(subagentParentInfo{CWD: "/repo", ClaudeDir: "/home/.claude", StartedAt: "not-a-time"}); ok {
		t.Fatal("unparseable StartedAt without an AgentSessionID must not resolve a path")
	}
}

// --- 親 plan plan_subagent-tree-popup.md C10（敵対レビュー R3・R4・R6・R7 の
// 直し）--------------------------------------------------------------------

// TestSubagentTreeSignatureChangesWithLastActivityAt は C10（R4）の直し「変化の
// 判定に LastActivityAt を含める」を確認する。ツール名・対象が変わらず
// LastActivityAt だけが進んだ 2 つの木で、シグネチャが変わること・
// subagentTreeForBroadcastLocked が再送すること（=「最後の動き N 秒前」が
// 3 秒ごとに更新されうること）の両方を見る。
func TestSubagentTreeSignatureChangesWithLastActivityAt(t *testing.T) {
	base := &proto.SubagentTree{Nodes: []proto.SubagentNode{{ID: "c1", State: "running", LastActivityAt: 1000}}}
	moved := &proto.SubagentTree{Nodes: []proto.SubagentNode{{ID: "c1", State: "running", LastActivityAt: 2000}}}
	if subagentTreeSignature(base) == subagentTreeSignature(moved) {
		t.Fatal("subagentTreeSignature must change when only LastActivityAt advances (R4)")
	}

	s := newTestServer()
	ses := &session{ID: 1}
	now := time.Now()
	if _, broadcast := s.subagentTreeForBroadcastLocked(ses, "subagent:claude-v1", base, true, now); !broadcast {
		t.Fatal("first non-empty tree must broadcast")
	}
	if _, broadcast := s.subagentTreeForBroadcastLocked(ses, "subagent:claude-v1", moved, true, now.Add(time.Second)); !broadcast {
		t.Fatal("a poll where only LastActivityAt advanced must re-broadcast (R4)")
	}
}

// TestSendSnapshotIncludesLatestSubagentTreePerSession は C10（R3）の直し
// 「ブラウザの新規接続・再読み込み時の初期データに、各セッションの最新の木を
// subagent_tree として入れる」を確認する。木を持つセッションだけが 1 通届き、
// 木を持たないセッションの分は届かないこと、送られる木がセッション側の
// ポインタの複製（別インスタンス）であることを見る。
func TestSendSnapshotIncludesLatestSubagentTreePerSession(t *testing.T) {
	s := newTestServer()
	withTree := registerTestSession(s, 1, "claude")
	registerTestSession(s, 2, "codex") // 木を持たない

	tree := &proto.SubagentTree{Provider: "subagent:claude-v1", Nodes: []proto.SubagentNode{{ID: "c1", State: "running"}}}
	s.sessionsMu.Lock()
	withTree.subagentTree = tree
	s.sessionsMu.Unlock()

	uc, got := captureSnapshotUI()
	s.sendSnapshot(uc)

	var trees []proto.Message
	for _, m := range got() {
		if m.Type == "subagent_tree" {
			trees = append(trees, m)
		}
	}
	if len(trees) != 1 {
		t.Fatalf("subagent_tree messages = %d, want 1 (only the session holding a tree)", len(trees))
	}
	if trees[0].SessionID != 1 {
		t.Fatalf("subagent_tree session id = %d, want 1", trees[0].SessionID)
	}
	if trees[0].SubagentTree == nil || len(trees[0].SubagentTree.Nodes) != 1 || trees[0].SubagentTree.Nodes[0].ID != "c1" {
		t.Fatalf("subagent_tree payload = %+v, want the session's stored tree", trees[0].SubagentTree)
	}
	if trees[0].SubagentTree == tree {
		t.Fatal("sendSnapshot must send a clone of ses.subagentTree, not alias it")
	}
}

// TestRestartSubagentTreePollLockedFallsBackToSessionStartWhenTurnStartIsZero
// は C10（R7）の直し「reattach 後に区切りの時刻がゼロのまま poll が始まる
// 場合は、セッションの開始時刻を使う」を確認する。
func TestRestartSubagentTreePollLockedFallsBackToSessionStartWhenTurnStartIsZero(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	startedAt := time.Date(2026, 9, 20, 10, 0, 0, 0, time.UTC)

	s.sessionsMu.Lock()
	ses.StartedAt = startedAt.Format(time.RFC3339)
	// ses.subagentTurnStartedAt はゼロ値のまま（最初の確定ユーザー入力より前に
	// reattach された場合の状態）。
	s.restartSubagentTreePollLocked(1, ses, time.Now())
	got := ses.subagentTurnStartedAt
	if ses.subagentTimer != nil {
		ses.subagentTimer.Stop()
		ses.subagentTimer = nil
	}
	s.sessionsMu.Unlock()

	if !got.Equal(startedAt) {
		t.Fatalf("subagentTurnStartedAt = %v, want the session start time %v (R7)", got, startedAt)
	}
}

// TestRestartSubagentTreePollLockedKeepsExistingTurnStart は上のテストの対:
// 区切りの時刻がすでにゼロでないなら、reattach の再開で上書きしない
// （親 plan 方針4「子が走っている途中で親に質問しても区切らない」を reattach
// でも壊さない）。
func TestRestartSubagentTreePollLockedKeepsExistingTurnStart(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	existing := time.Now().Add(-time.Minute)

	s.sessionsMu.Lock()
	ses.subagentTurnStartedAt = existing
	s.restartSubagentTreePollLocked(1, ses, time.Now())
	got := ses.subagentTurnStartedAt
	if ses.subagentTimer != nil {
		ses.subagentTimer.Stop()
		ses.subagentTimer = nil
	}
	s.sessionsMu.Unlock()

	if !got.Equal(existing) {
		t.Fatalf("subagentTurnStartedAt = %v, must not be overwritten when already non-zero, want %v", got, existing)
	}
}

// TestSubagentFallbackTurnStartUsesNowWhenStartedAtUnparsable は
// subagentFallbackTurnStart 単体で、StartedAt が読めないときに now へ倒れる
// （ゼロ値へは倒れない）ことを確認する。
func TestSubagentFallbackTurnStartUsesNowWhenStartedAtUnparsable(t *testing.T) {
	now := time.Now()
	ses := &session{StartedAt: "not-a-time"}
	got := subagentFallbackTurnStart(ses, now)
	if !got.Equal(now) {
		t.Fatalf("subagentFallbackTurnStart = %v, want now (%v) when StartedAt cannot be parsed", got, now)
	}
}

// TestRunSubagentTreePollCachesResolvedParentPath は C10（R6）の直し「親の
// 記録のパス解決の結果をセッションごとに覚え、毎 poll 解決し直さない」を、
// 解決関数の呼び出し回数を数えて確認する。覚えたパスが実在する間は呼ばれず、
// パスが消えたときはキャッシュを捨てるが、実際の再解決は 30 秒に 1 回までの
// レート制限に従う（本 C の実装時の確定; time.Sleep は使わず
// subagentParentPathAttemptedAt を直接繰り下げて 30 秒経過をシミュレートする）。
func TestRunSubagentTreePollCachesResolvedParentPath(t *testing.T) {
	tmp := t.TempDir()
	path := filepath.Join(tmp, "parent.jsonl")
	if err := os.WriteFile(path, []byte("{}"), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestServer()
	s.cfg.Workflow.SubagentTreeEnabled = true
	ses := registerTestSession(s, 1, "claude") // claude.json は subagent:claude-v1 を持つ

	var resolveCalls int
	savedResolver := subagentPathResolverByKey["subagent:claude-v1"]
	subagentPathResolverByKey["subagent:claude-v1"] = func(info subagentParentInfo) (string, bool) {
		resolveCalls++
		return path, true
	}
	savedReader := subagentReaderByKey["subagent:claude-v1"]
	subagentReaderByKey["subagent:claude-v1"] = func(transcriptPath string, since time.Time, prior any, budget subagentReadBudget) (*proto.SubagentTree, any, error) {
		return nil, prior, nil // 「子なし」を返すだけの単純な reader。パス解決の検証に専念する。
	}
	defer func() {
		subagentPathResolverByKey["subagent:claude-v1"] = savedResolver
		subagentReaderByKey["subagent:claude-v1"] = savedReader
	}()

	stopTimer := func() {
		s.sessionsMu.Lock()
		if ses.subagentTimer != nil {
			ses.subagentTimer.Stop()
			ses.subagentTimer = nil
		}
		s.sessionsMu.Unlock()
	}

	// 1st poll: まだ一度も解決していない -> 1 回だけ解決する。
	s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
	stopTimer()
	if resolveCalls != 1 {
		t.Fatalf("resolveCalls after the first poll = %d, want 1", resolveCalls)
	}
	s.sessionsMu.Lock()
	gotPath := ses.subagentParentPath
	s.sessionsMu.Unlock()
	if gotPath != path {
		t.Fatalf("subagentParentPath = %q, want the resolved path %q", gotPath, path)
	}

	// 2nd poll: 覚えたパスがまだ実在する -> resolver を呼び直さない。
	s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
	stopTimer()
	if resolveCalls != 1 {
		t.Fatalf("resolveCalls after the second poll (path still cached and existing) = %d, want still 1", resolveCalls)
	}

	// 覚えたパスが消える。直前の解決からまだ 30 秒経っていないので、
	// 実際の再解決はレート制限に阻まれる（キャッシュは捨てられる）。
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
	stopTimer()
	if resolveCalls != 1 {
		t.Fatalf("resolveCalls right after the cached path disappeared (still inside the 30s retry window) = %d, want still 1", resolveCalls)
	}
	s.sessionsMu.Lock()
	if ses.subagentParentPath != "" {
		t.Fatalf("subagentParentPath must be cleared once the cached path is confirmed gone, got %q", ses.subagentParentPath)
	}
	// 30 秒の再試行窓が経過したのと同じ状態にする。
	ses.subagentParentPathAttemptedAt = time.Now().Add(-subagentPathResolveRetryInterval - time.Second)
	s.sessionsMu.Unlock()

	s.runSubagentTreePoll(1, ses, ses.subagentGeneration)
	stopTimer()
	if resolveCalls != 2 {
		t.Fatalf("resolveCalls once the 30s retry window has elapsed = %d, want 2", resolveCalls)
	}
}

// TestSubagentTreePollStoppedOnSessionEnd は監査 F-01 の修正検証テスト。
// セッション終了（CLI 切断・終了）時および dismiss 時に subagent tree のポーリングタイマーが
// 確実に停止され、終端状態（StateExited等）のセッションに対して runSubagentTreePoll が
// 再スケジュールを行わないことを検証する。
func TestSubagentTreePollStoppedOnSessionEnd(t *testing.T) {
	t.Run("finalizeSubagentTreeOnSessionEnd stops timer and increments generation", func(t *testing.T) {
		s := newTestServer()
		s.cfg.Workflow.SubagentTreeEnabled = true
		ses := registerTestSession(s, 1, "claude")

		// タイマーを疑似的にセット
		s.sessionsMu.Lock()
		ses.subagentTimer = time.AfterFunc(time.Hour, func() {})
		initialGen := ses.subagentGeneration
		s.sessionsMu.Unlock()

		// セッション終了通知
		s.finalizeSubagentTreeOnSessionEnd(1)

		s.sessionsMu.Lock()
		defer s.sessionsMu.Unlock()
		if ses.subagentTimer != nil {
			t.Fatal("finalizeSubagentTreeOnSessionEnd did not clear subagentTimer")
		}
		if ses.subagentGeneration <= initialGen {
			t.Fatalf("generation = %d, want > initialGen %d", ses.subagentGeneration, initialGen)
		}
	})

	t.Run("runSubagentTreePoll does not reschedule on terminal session state", func(t *testing.T) {
		terminalStates := []string{"completed", "error", "disconnected", "done", "timeout", "dismissed"}
		for _, st := range terminalStates {
			t.Run(st, func(t *testing.T) {
				s := newTestServer()
				s.cfg.Workflow.SubagentTreeEnabled = true
				ses := registerTestSession(s, 1, "claude")

				s.sessionsMu.Lock()
				ses.State = st
				s.sessionsMu.Unlock()

				s.runSubagentTreePoll(1, ses, ses.subagentGeneration)

				s.sessionsMu.Lock()
				defer s.sessionsMu.Unlock()
				if ses.subagentTimer != nil {
					t.Fatalf("runSubagentTreePoll rescheduled timer on terminal session state %q", st)
				}
			})
		}
	})

	t.Run("runSubagentTreePoll does not reschedule on stale generation", func(t *testing.T) {
		s := newTestServer()
		s.cfg.Workflow.SubagentTreeEnabled = true
		ses := registerTestSession(s, 1, "claude")

		s.sessionsMu.Lock()
		curGen := ses.subagentGeneration
		s.sessionsMu.Unlock()

		// 古い世代番号で poll を呼び出す
		s.runSubagentTreePoll(1, ses, curGen-1)

		s.sessionsMu.Lock()
		defer s.sessionsMu.Unlock()
		if ses.subagentTimer != nil {
			t.Fatal("runSubagentTreePoll rescheduled timer with stale generation")
		}
	})

	t.Run("stopSubagentTreePollLocked stops timer", func(t *testing.T) {
		s := newTestServer()
		s.cfg.Workflow.SubagentTreeEnabled = true
		ses := registerTestSession(s, 1, "claude")

		s.sessionsMu.Lock()
		ses.subagentTimer = time.AfterFunc(time.Hour, func() {})
		initialGen := ses.subagentGeneration
		s.stopSubagentTreePollLocked(ses)
		defer s.sessionsMu.Unlock()

		if ses.subagentTimer != nil {
			t.Fatal("stopSubagentTreePollLocked did not clear subagentTimer")
		}
		if ses.subagentGeneration <= initialGen {
			t.Fatalf("generation = %d, want > initialGen %d", ses.subagentGeneration, initialGen)
		}
	})
}
