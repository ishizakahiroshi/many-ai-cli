// render.go builds the one-screen markdown handed to a successor session
// (親 plan: docs/local/plan_session-handoff-board.md 「後継は子ではなく新し
// い親」. 子 plan: docs/local/plan_session-handoff-board_c5_handoff-md.md
// 内部 C1).
//
// This file only ever reads fields already present on Record — the type's
// allowlist (handoff.go package doc, 親 plan 不変条件 1) already decided what
// can reach here. Rendering must not reach past Record into a PTY buffer, a
// file, or a diff to "fill in" a missing section; a missing section is
// rendered honestly as 記録なし, never guessed at (子 plan 内部 C1
// 「意図の記録が無い場合は、その事実を md に書く」).
package handoff

import (
	"fmt"
	"strconv"
	"strings"
)

const (
	// RenderMaxDoneEntries and RenderMaxGitTurnEntries bound each section to
	// "1 画面に収まる量" (子 plan 内部 C1 完了条件). Beyond the limit, older
	// entries are dropped in favor of a "…他 N 件" pointer, never truncated
	// mid-content.
	RenderMaxDoneEntries    = 8
	RenderMaxGitTurnEntries = 6
)

// RenderMarkdown builds the handoff document for one session from its
// already-read records (ReadAll's file order = chronological). sessionID is
// used only as the header when records is empty or carries no session_start
// line (for example a session that predates handoff recording).
func RenderMarkdown(sessionID int, records []Record) string {
	var start, end *Record
	var dones, gitTurns, intents []Record
	workDoc := ""
	transcript := ""
	note := ""
	for i := range records {
		r := records[i]
		switch r.Kind {
		case KindSessionStart:
			cp := r
			start = &cp
		case KindSessionEnd:
			cp := r
			end = &cp
		case KindDone:
			dones = append(dones, r)
		case KindGitTurn:
			gitTurns = append(gitTurns, r)
		case KindIntent:
			intents = append(intents, r)
		}
		if strings.TrimSpace(r.WorkDoc) != "" {
			workDoc = r.WorkDoc
		}
		// Transcript / Note are read from whatever record carries them (a
		// session_start, a session_end, or a later transcript/note line),
		// newest wins. Which kind recorded the path is a Hub-side detail the
		// successor has no use for.
		if strings.TrimSpace(r.Transcript) != "" {
			transcript = r.Transcript
		}
		if strings.TrimSpace(r.Note) != "" {
			note = r.Note
		}
	}
	if sessionID == 0 && start != nil {
		sessionID = start.SessionID
	}

	var b strings.Builder
	fmt.Fprintf(&b, "# 引き継ぎ: セッション #%d\n\n", sessionID)

	b.WriteString("## セッションの素性\n")
	writeIdentitySection(&b, start, end)
	b.WriteString("\n")

	// 引き継ぎメモ / 前任の会話ログ は「この md の外にある、もっと濃い記録」への
	// 案内。無いときは節ごと出さない（記録なしと書く欄ではなく、そもそも渡せる
	// ものが無いだけなので、読む側の注意をそこへ向ける必要がない）。
	if note != "" {
		b.WriteString("## 引き継ぎメモ\n")
		fmt.Fprintf(&b, "- %s\n", note)
		b.WriteString("前任が止まる前に書いた引き継ぎメモ。**本書より先にこれを読む**" +
			"（次の一手・未検証の前提・開いている論点が、この md より詳しく書かれている）。\n")
		b.WriteString("\n")
	}
	if transcript != "" {
		b.WriteString("## 前任の会話ログ\n")
		fmt.Fprintf(&b, "- %s\n", transcript)
		b.WriteString("このファイルは前任の会話ログ（JSONL）。まず末尾から、最後のユーザーの指示と" +
			"最後の assistant の発言（作業の要約・次のステップがあればそれ）を読み、次に本書の" +
			"「次の一手」と突き合わせてから作業に入る。ファイルが大きいときは末尾 200 行と " +
			"`grep` で足りる。\n")
		b.WriteString("\n")
	}

	b.WriteString("## 作業中の md\n")
	if workDoc != "" {
		fmt.Fprintf(&b, "- %s\n", workDoc)
	} else {
		b.WriteString("記録なし（この経路からはまだ分からない。開いていた plan/bugfix があれば手で伝えること）\n")
	}
	b.WriteString("\n")

	b.WriteString("## 直近の完了\n")
	writeRecentSection(&b, dones, RenderMaxDoneEntries, formatDoneLine)
	b.WriteString("\n")

	b.WriteString("## 直近の変更\n")
	writeRecentSection(&b, gitTurns, RenderMaxGitTurnEntries, formatGitTurnLine)
	b.WriteString("\n")

	b.WriteString("## 次の一手 / 未検証の前提\n")
	if len(intents) > 0 {
		latest := intents[len(intents)-1]
		fmt.Fprintf(&b, "- %s\n", latest.Text)
	} else {
		b.WriteString("記録なし。何が変わったかは分かるが、なぜそうしたかは分からない" +
			"（最後の完了報告から止まるまでの1ターン分は構造的に欠けている）。\n")
	}
	b.WriteString("\n")

	b.WriteString("## 後継への指示\n")
	b.WriteString("1. まずこの md を読み、上の「作業中の md」に記録があればそのファイルも読む。\n")
	b.WriteString("2. 「直近の変更」に挙げたコミット以降から作業を再開する" +
		"（この md にはコードの中身・diff・PTY 出力・環境変数は一切含まれていない）。\n")
	b.WriteString("3. 「次の一手 / 未検証の前提」に書かれていないことは分からない前提で動く。憶測で断定しない。\n")
	if start != nil && start.HandoffFrom != 0 {
		fmt.Fprintf(&b, "4. このセッション自身も別セッション（#%d）からの引き継ぎである。"+
			"さらに遡って読みたい場合は看板ディレクトリの s%d.jsonl を確認する。\n",
			start.HandoffFrom, start.HandoffFrom)
	}

	return b.String()
}

func writeIdentitySection(b *strings.Builder, start, end *Record) {
	if start == nil {
		b.WriteString("記録なし（session_start が看板に無い）\n")
		return
	}
	fmt.Fprintf(b, "- provider: %s\n", orNoRecord(start.Provider))
	fmt.Fprintf(b, "- cwd: %s\n", orNoRecord(start.CWD))
	fmt.Fprintf(b, "- branch: %s\n", orNoRecord(start.Branch))
	fmt.Fprintf(b, "- model: %s\n", orNoRecord(start.Model))
	fmt.Fprintf(b, "- subscription: %s\n", orNoRecord(start.SubscriptionID))
	fmt.Fprintf(b, "- 開始: %s\n", orNoRecord(start.TS))
	if start.HandoffFrom != 0 {
		fmt.Fprintf(b, "- 引き継ぎ元セッション: #%d\n", start.HandoffFrom)
	}
	if end != nil {
		fmt.Fprintf(b, "- 終了: %s (%s)\n", orNoRecord(end.TS), orNoRecord(end.Text))
	} else {
		b.WriteString("- 終了: (未終了 — 看板の最新記録時点)\n")
	}
}

func orNoRecord(s string) string {
	if strings.TrimSpace(s) == "" {
		return "(記録なし)"
	}
	return s
}

// writeRecentSection renders the newest `max` entries. records is in file /
// chronological order, so the newest are at the tail. When entries are
// dropped, a pointer replaces their content instead of truncating it (子
// plan 内部 C1: 「超える場合は中身ではなくポインタに落とす（古い kind=done
// から落とし、件数を『他 N 件』と書く）」).
func writeRecentSection(b *strings.Builder, records []Record, max int, format func(Record) string) {
	if len(records) == 0 {
		b.WriteString("記録なし\n")
		return
	}
	start := 0
	dropped := 0
	if len(records) > max {
		dropped = len(records) - max
		start = dropped
	}
	for i := len(records) - 1; i >= start; i-- {
		fmt.Fprintf(b, "- %s\n", format(records[i]))
	}
	if dropped > 0 {
		fmt.Fprintf(b, "- …他 %d 件（古いものから省略）\n", dropped)
	}
}

// formatDoneLine intentionally passes r.Text through unchanged. C2's
// recordHandoffDone may have prefixed it with "[success] " / "[failure] " /
// etc., but this package must not parse or strip that prefix (子 plan 内部
// C1: 「Text はそのまま出す ... これを解析して剥がさない」) — doing so would
// create a contract that Text is structured, when it is a masked,
// length-capped free-text field (internal/hub/handoff.go の
// recordHandoffDone 冒頭).
func formatDoneLine(r Record) string {
	ts := orNoRecord(r.TS)
	text := r.Text
	if strings.TrimSpace(text) == "" {
		text = "(本文なし)"
	}
	return fmt.Sprintf("%s: %s", ts, text)
}

func formatGitTurnLine(r Record) string {
	var parts []string
	if r.Commit != "" {
		commit := r.Commit
		if len(commit) > 12 {
			commit = commit[:12]
		}
		subject := r.CommitSubject
		if subject == "" {
			subject = "(件名なし)"
		}
		parts = append(parts, fmt.Sprintf("commit %s %q", commit, subject))
	}
	if r.Turn > 0 {
		parts = append(parts, "turn "+strconv.Itoa(r.Turn))
	}
	if len(r.Files) > 0 {
		parts = append(parts, "files: "+strings.Join(r.Files, ", "))
	}
	if len(parts) == 0 {
		return orNoRecord(r.TS)
	}
	return strings.Join(parts, " / ")
}
