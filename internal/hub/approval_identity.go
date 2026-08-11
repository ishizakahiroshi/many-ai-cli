package hub

import (
	"crypto/sha256"
	"encoding/hex"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

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
	// Every confirmed non-empty user turn is a live prompt boundary, including
	// browser-only fallback sessions where Hub never observed a candidate. The
	// caller skips this helper when the approval Enter is still waiting for the
	// approval_consumed frame, so answering that prompt does not jump ahead.
	advanceApprovalSourceEpochLocked(ses)
}

func approvalCandidateActiveLocked(ses *session) bool {
	return ses != nil && (ses.nativeApprovalSig != "" || ses.approvalMarkerCandidateKey != "")
}
