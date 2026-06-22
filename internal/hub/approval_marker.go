package hub

import (
	"crypto/sha256"
	"encoding/hex"
	"regexp"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

var approvalMarkerBlockRe = regexp.MustCompile(`(?s)\[MANY-AI-CLI\][\s\S]*?\[/MANY-AI-CLI\]`)

type approvalMarkerBlock struct {
	Block string
	Sig   string
}

func extractApprovalMarkerBlock(lines []string) *approvalMarkerBlock {
	if len(lines) == 0 {
		return nil
	}
	text := strings.Join(lines, "\n")
	matches := approvalMarkerBlockRe.FindAllString(text, -1)
	if len(matches) == 0 {
		return nil
	}
	block := matches[len(matches)-1]
	return &approvalMarkerBlock{
		Block: block,
		Sig:   approvalMarkerSignature(block),
	}
}

func approvalMarkerSignature(block string) string {
	sum := sha256.Sum256([]byte(block))
	return hex.EncodeToString(sum[:])
}

func (s *Server) maybeBroadcastApprovalMarker(id int, marker *approvalMarkerBlock, detectedAt time.Time) bool {
	if marker == nil || marker.Block == "" || marker.Sig == "" {
		return false
	}

	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return false
	}
	if ses.approvalMarkerSig == marker.Sig {
		s.sessionsMu.Unlock()
		return false
	}
	ses.approvalMarkerSig = marker.Sig
	provider := ses.Provider
	s.sessionsMu.Unlock()

	s.broadcast(proto.Message{
		Type:           "approval_marker",
		SessionID:      id,
		Provider:       provider,
		ApprovalSig:    marker.Sig,
		ApprovalSource: approvalSourceGoVT,
		Block:          marker.Block,
		DetectedAt:     detectedAt.Format(time.RFC3339),
	})
	return true
}
