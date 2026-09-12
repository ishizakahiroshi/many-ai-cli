package hub

// handoff.go is the single window from Hub into internal/handoff
// (親 plan の方針: 「配線の入口を 1 本にして、後から allowlist を点検しやすく
// する」。子 plan: docs/local/plan_session-handoff-board_c2_machine-layer.md
// C1 作業内容)。every other hub file that wants a line written to a session's
// handoff log calls a function in this file; nothing else in this package
// imports internal/handoff directly.
//
// This file must never read config.LogConfig.SessionEnabled (子 plan C2 の
// 肝). Handoff recording has its own gate, config.HandoffConfig, so a user
// who has never opted into the raw session log (existing default: off) still
// gets a filled-in board. handoffEnabled below is the only gate; every write
// path funnels through appendHandoff, which checks it once.
import (
	"context"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"

	"many-ai-cli/internal/handoff"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
	"many-ai-cli/internal/subscription"
)

// handoffEnabled reports whether handoff recording is on for this Hub
// process. It is re-read on every call (not cached) because config can be
// reloaded while sessions are live, matching s.doneSummaryNotifyEnabled's
// pattern for the same reason.
func (s *Server) handoffEnabled() bool {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	return s.cfg.Handoff.EnabledOrDefault()
}

// appendHandoff writes one record, unless handoff recording is disabled. A
// write failure is logged, not surfaced: recording is not any session's
// primary job (mirrors handoff.Append's own doc comment).
func (s *Server) appendHandoff(sessionID int, r handoff.Record) {
	if !s.handoffEnabled() {
		return
	}
	if err := handoff.Append(sessionID, r); err != nil {
		s.logger.Warn("handoff append failed", "session_id", sessionID, "kind", r.Kind, "err", err)
	}
}

// recordHandoffSessionStart writes a session_start line at initial
// registration (not on reattach — reattach continues the same session id, it
// does not start a new one). Only public session metadata goes in; the
// user's initial prompt is never available here and must never be added.
//
// handoffFrom is the predecessor session's ID when this session was spawned
// from a handoff (子 plan: docs/local/plan_session-handoff-board_c5_handoff-md.md
// 内部 C3), or 0 for an ordinary spawn. It comes from pendingChild.HandoffFrom,
// which spawn_handler.go populates the same way it already does for
// InitialPrompt — the identity of "which session this one continues" lives on
// the successor's own board, not on a UI parent/child relationship.
func (s *Server) recordHandoffSessionStart(sessionID int, provider, cwd, branch, model, subscriptionID string, handoffFrom int) {
	s.appendHandoff(sessionID, handoff.Record{
		Kind:           handoff.KindSessionStart,
		Provider:       provider,
		CWD:            cwd,
		Branch:         branch,
		Model:          model,
		SubscriptionID: subscriptionID,
		HandoffFrom:    handoffFrom,
		Transcript:     s.handoffTranscriptPathFor(sessionID),
	})
}

// handoffTranscriptPathFor resolves the provider's own conversation log for a
// live session, or "" when it cannot be resolved yet (子 plan:
// docs/local/plan_derived-session-launch_c4_handoff-routes.md 内部 C1).
//
// It deliberately owns no resolution logic of its own: the profile-aware
// lookup (CLAUDE_CONFIG_DIR / CODEX_HOME per subscription profile, Codex's
// Stop-hook NativeLogPath before the start-time search) already exists as
// agentChatTranscriptPathForSnapshot, and a second copy here would drift from
// it the first time a provider's layout changes. Providers that resolver does
// not cover return "" — no new `case "claude":` switch is introduced here
// (親 plan 不変条件 7).
//
// Only a path is returned, and only a path is ever recorded; the file itself is
// never opened by many-ai-cli (internal/handoff/handoff.go package doc).
func (s *Server) handoffTranscriptPathFor(sessionID int) string {
	path, ok := agentChatTranscriptPathForSnapshot(s.agentChatSnapshot(sessionID))
	if !ok {
		return ""
	}
	return path
}

// recordHandoffSessionEnd writes a session_end line. state and reason are
// always one of a small set of code-computed labels (session state names,
// wrapper.classifyStartFailure's reason codes such as "exec_not_found") —
// never PTY output or user text — so folding them into Text (the allowlist's
// one free-form field) does not widen what a Record can carry.
//
// The transcript path is resolved here too, not only at session_start: Codex
// only learns its rollout file from its Stop hook (or from the start-time
// search once the file exists), so for a Codex session this is usually the
// first moment the path is knowable at all (子 plan 内部 C1). Every caller of
// this function runs after the session has been marked ended but before it is
// removed from s.sessions, so the snapshot still resolves.
func (s *Server) recordHandoffSessionEnd(sessionID int, state, reason string) {
	text := strings.TrimSpace(state)
	if reason != "" {
		text = strings.TrimSpace(text + ": " + reason)
	}
	s.appendHandoff(sessionID, handoff.Record{
		Kind:       handoff.KindSessionEnd,
		Text:       text,
		Transcript: s.handoffTranscriptPathFor(sessionID),
	})
}

// recordHandoffDone writes a kind=done line for one completion. Both
// publishDoneSummary and publishRelayDoneSummary flow through
// publishDoneSummaryInternal, so a relay completion lands here as the same
// one line as an ordinary one (子 plan C2 「看板側では通常の完了と同じ 1 行
// に留める」) — this function does not know or need to know which path it
// came from.
//
// summary.Kind is a fixed, code-computed classification (success | failure |
// needs_action | aborted | unknown — see classifyDoneSummary and
// fallbackDoneSummaryKind; "unknown" only ever comes from the git-change
// fallback path). handoff.Record's own Kind field already names the record
// TYPE (session_start / done / git_turn / ...), so the classification cannot
// reuse that field; it travels instead as a "[kind] " prefix on Text. This is
// a deliberate choice, not a new field on Record: Text is already the
// allowlist's one free-form slot (親 plan 不変条件 1), the prefix is drawn
// from a fixed 5-value set this package computes (never AI or PTY text), and
// the same classification already leaves the machine today via
// notify/push (不変条件 3) for this very DoneSummary — recording it here
// widens nothing.
//
// WorkDoc (作業中の plan/bugfix md パス) is intentionally left unset: nothing
// reaching this function today carries a plan path (proto.DoneSummary has no
// such field, and threading relay's run.planPath through it is out of this
// C's file scope). A later C may fill it in once that plumbing exists.
func (s *Server) recordHandoffDone(summary proto.DoneSummary) {
	text := summary.Text
	if k := strings.TrimSpace(summary.Kind); k != "" {
		text = "[" + k + "] " + text
	}
	s.appendHandoff(summary.SessionID, handoff.Record{
		Kind: handoff.KindDone,
		Text: text,
	})
}

// recordHandoffIntent writes a kind=intent line for the DONE format's optional
// "次:" / "未検証:" lines, extracted by done_summary.go's
// extractIntentFromDoneText (子 plan: docs/local/plan_session-handoff-board_c3_intent-layer.md
// 内部 C2). Callers must only call this when extractIntentFromDoneText
// returned ok=true — this function does not itself guard against both
// arguments being empty, so an empty call would write an empty intent line
// the child plan explicitly forbids ("空の行を作らない").
//
// next/unverified are already carved out of summary.Text, which
// publishDoneSummaryInternal has already run through sessionlog.MaskSecrets
// and truncateDoneSummary before recordHandoffDone (and this function) ever
// see it — no additional masking is needed here. handoff.Sanitize still runs
// its own length cap over the combined Text inside handoff.Append.
func (s *Server) recordHandoffIntent(sessionID int, next, unverified string) {
	s.appendHandoff(sessionID, handoff.Record{
		Kind: handoff.KindIntent,
		Text: formatHandoffIntentText(next, unverified),
	})
}

// formatHandoffIntentText combines the two optional lines into Record's one
// free-form Text field (親 plan 不変条件 1 — Text is the only field an AI's
// words may travel through). Only the labels present are included, so a
// reader parsing the jsonl later can tell which of the two the AI actually
// wrote.
func formatHandoffIntentText(next, unverified string) string {
	switch {
	case next != "" && unverified != "":
		return "次: " + next + " / 未検証: " + unverified
	case next != "":
		return "次: " + next
	default:
		return "未検証: " + unverified
	}
}

// recordHandoffGitTurn writes a git_turn line for one confirmed turn. It
// looks up the repository's current HEAD commit itself (best-effort) so the
// call site in git_turns.go stays a single line; the extra `git log -1` this
// costs only runs when handoff recording is enabled.
//
// files carries changed-file paths only (gitShowFile.Path) — the caller must
// not pass anything that also carries a Diff field's contents through here.
func (s *Server) recordHandoffGitTurn(sessionID int, gitRoot string, turn gitTurnSnapshot, files []gitShowFile) {
	if !s.handoffEnabled() {
		return
	}
	paths := make([]string, 0, len(files))
	for _, f := range files {
		paths = append(paths, f.Path)
	}
	ctx, cancel := context.WithTimeout(context.Background(), gitCommandTimeout)
	defer cancel()
	commit, subject := handoffLatestCommit(ctx, gitRoot)
	s.appendHandoff(sessionID, handoff.Record{
		Kind:          handoff.KindGitTurn,
		Turn:          turn.Turn,
		FilesChanged:  turn.Files,
		Added:         turn.Added,
		Removed:       turn.Removed,
		Files:         paths,
		Commit:        commit,
		CommitSubject: subject,
	})
}

// handoffLatestCommit returns the repository's current HEAD hash and
// (masked) subject line, best-effort. A lookup failure (unborn HEAD, git
// unavailable) simply omits Commit/CommitSubject from the record; it never
// blocks a git_turn record from being written.
//
// The subject is masked the same way internal/hub/git_commit_ai.go already
// masks an AI-suggested commit subject before it leaves the Hub — a real
// commit subject is free text a person or an AI wrote, so it gets the same
// "last net" as done_summary.go's Text (親 plan 不変条件 2). handoff.Sanitize
// (inside handoff.Append) never touches CommitSubject itself, only Text, so
// this masking has to happen here.
func handoffLatestCommit(ctx context.Context, gitRoot string) (hash, subject string) {
	out, err := runGit(ctx, gitRoot, "log", "-1", "--pretty=format:%H%x09%s")
	if err != nil {
		return "", ""
	}
	parts := strings.SplitN(strings.TrimSpace(string(out)), "\t", 2)
	hash = strings.TrimSpace(parts[0])
	if len(parts) > 1 {
		subject = sessionlog.MaskSecrets(strings.TrimSpace(parts[1]))
	}
	return hash, subject
}

// --- 案 3 (turn-summary): 厚い記録のオプション設定 ------------------------
//
// 子 plan (docs/local/plan_session-handoff-board_c3_intent-layer.md) 内部 C3。
// config.HandoffConfig.IntentMode が "turn-summary" のときだけ、ターンが
// 終わるたびに現在のセッションへ短い要約プロンプトを注入し、専用マーカーで
// 拾って kind=turn_summary として看板へ書く。既定は "done-only" で、この節の
// コードは一切走らない（appendHandoff と同じ「1 つのゲートで全経路を塞ぐ」
// 形にする）。
//
// 注入とマーカー拾いの方式は internal/hub/git_commit_ai.go の「Ask AI」
// コミットメッセージ生成と同じにする（外部 API を叩かない。同じセッションへ
// プロンプトを注入し、PTY 出力に現れるマーカーを拾うだけ）。TUI 再描画への
// 耐性も extractMarkerBlock を共有することで二重実装を避ける。
const (
	handoffTurnSummaryMarkerOpen  = "[MANY-AI-CLI-TURN-SUMMARY]"
	handoffTurnSummaryMarkerClose = "[/MANY-AI-CLI-TURN-SUMMARY]"
	// 待ち受けの打ち切り時間。commitMsgAwaitTimeout より短くて良い: 求めているのは
	// 1 行要約で、コミットメッセージ生成より軽い応答のため。
	handoffTurnSummaryAwaitTimeout = 60 * time.Second
	// マーカー抽出用バッファの上限。commitMsgScanBufMax と同じ発想。
	handoffTurnSummaryScanBufMax = 16 * 1024
)

// turnSummaryEnabled is the single choke point this file reads before
// injecting a turn-summary prompt or accepting its marker. Locked
// independently from handoffEnabled (both take cfgMu internally, briefly, one
// after the other — never nested).
func (s *Server) turnSummaryEnabled() bool {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	return s.cfg.Handoff.TurnSummaryEnabled()
}

// maybeInjectHandoffTurnSummary injects the turn-summary prompt for one
// just-finished turn, unless recording is off, turn-summary mode is off, the
// session is not a connected AI session, or a previous turn's await is still
// pending (a slow or silent AI must not pile up prompts). Called from
// git_turns.go right after recordHandoffGitTurn, so this only ever fires on a
// turn boundary the machine layer already recognizes as one.
func (s *Server) maybeInjectHandoffTurnSummary(sessionID, turnNo int) {
	if !s.handoffEnabled() || !s.turnSummaryEnabled() {
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	wc := s.wrappers[sessionID]
	if ses == nil || wc == nil || !isAIProvider(ses.Provider) || ses.turnSummaryAwait {
		s.sessionsMu.Unlock()
		return
	}
	ses.turnSummaryAwait = true
	ses.turnSummaryDeadline = time.Now().Add(handoffTurnSummaryAwaitTimeout)
	ses.turnSummaryTurn = turnNo
	ses.turnSummaryBuf.Reset()
	ja := strings.EqualFold(s.cfg.UserPrefs.Display.Lang, "ja") || strings.TrimSpace(s.cfg.UserPrefs.Display.Lang) == ""
	s.sessionsMu.Unlock()

	prompt := handoffTurnSummaryPrompt(ja)
	s.submitInput(sessionID, bracketedPasteStart+prompt+bracketedPasteEnd+"\r")
}

// handoffTurnSummaryPrompt mirrors aiCommitPrompt's shape: a single line (no
// embedded newlines, which some CLIs would submit early), asking for one line
// of output wrapped in the dedicated marker pair.
func handoffTurnSummaryPrompt(ja bool) string {
	if ja {
		return "[many-ai-cli] 直前の 1 ターンで何をしたかを 1 行で要約してください。前置きや説明は一切付けず、" +
			handoffTurnSummaryMarkerOpen + " の直後に要約本文（1 行）を書き、続けて " + handoffTurnSummaryMarkerClose +
			" を出力してください（すべて 1 行に収め、装飾を付けず、コードブロックで囲まないこと）。"
	}
	return "[many-ai-cli] Summarize what you just did in this one turn, in a single line. Do not add any preamble: output " +
		handoffTurnSummaryMarkerOpen + " immediately followed by the one-line summary, then " + handoffTurnSummaryMarkerClose +
		" (keep it all on one line, undecorated, and not wrapped in a code block)."
}

// handleHandoffTurnSummaryChunk accumulates ANSI-stripped output while a
// turn-summary prompt is awaiting its marker, mirroring
// handleCommitMsgChunk's buffering/timeout shape. It is a no-op for every
// session that is not currently awaiting one (the common case — called once
// per chunk regardless).
func (s *Server) handleHandoffTurnSummaryChunk(id int, cleanText string) {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || !ses.turnSummaryAwait {
		s.sessionsMu.Unlock()
		return
	}
	if time.Now().After(ses.turnSummaryDeadline) {
		ses.turnSummaryAwait = false
		ses.turnSummaryBuf.Reset()
		s.sessionsMu.Unlock()
		return
	}
	ses.turnSummaryBuf.WriteString(cleanText)
	if ses.turnSummaryBuf.Len() > handoffTurnSummaryScanBufMax {
		trimmed := ses.turnSummaryBuf.String()
		trimmed = trimmed[len(trimmed)-handoffTurnSummaryScanBufMax:]
		ses.turnSummaryBuf.Reset()
		ses.turnSummaryBuf.WriteString(trimmed)
	}
	buf := ses.turnSummaryBuf.String()
	turnNo := ses.turnSummaryTurn
	line, ok := extractHandoffTurnSummaryLine(buf)
	if !ok {
		s.sessionsMu.Unlock()
		return
	}
	ses.turnSummaryAwait = false
	ses.turnSummaryBuf.Reset()
	s.sessionsMu.Unlock()

	if line == "" {
		return
	}
	s.appendHandoff(id, handoff.Record{
		Kind: handoff.KindTurnSummary,
		Turn: turnNo,
		Text: sessionlog.MaskSecrets(line),
	})
}

// extractHandoffTurnSummaryLine wraps extractMarkerBlock (git_commit_ai.go)
// for the turn-summary marker pair. The prompt asks for a single line, but a
// TUI may still wrap it across several rendered lines; joining every field
// with a single space collapses that back into the one line Record.Text
// expects (same treatment as truncateDoneSummary's DONE text).
func extractHandoffTurnSummaryLine(buf string) (string, bool) {
	first, rest, ok := extractMarkerBlock(buf, handoffTurnSummaryMarkerOpen, handoffTurnSummaryMarkerClose)
	if !ok {
		return "", false
	}
	combined := first
	if rest != "" {
		combined += " " + rest
	}
	return strings.Join(strings.Fields(combined), " "), true
}

// --- 引き継ぎメモ: 止まる前の前任に 1 本だけ書かせる -------------------------
//
// 子 plan (docs/local/plan_derived-session-launch_c4_handoff-routes.md) 内部 C2。
// 残量の帯から人が押す（設定 handoff.note_on_threshold が auto なら帯と同時に
// 画面が 1 回だけ叩く）。注入とマーカー拾いは上の turn-summary と同じ作りで、
// 違うのは「1 ターンの要約」ではなく「止まる前に書き残す 1 ファイル」であること。
//
// **Hub はメモの中身を読まない。** 書く先のパスを指示し、書けたという合図
// （マーカー）を拾ったらそのパスを看板へ記録するだけ（親 plan 不変条件 2）。
const (
	handoffNoteMarkerOpen  = "[MANY-AI-CLI-HANDOFF-NOTE]"
	handoffNoteMarkerClose = "[/MANY-AI-CLI-HANDOFF-NOTE]"
	// 待ち受けの打ち切り時間。turn-summary と同じ 60 秒にしてある: 求めているのは
	// 短い md 1 本で、どちらも「1 ターン分の応答を待つ」以上の意味はない。
	handoffNoteAwaitTimeout = handoffTurnSummaryAwaitTimeout
	// マーカー抽出用バッファの上限（handoffTurnSummaryScanBufMax と同じ発想）。
	handoffNoteScanBufMax = handoffTurnSummaryScanBufMax
)

// requestHandoffNote injects the memo request into one live session and starts
// waiting for its marker. It returns an error string (a short, code-computed
// reason — never PTY text) when the request cannot be made at all, so the HTTP
// handler can answer 4xx instead of silently pretending it asked.
//
// The path is decided here, not by the caller: it is always
// handoff.NotePathFor(sessionID), so a request can never point the AI at a file
// of someone else's choosing.
func (s *Server) requestHandoffNote(sessionID int) (string, string) {
	if !s.handoffEnabled() {
		return "", "handoff_disabled"
	}
	path, err := handoff.NotePathFor(sessionID)
	if err != nil {
		return "", "note_path_unavailable"
	}
	if mkErr := os.MkdirAll(filepath.Dir(path), sessionlog.PrivateDirMode); mkErr != nil {
		// 書く側（AI）がディレクトリを作れるとは限らないので、ここで用意しておく。
		// 失敗しても依頼自体は成り立つ（AI が自分で作れることもある）ので、
		// 記録だけ残して続ける。
		s.logger.Warn("handoff note dir create failed", "session_id", sessionID, "err", mkErr)
	}

	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	wc := s.wrappers[sessionID]
	if ses == nil || wc == nil || !isAIProvider(ses.Provider) {
		s.sessionsMu.Unlock()
		return "", "session_not_writable"
	}
	if ses.handoffNoteAwait {
		s.sessionsMu.Unlock()
		return "", "already_pending"
	}
	ses.handoffNoteAwait = true
	ses.handoffNoteDeadline = time.Now().Add(handoffNoteAwaitTimeout)
	ses.handoffNotePath = path
	ses.handoffNoteSeq++
	seq := ses.handoffNoteSeq
	ses.handoffNoteBuf.Reset()
	ja := strings.EqualFold(s.cfg.UserPrefs.Display.Lang, "ja") || strings.TrimSpace(s.cfg.UserPrefs.Display.Lang) == ""
	s.sessionsMu.Unlock()

	// 打ち切りは chunk 到着に頼らない: 枯渇して黙り込んだセッションからは 1 バイトも
	// 来ないことがあり、それこそがこの依頼を出す場面なので、時計で必ず終わらせる。
	time.AfterFunc(handoffNoteAwaitTimeout, func() { s.expireHandoffNote(sessionID, seq) })

	s.submitInput(sessionID, bracketedPasteStart+handoffNotePrompt(path, ja)+bracketedPasteEnd+"\r")
	return path, ""
}

// handoffNotePrompt mirrors handoffTurnSummaryPrompt's shape: one line, no
// embedded newlines (some CLIs submit on the first one), naming both the file
// to write and the marker to print when done.
func handoffNotePrompt(path string, ja bool) string {
	if ja {
		return "[many-ai-cli] 残量が少ないので引き継ぎメモを書いてください。" + path +
			" に markdown で、次の一手・未検証の前提・開いている論点・触っていた md のパスを書き、" +
			"書き終えたら " + handoffNoteMarkerOpen + " written " + handoffNoteMarkerClose +
			" を 1 行で出力してください（コードブロックで囲まない）。"
	}
	return "[many-ai-cli] This session is running low on quota; please write a handoff memo. Write markdown to " + path +
		" covering the next step, unverified assumptions, open questions and the path of any plan/bugfix md you had open, " +
		"then output " + handoffNoteMarkerOpen + " written " + handoffNoteMarkerClose +
		" on a single line (not wrapped in a code block)."
}

// handleHandoffNoteChunk accumulates ANSI-stripped output while a memo request
// is awaiting its marker (same shape as handleHandoffTurnSummaryChunk, and a
// no-op for every session that is not currently awaiting one).
func (s *Server) handleHandoffNoteChunk(id int, cleanText string) {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || !ses.handoffNoteAwait {
		s.sessionsMu.Unlock()
		return
	}
	ses.handoffNoteBuf.WriteString(cleanText)
	if ses.handoffNoteBuf.Len() > handoffNoteScanBufMax {
		trimmed := ses.handoffNoteBuf.String()
		trimmed = trimmed[len(trimmed)-handoffNoteScanBufMax:]
		ses.handoffNoteBuf.Reset()
		ses.handoffNoteBuf.WriteString(trimmed)
	}
	if _, _, ok := extractMarkerBlock(ses.handoffNoteBuf.String(), handoffNoteMarkerOpen, handoffNoteMarkerClose); !ok {
		s.sessionsMu.Unlock()
		return
	}
	path := ses.handoffNotePath
	ses.handoffNoteAwait = false
	ses.handoffNoteBuf.Reset()
	s.sessionsMu.Unlock()

	// マーカーは「書いた」という AI の申告でしかないので、ファイルがあることを
	// 自分で確かめてから記録する（中身は読まない）。
	if !isExistingFile(path) {
		s.finishHandoffNote(id, false, "")
		return
	}
	s.appendHandoff(id, handoff.Record{Kind: handoff.KindNote, Note: path})
	s.finishHandoffNote(id, true, path)
}

// expireHandoffNote ends a memo request whose marker never arrived. seq guards
// against a timer from a previous request cancelling a newer one.
func (s *Server) expireHandoffNote(id int, seq uint64) {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || !ses.handoffNoteAwait || ses.handoffNoteSeq != seq {
		s.sessionsMu.Unlock()
		return
	}
	ses.handoffNoteAwait = false
	ses.handoffNoteBuf.Reset()
	s.sessionsMu.Unlock()
	s.finishHandoffNote(id, false, "")
}

// finishHandoffNote tells the browser how the request ended. The banner that
// asked for it may be long gone (it auto-dismisses), so the UI decides where to
// show this; the Hub just says what happened, once.
func (s *Server) finishHandoffNote(id int, ok bool, path string) {
	s.broadcast(proto.Message{Type: "handoff_note", SessionID: id, NoteOK: ok, NotePath: path})
}

// --- 読み出し窓口: 引き継ぎ md とセッション一覧 -----------------------------
//
// 子 plan (docs/local/plan_session-handoff-board_c5_handoff-md.md) 内部 C1・
// C2・C3。書き込み経路（上の appendHandoff 系）と同じく、internal/handoff を
// 直接触るのはこのファイルだけにする。handoff_handler.go はここの関数だけを
// 呼び、Record を直接組み立てたり handoff.ReadAll を直接呼んだりしない。

// handoffPreview is the read-side shape handoff_handler.go returns to the
// browser: identity fields the browser needs to decide whether to preview
// (provider/cwd/branch/…), and Markdown only when a full preview (not a list
// row) was requested. Live is filled in by the caller (handoff_handler.go),
// not here — whether a session is still in s.sessions is an ordinary Hub
// concern, not part of internal/handoff's own record shape.
type handoffPreview struct {
	OK          bool   `json:"ok"`
	SessionID   int    `json:"session_id"`
	Exists      bool   `json:"exists"`
	Live        bool   `json:"live"`
	Provider    string `json:"provider,omitempty"`
	CWD         string `json:"cwd,omitempty"`
	Branch      string `json:"branch,omitempty"`
	Model       string `json:"model,omitempty"`
	StartedAt   string `json:"started_at,omitempty"`
	EndedAt     string `json:"ended_at,omitempty"`
	HandoffFrom int    `json:"handoff_from,omitempty"`
	// HandoffTo is the successor session that was launched to continue this one.
	// It is a **reverse lookup** over the directory (handoffListEntries), not a
	// recorded field: the predecessor is stopped — that is the whole premise of a
	// handoff — so nothing may be appended to its own jsonl after the fact
	// (子 plan: docs/local/plan_derived-session-launch_c3_derive-launch.md 内部 C4).
	// Filled by handoffListEntries only; a single-item preview leaves it 0.
	HandoffTo int `json:"handoff_to,omitempty"`
	// TranscriptPath / NotePath are the two "thicker than the board" routes
	// (子 plan: docs/local/plan_derived-session-launch_c4_handoff-routes.md).
	// Both are **paths only**, and both are present only when the file is
	// actually there right now: a path recorded 10 days ago whose file the user
	// has since deleted must not be offered to a successor as something to read.
	TranscriptPath string `json:"transcript_path,omitempty"`
	NotePath       string `json:"note_path,omitempty"`
	Markdown       string `json:"markdown,omitempty"`
	// CandidateProviders (handleHandoffItem のみが埋める): 起動先として選べる
	// provider の一覧。子 plan 内部 C2「同一 provider の別 subscription
	// profile は候補に出さない」を満たすため、この一覧に元セッションの
	// provider は含まれない（handoffCandidateProviders 参照）。
	CandidateProviders []string `json:"candidate_providers,omitempty"`
}

// handoffIdentityFor reads one session's identity out of its handoff jsonl,
// without rendering (or writing) the markdown preview. handoffListEntries
// uses this for every file in the directory, so it must stay cheap — no
// render.RenderMarkdown call, no disk write.
func (s *Server) handoffIdentityFor(sessionID int) (handoffPreview, error) {
	preview := handoffPreview{OK: true, SessionID: sessionID}
	records, err := handoff.ReadSession(sessionID)
	if err != nil {
		return preview, err
	}
	preview.Exists = len(records) > 0
	for _, r := range records {
		switch r.Kind {
		case handoff.KindSessionStart:
			preview.Provider = r.Provider
			preview.CWD = r.CWD
			preview.Branch = r.Branch
			preview.Model = r.Model
			preview.StartedAt = r.TS
			preview.HandoffFrom = r.HandoffFrom
		case handoff.KindSessionEnd:
			preview.EndedAt = r.TS
		}
		// Transcript / Note are read from every kind that can carry them
		// (session_start, session_end, and the dedicated transcript/note
		// lines), newest wins — same rule as render.go. isExistingFile is the
		// gate: a recorded path whose file is gone is reported as absent rather
		// than handed to a successor that would then fail to open it.
		if path := strings.TrimSpace(r.Transcript); path != "" && isExistingFile(path) {
			preview.TranscriptPath = path
		}
		if path := strings.TrimSpace(r.Note); path != "" && isExistingFile(path) {
			preview.NotePath = path
		}
	}
	return preview, nil
}

// ensureHandoffTranscriptRecorded records the transcript path of a session that
// is still live but whose board does not name it yet (子 plan 内部 C1).
//
// This is the case the whole route exists for: a predecessor that hit its quota
// and stopped responding has not ended, so recordHandoffSessionEnd has not run,
// and at session_start the provider had usually not created its log file yet.
// The single-item preview (a person opening the derive dialog for that session)
// is a human-paced action, so resolving the path here costs nothing and keeps
// the list view — which runs over every file in the directory — untouched.
//
// A session with no board at all is left alone: this must never be the thing
// that creates one (usage-probe sessions deliberately have none).
func (s *Server) ensureHandoffTranscriptRecorded(sessionID int) {
	if !s.handoffEnabled() {
		return
	}
	path := s.handoffTranscriptPathFor(sessionID)
	if path == "" {
		return
	}
	records, err := handoff.ReadSession(sessionID)
	if err != nil || len(records) == 0 {
		return
	}
	for _, r := range records {
		if r.Transcript == path {
			return
		}
	}
	s.appendHandoff(sessionID, handoff.Record{Kind: handoff.KindTranscript, Transcript: path})
}

// handoffPreviewFor is handoffIdentityFor plus the rendered markdown (子 plan
// 内部 C1). It is the single call site of handoff.WriteRendered outside of
// internal/handoff's own tests — a GET of one session's preview is exactly
// the "show it before sending it anywhere" step 親 plan 不変条件 3 requires.
func (s *Server) handoffPreviewFor(sessionID int) (handoffPreview, error) {
	s.ensureHandoffTranscriptRecorded(sessionID)
	preview, err := s.handoffIdentityFor(sessionID)
	if err != nil || !preview.Exists {
		return preview, err
	}
	_, markdown, err := handoff.WriteRendered(sessionID)
	if err != nil {
		return preview, err
	}
	preview.Markdown = markdown
	return preview, nil
}

// handoffListEntries lists every session with a handoff record on disk,
// newest (highest session ID) first. This reads handoff.Dir() directly, not
// s.sessions — that is the point of 子 plan 内部 C3's "Hub を再起動した後で
// も、看板が残っているセッションについて同じことができている": it must keep
// answering after the in-memory session map has been rebuilt from nothing.
func (s *Server) handoffListEntries() ([]handoffPreview, error) {
	dir, err := handoff.Dir()
	if err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var ids []int
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		name := e.Name()
		if !strings.HasSuffix(name, ".jsonl") {
			continue // skips the rendered *_handoff.md siblings too
		}
		idStr := strings.TrimSuffix(strings.TrimPrefix(name, "s"), ".jsonl")
		id, convErr := strconv.Atoi(idStr)
		if convErr != nil {
			continue
		}
		ids = append(ids, id)
	}
	sort.Sort(sort.Reverse(sort.IntSlice(ids)))
	out := make([]handoffPreview, 0, len(ids))
	for _, id := range ids {
		preview, identErr := s.handoffIdentityFor(id)
		if identErr != nil || !preview.Exists {
			continue
		}
		out = append(out, preview)
	}
	fillHandoffSuccessors(out)
	return out, nil
}

// fillHandoffSuccessors turns the successors' own HandoffFrom into the
// predecessors' HandoffTo, in place.
//
// The link is recorded on one side only: the successor writes "I continue #N"
// into its own session_start, and nothing is ever appended to the predecessor's
// jsonl (it is stopped, which is why a handoff happened at all). The list view
// wants the other direction — "this session was handed off to #M" — so it is
// derived here rather than written anywhere.
//
// If several successors name the same predecessor (a handoff was prepared
// twice), the newest one wins: entries arrive newest-first, so the first match
// is the latest successor and later matches are skipped.
func fillHandoffSuccessors(entries []handoffPreview) {
	successorOf := make(map[int]int, len(entries))
	for _, e := range entries {
		if e.HandoffFrom == 0 {
			continue
		}
		if _, seen := successorOf[e.HandoffFrom]; !seen {
			successorOf[e.HandoffFrom] = e.SessionID
		}
	}
	for i := range entries {
		if successor, ok := successorOf[entries[i].SessionID]; ok {
			entries[i].HandoffTo = successor
		}
	}
}

// handoffCandidateProviders returns the built-in providers a handoff
// successor may be spawned as, excluding sourceProvider. 子 plan 内部 C2:
// 「同一 provider の別 subscription profile は候補に出さない」— the
// simplest way to guarantee that is to never offer the source session's own
// provider at all (internal/hub/subscription.go's "残量を見て自動で契約を
// 切り替えない" invariant then never even becomes reachable from this UI:
// there is no picker through which a same-provider different profile could
// be chosen in the first place).
//
// The provider list itself lives in subscription.usageSources (子 plan:
// docs/local/plan_session-handoff-board_c6_usage-dispatch.md), not a literal
// slice here, so a provider added to that table becomes a handoff target
// without a second edit in this file.
func handoffCandidateProviders(sourceProvider string) []string {
	return subscription.HandoffTargetProviders(sourceProvider)
}
