package hub

import (
	"crypto/sha256"
	"encoding/hex"
	"regexp"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
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

// approvalMarkerSignature は dedupe 用シグネチャを返す。
// Grok 等の差分再描画 TUI は同一質問でも色・余白の ANSI だけが揺れるため、
// raw バイトの sha256 だと再 broadcast → Web 側 dismiss 抑止が外れる。
// ANSI 除去 + 空白正規化後の本文をハッシュし、見た目同一の質問を同一 sig に寄せる。
// Block フィールド自体は raw のまま配信し、クライアント側パース用に残す。
func approvalMarkerSignature(block string) string {
	clean := sessionlog.StripANSI(block)
	clean = strings.Join(strings.Fields(clean), " ")
	sum := sha256.Sum256([]byte(clean))
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
