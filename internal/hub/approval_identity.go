package hub

import (
	"crypto/sha256"
	"encoding/hex"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"many-ai-cli/internal/approval"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

// 承認の同一性 — このファイルがルールの正本である。
//
// 本文は 2026-08-19 まで CLAUDE.md に 23 行の節として置かれていたが、常時ロード
// されるファイルが事故のたびに太る原因になっていたため、ここへ移した。承認の
// 誤表示を直す担当は必ずこのファイルを開くので、届き方はむしろ確実になる。
// CLAUDE.md 側は「設計原則の索引」に 1 行だけ持つ。
//
// 「この承認はもう回答済みか」を決める state は candidateKey + sourceEpoch の
// 1 本だけである（ブラウザ側の正本は web/src/app/approval-answered.ts）。
//
// 以前はこの役目が 3 本に分かれていて、それぞれ「同じ質問とは何か」の定義が
// 違った。approvalConsumedSig は選択肢の署名を 5〜10 秒のタイマーで、
// answeredMarkerSigs はマーカーブロック全文のハッシュを失効なしで、
// approvalQuestionKey は質問文のハッシュを手動 dismiss の間だけ持っていた。
// 3 者の食い違いそのものが症状だった。タイマー方式は TUI の再描画が続いている
// 最中に失効して回答済みを再表示し、ブロック全文方式は逆に「エージェントが
// 本当に出し直した同じ質問」まで永久に埋めた。v0.7 で 3 本とも撤去した。
//
//   - 承認の誤表示を踏んでも、抑止をもう 1 本足さない。まず既存の 1 本で説明
//     できない症状かを確かめる。説明できるなら直すのは candidateKey の作り方か
//     sourceEpoch の進み方であって、新しい state ではない
//     （TestApprovalSuppressionStateIsSingleSource がソース走査で固定している）
//   - candidateKey は provider・承認種別・正規化した質問・選択肢番号・送信文字列
//     から作る。ラベル・空白・罫線を含めない（含めると再描画のたびに別候補になる）
//   - 例外が 1 つある。質問が自分でコマンドを名乗っていないネイティブ承認では、
//     本文から取ったコマンドを identity に含める。詳細と、そこで受け入れた代償は
//     approvalIdentityQuestionWithContext のコメントにある
//   - sourceEpoch は live prompt の境界でのみ進む。replay と reflow では進めない
//     （進めると復元しただけで新しい承認に見える）
//   - 世代が進めば同じ質問文でも新しい候補として表示するのが仕様。「同じ質問を
//     二度と出さない」方向の永久抑止に戻さない
//   - 回答済み記録のユーザーターン持ち越しは 1 回だけ（approvalConsumedCarried）。
//     答え終わったブロックは VT 末尾に何ターンも残るので、境界のたびに持ち越しを
//     再武装すると前項の永久抑止に逆戻りする

var (
	approvalIdentityOptionRe   = regexp.MustCompile(`^\s*(\d{1,2})\.\s*(.*?)\s*$`)
	approvalIdentityQuestionRe = regexp.MustCompile(`^\s*(?:Q\d{1,3}|#multi)\s*[:：]?\s*(.*?)\s*$`)
	approvalIdentityYesNoRe    = regexp.MustCompile(`[（(]\s*[YＹ]\s*[:：]\s*1\s*[／/]\s*[NＮ]\s*[:：]\s*0\s*[）)]`)
)

// normalizeApprovalIdentityText keeps the semantic text used by candidate
// identity while dropping ANSI, terminal padding, and line-wrap whitespace.
// It is intentionally stricter than the display text sent to the browser:
// status lines, borders, and labels must not make a reflow look like a new
// approval.
func normalizeApprovalIdentityText(text string) string {
	text = sessionlog.StripANSI(text)
	return strings.Join(strings.Fields(text), " ")
}

// approvalCandidateShape is the presentation-independent material used for a
// candidate identity. Option labels are not included because TUI reflow can
// overwrite or repaint them. The question, option numbers, and provider-
// specific send bytes are the stable parts that distinguish an approval
// request from a redraw of that request.
func approvalCandidateShape(provider, kind, question string, options []proto.ApprovalOption) string {
	ordered := append([]proto.ApprovalOption(nil), options...)
	sort.SliceStable(ordered, func(i, j int) bool {
		if ordered[i].Num != ordered[j].Num {
			return ordered[i].Num < ordered[j].Num
		}
		return ordered[i].SendText < ordered[j].SendText
	})

	var b strings.Builder
	b.WriteString(normalizeApprovalIdentityText(strings.ToLower(provider)))
	b.WriteByte('\n')
	b.WriteString(normalizeApprovalIdentityText(strings.ToLower(kind)))
	b.WriteByte('\n')
	b.WriteString(normalizeApprovalIdentityText(question))
	for _, option := range ordered {
		b.WriteByte('\n')
		b.WriteString(strconv.Itoa(option.Num))
		b.WriteByte(':')
		b.WriteString(normalizeApprovalIdentityText(option.SendText))
	}
	return b.String()
}

// approvalCandidateKey returns a short, non-disclosing identity for the
// presentation-independent candidate shape.
func approvalCandidateKey(provider, kind, question string, options []proto.ApprovalOption) string {
	shape := approvalCandidateShape(provider, kind, question, options)
	sum := sha256.Sum256([]byte(shape))
	return hex.EncodeToString(sum[:])[:16]
}

func approvalCandidateShapeWithContext(provider, kind, question, context string, options []proto.ApprovalOption) string {
	question = approvalIdentityQuestionWithContext(question, context, options)
	return approvalCandidateShape(provider, kind, question, options)
}

func approvalCandidateKeyWithContext(provider, kind, question, context string, options []proto.ApprovalOption) string {
	shape := approvalCandidateShapeWithContext(provider, kind, question, context, options)
	sum := sha256.Sum256([]byte(shape))
	return hex.EncodeToString(sum[:])[:16]
}

// approvalIdentityQuestionWithContext folds the semantic command from the
// prompt body into the identity question when the question line itself carries
// none. Claude and Grok render a fixed question ("Do you want to proceed?" and
// its siblings) above the actual command, so identity built from the question
// alone collides across every command in a turn and swallows the second
// approval (F-12).
//
// The trigger is deliberately not a list of known phrases: any fixed wording,
// present or future, collapses to the same key. Instead the rule is structural
// — append the subject only when the context supplies a command the question
// does not already contain, so the key can only ever get *more* specific.
//
// Accepted trade-off (user decision, 2026-08-19 — rule body is the file header
// above): the subject comes from a terminal line, so a resize that
// hard-wraps a long command changes the extracted subject and therefore the
// key, and the answered prompt can be shown again as a new candidate.
// compact() absorbs pure whitespace reflow (fixture:
// TestApprovalCandidateKeyIgnoresReflowOnlyChanges); a changed wrap position is
// not absorbed.
//
// Do NOT "fix" that re-display by dropping the subject from identity — that
// reinstates F-12, where the second approval of a turn is silently swallowed
// with no panel and no notification. Re-showing an answered prompt is the safe
// direction. If the re-display becomes a real nuisance, the thing to change is
// how the subject is read (joining wrapped lines), not whether it is in the key.
func approvalIdentityQuestionWithContext(question, context string, options []proto.ApprovalOption) string {
	if approvalOptionsHaveSendText(options) {
		return question
	}
	if _, ok := approval.CommandFromLine(question); ok {
		// The question already names the command; nothing to disambiguate.
		return question
	}
	subject := approval.Summarize(question, context).Command
	if subject == "" || normalizeApprovalIdentityText(subject) == normalizeApprovalIdentityText(question) {
		return question
	}
	return strings.TrimSpace(question) + "\n" + subject
}

// approvalMarkerCandidateKey extracts only the semantic parts of a marker.
// The full marker remains available for UI parsing, but its prose, labels,
// borders, and formatting are not identity. This is deliberately a small
// parser: classifyApprovalMarkerBlock and the browser parser remain the source
// of truth for whether a marker is actionable.
func approvalMarkerCandidateKey(provider, block string) string {
	return approvalMarkerCandidateIdentity(provider, block).key
}

func approvalMarkerCandidateIdentity(provider, block string) struct{ key, shape string } {
	clean := sessionlog.StripANSI(block)
	lines := strings.Split(clean, "\n")
	questions := make([]string, 0, 2)
	options := make([]proto.ApprovalOption, 0, 4)
	seenQuestion := false
	for _, raw := range lines {
		line := strings.TrimSpace(raw)
		if line == "" || strings.Contains(line, "[MANY-AI-CLI]") {
			continue
		}
		if strings.Contains(line, "[/MANY-AI-CLI]") {
			continue
		}
		if match := approvalIdentityQuestionRe.FindStringSubmatch(line); match != nil {
			if question := normalizeApprovalIdentityText(match[1]); question != "" {
				questions = append(questions, question)
				seenQuestion = true
			}
			continue
		}
		if match := approvalIdentityOptionRe.FindStringSubmatch(line); match != nil {
			num, err := strconv.Atoi(match[1])
			if err == nil {
				options = append(options, proto.ApprovalOption{Num: num})
			}
			continue
		}
		if approvalIdentityYesNoRe.MatchString(line) {
			question := normalizeApprovalIdentityText(approvalIdentityYesNoRe.ReplaceAllString(line, ""))
			if question != "" {
				questions = append(questions, question)
				seenQuestion = true
			}
			options = append(options, proto.ApprovalOption{Num: 1}, proto.ApprovalOption{Num: 0})
			continue
		}
		// A single-question marker is allowed without a Q1 header. Prefer a
		// question-looking line over preamble prose, while keeping the fallback
		// below for short prompts that end in a statement rather than '?'.
		if !seenQuestion && (strings.HasSuffix(line, "?") || strings.HasSuffix(line, "？")) {
			questions = append(questions, normalizeApprovalIdentityText(line))
			seenQuestion = true
		}
	}
	if len(questions) == 0 {
		for _, raw := range lines {
			line := strings.TrimSpace(raw)
			if line == "" || strings.Contains(line, "[MANY-AI-CLI]") || strings.Contains(line, "[/MANY-AI-CLI]") {
				continue
			}
			if approvalIdentityOptionRe.MatchString(line) || strings.HasPrefix(line, "N.") {
				continue
			}
			questions = append(questions, normalizeApprovalIdentityText(line))
			break
		}
	}
	shape := approvalCandidateShape(provider, "marker", strings.Join(questions, "\n"), options)
	return struct{ key, shape string }{
		key:   approvalCandidateKey(provider, "marker", strings.Join(questions, "\n"), options),
		shape: shape,
	}
}

// ensureApprovalSourceEpochLocked gives old test fixtures and sessions created
// by older code a valid first generation. Callers must hold sessionsMu.
func ensureApprovalSourceEpochLocked(ses *session) uint64 {
	if ses.approvalSourceEpoch == 0 {
		ses.approvalSourceEpoch = 1
	}
	return ses.approvalSourceEpoch
}

func ensureReplayEpochLocked(ses *session) uint64 {
	if ses.replayEpoch == 0 {
		ses.replayEpoch = 1
	}
	return ses.replayEpoch
}

// advanceApprovalSourceEpochLocked marks a live prompt boundary. Replay and
// VT reflow never call this helper. Callers must hold sessionsMu.
func advanceApprovalSourceEpochLocked(ses *session) uint64 {
	epoch := ensureApprovalSourceEpochLocked(ses)
	epoch++
	if epoch == 0 {
		epoch = 1
	}
	ses.approvalSourceEpoch = epoch
	ses.approvalEpochPending = false
	return epoch
}

// approvalCandidateEpochLocked returns whether a candidate is the prompt that
// was just consumed. A different candidate after consumption is a new prompt
// boundary and is allowed to advance the source epoch. Callers must hold the
// sessionsMu lock.
func approvalCandidateEpochLocked(ses *session, candidateKey string) (uint64, bool) {
	epoch := ensureApprovalSourceEpochLocked(ses)
	if ses.approvalEpochPending {
		if candidateKey == ses.approvalConsumedCandidateKey &&
			ses.approvalConsumedEpoch == epoch {
			return epoch, true
		}
		advanceApprovalSourceEpochLocked(ses)
		epoch = ses.approvalSourceEpoch
	}
	return epoch, false
}

// markApprovalConsumedLocked is shared by native, marker, direct-input, and
// close paths. The old signature/TTL fields remain populated for older code,
// but candidate+epoch is the durable in-session suppression state.
func markApprovalConsumedLocked(ses *session, candidateKey string, sig string) uint64 {
	return markApprovalConsumedAtEpochLocked(ses, candidateKey, sig, 0)
}

// markApprovalConsumedAtEpochLocked records a delayed answer against the
// generation that produced it. A one-generation-delayed approval_consumed
// frame can occur when the PTY input opens the next boundary before Hub has
// handled the auxiliary frame; it must not suppress an identical candidate in
// that newer generation.
func markApprovalConsumedAtEpochLocked(ses *session, candidateKey string, sig string, sourceEpoch uint64) uint64 {
	currentEpoch := ensureApprovalSourceEpochLocked(ses)
	epoch := sourceEpoch
	if epoch == 0 {
		epoch = currentEpoch
	}
	ses.approvalConsumedCandidateKey = candidateKey
	ses.approvalConsumedEpoch = epoch
	// A freshly answered prompt earns one user-turn carry (see
	// markApprovalUserTurnBoundaryLocked).
	ses.approvalConsumedCarried = false
	ses.approvalEpochPending = epoch == currentEpoch
	ses.approvalConsumedCandidateShape = approvalCandidateShapeForKeyLocked(ses, candidateKey)
	if sig != "" {
		ses.nativeApprovalConsumed = sig
		ses.nativeApprovalConsumedAt = time.Now()
	}
	return epoch
}

func approvalCandidateShapeForKeyLocked(ses *session, candidateKey string) string {
	if ses == nil || candidateKey == "" {
		return ""
	}
	if ses.nativeApprovalCandidateKey == candidateKey {
		return ses.nativeApprovalCandidateShape
	}
	if ses.approvalMarkerCandidateKey == candidateKey {
		return ses.approvalMarkerCandidateShape
	}
	if ses.approvalConsumedCandidateKey == candidateKey {
		return ses.approvalConsumedCandidateShape
	}
	return ""
}

func markApprovalUserTurnBoundaryLocked(ses *session) {
	if ses == nil {
		return
	}
	consumedKey := ses.approvalConsumedCandidateKey
	consumedShape := ses.approvalConsumedCandidateShape
	// Every confirmed non-empty user turn is a live prompt boundary, including
	// browser-only fallback sessions where Hub never observed a candidate. The
	// caller skips this helper when the approval Enter is still waiting for the
	// approval_consumed frame, so answering that prompt does not jump ahead.
	epoch := advanceApprovalSourceEpochLocked(ses)
	if consumedKey == "" || ses.vt == nil {
		return
	}
	// The terminal is a scrollback history, so the just-consumed marker can be
	// the latest complete block even after the user has started the next turn.
	// Carry the existing consumed record across exactly this new epoch. This
	// reuses candidateKey + sourceEpoch; it does not introduce another replay
	// suppression state.
	//
	// Exactly one carry per answer. The answered block stays inside the VT tail
	// for many turns, so without this bound the record would be re-armed at
	// every boundary and an agent that legitimately re-asks the same question
	// could never surface it again — the permanent suppression CLAUDE.md
	// forbids ("同じ質問を二度と出さない方向の永久抑止に戻さない").
	if ses.approvalConsumedCarried {
		return
	}
	marker := extractApprovalMarkerBlock(ses.vt.TailLinesWithScrollback(vtTailLinesForMarker))
	if marker == nil {
		return
	}
	identity := approvalMarkerCandidateIdentity(ses.Provider, marker.Block)
	if identity.key != consumedKey {
		return
	}
	ses.approvalConsumedCandidateShape = identity.shape
	if ses.approvalConsumedCandidateShape == "" {
		ses.approvalConsumedCandidateShape = consumedShape
	}
	ses.approvalConsumedEpoch = epoch
	ses.approvalConsumedCarried = true
	ses.approvalEpochPending = true
}

func approvalCandidateActiveLocked(ses *session) bool {
	return ses != nil && (ses.nativeApprovalSig != "" || ses.approvalMarkerCandidateKey != "")
}
