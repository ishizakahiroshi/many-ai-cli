package hub

import (
	"regexp"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

const maxCrossSessionMessages = 50

// Claude currently paints a received cross-session message as a short header
// in the receiver's terminal. Keep the detector deliberately narrow: this is
// an observation path, not an authorization or message-routing path.
var crossSessionMessageHeaderRE = regexp.MustCompile(`(?i)^\s*[•●⏺]?\s*(message from|received message from)\s+(.+?)\s*$`)

type crossSessionMessageCandidate struct {
	Sender    string
	Line      string
	Signature string
}

func detectCrossSessionMessage(lines []string) *crossSessionMessageCandidate {
	var matches []string
	var sender string
	var line string
	for _, raw := range lines {
		trimmed := strings.TrimSpace(strings.TrimRight(raw, "\r"))
		if trimmed == "" {
			continue
		}
		parts := crossSessionMessageHeaderRE.FindStringSubmatch(trimmed)
		if len(parts) != 3 {
			continue
		}
		sender = strings.TrimSpace(parts[2])
		if sender == "" {
			continue
		}
		line = trimmed
		matches = append(matches, trimmed)
	}
	if len(matches) == 0 {
		return nil
	}
	return &crossSessionMessageCandidate{
		Sender:    sender,
		Line:      line,
		Signature: strings.Join(matches, "\n"),
	}
}

func copyCrossSessionMessages(messages []proto.CrossSessionMessage) []proto.CrossSessionMessage {
	if len(messages) == 0 {
		return nil
	}
	return append([]proto.CrossSessionMessage(nil), messages...)
}

// recordCrossSessionMessage attaches a receiver-side observation to the
// orchestration conductor. Bodies are intentionally omitted; the masked
// display header is enough to show that board-external communication happened
// without turning the Hub into a second transcript store.
func (s *Server) recordCrossSessionMessage(receiverID int, candidate *crossSessionMessageCandidate, at time.Time) {
	if candidate == nil {
		return
	}
	s.sessionsMu.Lock()
	receiver := s.sessions[receiverID]
	if receiver == nil || receiver.OrchestrationID == "" {
		s.sessionsMu.Unlock()
		return
	}
	conductor := receiver
	if receiver.ParentSessionID != 0 {
		conductor = s.sessions[receiver.ParentSessionID]
	}
	if conductor == nil || conductor.OrchestrationID == "" {
		s.sessionsMu.Unlock()
		return
	}
	maskedSender := sessionlog.MaskSecrets(candidate.Sender)
	maskedText := sessionlog.MaskSecrets(candidate.Line)
	if len(conductor.CrossSessionMessages) > 0 {
		last := conductor.CrossSessionMessages[len(conductor.CrossSessionMessages)-1]
		if last.ReceiverSessionID == receiver.ID && last.Sender == maskedSender && last.Text == maskedText {
			s.sessionsMu.Unlock()
			return
		}
	}
	event := proto.CrossSessionMessage{
		At:                at.Format(time.RFC3339),
		ReceiverSessionID: receiver.ID,
		ReceiverRole:      receiver.Role,
		Sender:            maskedSender,
		Text:              maskedText,
	}
	next := append(copyCrossSessionMessages(conductor.CrossSessionMessages), event)
	if len(next) > maxCrossSessionMessages {
		next = next[len(next)-maxCrossSessionMessages:]
	}
	conductor.CrossSessionMessages = next
	message := sessionUpdateMessage(conductor)
	s.sessionsMu.Unlock()
	s.broadcast(message)
}
