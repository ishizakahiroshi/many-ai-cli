package hub

import (
	"net/http"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

// Git タブ「Ask AI」: 接続中の AI セッションへコミットメッセージ生成プロンプトを
// 注入し、PTY 出力に現れる下記マーカーを拾ってフォームへ反映する。承認検出と同じ
// 「PTY 出力をスキャンしてマーカーを抽出する」方式（方針3 / 方式1）。
//
// 精度は AI 側の出力描画に依存するため下書きレベルである点は決定論生成と同じで、
// UI 側は Review を挟んでからコミットさせる。
const (
	commitMsgMarkerOpen  = "[MANY-AI-CLI-COMMIT]"
	commitMsgMarkerClose = "[/MANY-AI-CLI-COMMIT]"
	// 待ち受けの打ち切り時間。AI が応答しない/マーカーを出さない場合の保険。
	commitMsgAwaitTimeout = 180 * time.Second
	// マーカー抽出用バッファの上限。古い方から丸める。
	commitMsgScanBufMax = 64 * 1024
)

// startAICommitMessage は AI セッションへ生成プロンプトを注入し、待ち受けを開始する。
// 結果は handleCommitMsgChunk が WS（commit_msg_suggested）でブロードキャストする。
func (s *Server) startAICommitMessage(w http.ResponseWriter, sessionID int, language string) {
	ja := strings.EqualFold(language, "ja") || language == ""

	s.sessionsMu.Lock()
	wc := s.wrappers[sessionID]
	ses := s.sessions[sessionID]
	if ses == nil {
		s.sessionsMu.Unlock()
		writeGitError(w, http.StatusNotFound, "session_not_found", "session not found")
		return
	}
	if !isAIProvider(ses.Provider) {
		s.sessionsMu.Unlock()
		writeGitError(w, http.StatusBadRequest, "not_ai_session", "session is not an AI agent")
		return
	}
	if wc == nil {
		s.sessionsMu.Unlock()
		writeGitError(w, http.StatusConflict, "no_wrapper", "AI session is not connected")
		return
	}
	ses.commitMsgAwait = true
	ses.commitMsgDeadline = time.Now().Add(commitMsgAwaitTimeout)
	ses.commitMsgLang = language
	ses.commitMsgBuf.Reset()
	s.sessionsMu.Unlock()

	prompt := aiCommitPrompt(ja)
	s.submitInput(wc, sessionID, prompt+"\r")

	writeJSON(w, gitCommitMessageResp{OK: true, Pending: true})
}

// aiCommitPrompt は AI へ注入する 1 行プロンプト。複数行を注入すると CLI 側で
// 途中送信される恐れがあるため、改行を含めず 1 行に収める（マーカー出力は AI 応答側で行う）。
func aiCommitPrompt(ja bool) string {
	if ja {
		return "[many-ai-cli] このリポジトリの未コミットの変更について、`git --no-pager diff HEAD` と `git status --short` で内容を確認し、Conventional Commits 形式（feat/fix/refactor/docs/test/chore など）のコミットメッセージを 1 つ提案してください。前置きや説明は一切付けず、" +
			commitMsgMarkerOpen + " を単独行で出力し、その次の行に subject（1 行）、必要なら空行を挟んで本文（複数行可）、最後に " + commitMsgMarkerClose +
			" を単独行で出力してください。各マーカー行は行頭・装飾なし・コードブロックで囲まないこと。"
	}
	return "[many-ai-cli] For the uncommitted changes in this repository, inspect them with `git --no-pager diff HEAD` and `git status --short`, then propose one commit message in Conventional Commits form (feat/fix/refactor/docs/test/chore, etc.). Do not add any preamble or explanation: output " +
		commitMsgMarkerOpen + " on its own line, then the subject on the next line, optionally a blank line and a body (multiple lines allowed), and finally " + commitMsgMarkerClose +
		" on its own line. Keep each marker line at the start of the line, undecorated, and not wrapped in a code block."
}

// handleCommitMsgChunk は待ち受け中セッションの ANSI 除去済み出力を蓄積し、
// マーカー対が揃ったら subject/body を抽出して WS でブロードキャストする。
// 待ち受けでないセッションでは即 return する（毎 chunk 呼ばれる軽量パス）。
func (s *Server) handleCommitMsgChunk(id int, cleanText string) {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || !ses.commitMsgAwait {
		s.sessionsMu.Unlock()
		return
	}
	if time.Now().After(ses.commitMsgDeadline) {
		lang := ses.commitMsgLang
		ses.commitMsgAwait = false
		ses.commitMsgBuf.Reset()
		s.sessionsMu.Unlock()
		s.broadcast(proto.Message{Type: "commit_msg_error", SessionID: id, Reason: commitMsgTimeoutReason(lang)})
		return
	}
	ses.commitMsgBuf.WriteString(cleanText)
	if ses.commitMsgBuf.Len() > commitMsgScanBufMax {
		trimmed := ses.commitMsgBuf.String()
		trimmed = trimmed[len(trimmed)-commitMsgScanBufMax:]
		ses.commitMsgBuf.Reset()
		ses.commitMsgBuf.WriteString(trimmed)
	}
	buf := ses.commitMsgBuf.String()
	subject, body, ok := extractCommitMarker(buf)
	if !ok {
		s.sessionsMu.Unlock()
		return
	}
	ses.commitMsgAwait = false
	ses.commitMsgBuf.Reset()
	s.sessionsMu.Unlock()

	subject = sanitizeCommitMessage(subject, gitCommitSubjectMaxLen)
	body = sanitizeCommitMessage(body, gitCommitBodyMaxLen)
	s.broadcast(proto.Message{Type: "commit_msg_suggested", SessionID: id, CommitSubject: subject, CommitBody: body})
}

// extractCommitMarker はバッファから最新のマーカー対を探して subject/body を取り出す。
//
// マーカーは「行頭・単独・装飾なし」で出力させる仕様（aiCommitPrompt）なので、
// マーカー語を含む行ではなく "マーカー語だけの行"（TUI ガター除去後）で一致させる。
// これにより、注入プロンプトのエコーがマーカー語を文中にインラインで含んでいても
// （AI の本応答が届く前の中間状態でも）プロンプト指示文を subject に取り違えない。
func extractCommitMarker(buf string) (subject, body string, ok bool) {
	rawLines := strings.Split(buf, "\n")
	lines := make([]string, len(rawLines))
	for i, ln := range rawLines {
		lines[i] = cleanTUILine(ln)
	}
	// 最後の「OPEN マーカー単独行」を基点にする。
	openIdx := -1
	for i := len(lines) - 1; i >= 0; i-- {
		if strings.TrimSpace(lines[i]) == commitMsgMarkerOpen {
			openIdx = i
			break
		}
	}
	if openIdx < 0 {
		return "", "", false
	}
	// その後ろの最初の「CLOSE マーカー単独行」までを inner とする。
	closeIdx := -1
	for i := openIdx + 1; i < len(lines); i++ {
		if strings.TrimSpace(lines[i]) == commitMsgMarkerClose {
			closeIdx = i
			break
		}
	}
	if closeIdx < 0 {
		return "", "", false
	}
	inner := lines[openIdx+1 : closeIdx]
	for len(inner) > 0 && strings.TrimSpace(inner[0]) == "" {
		inner = inner[1:]
	}
	for len(inner) > 0 && strings.TrimSpace(inner[len(inner)-1]) == "" {
		inner = inner[:len(inner)-1]
	}
	if len(inner) == 0 {
		return "", "", false
	}
	subject = strings.TrimSpace(inner[0])
	if subject == "" {
		return "", "", false
	}
	if len(inner) > 1 {
		body = strings.TrimSpace(strings.Join(inner[1:], "\n"))
	}
	return subject, body, true
}

// cleanTUILine は TUI 描画でよく行頭に付く罫線・ガター文字を取り除く。
func cleanTUILine(ln string) string {
	ln = strings.TrimRight(ln, "\r")
	return strings.TrimLeft(ln, "│┃┆┊╎▏|⏺•◦· \t")
}

func commitMsgTimeoutReason(language string) string {
	if strings.EqualFold(language, "ja") || language == "" {
		return "AI からの応答がタイムアウトしました。"
	}
	return "Timed out waiting for the AI response."
}
