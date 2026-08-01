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

// approvalMarkerSuppressNotifyInterval は破損ブロック抑止の告知を UI へ送る最小間隔。
// 破損形が 2 種類交互に現れると sig 比較だけでは毎チャンク告知になり、Web 側で
// バナーが積み上がる。時間スロットルを併用して 1 事象 1 本に抑える。
const approvalMarkerSuppressNotifyInterval = 30 * time.Second

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

	// 構造が壊れたブロックは配信しない。
	// VT ミラーの乖離で選択肢行が欠けたまま両端マーカーだけ揃うことがあり、そのまま送ると
	// Web に「選択肢が 3 から始まる承認パネル」が出る（bugfix_codex-approval-marker-vt-wrap-corruption_2026-07-31.md）。
	// ここで approvalMarkerSig を書き換えないのが要点 — 書き換えると後続の正常なブロックが
	// dedupe で潰れて承認が二度と出なくなる。
	if reason := classifyApprovalMarkerBlock(marker.Block); reason != "" {
		s.sessionsMu.Lock()
		ses := s.sessions[id]
		if ses == nil {
			s.sessionsMu.Unlock()
			return false
		}
		alreadyLogged := ses.approvalMarkerSuppressedSig == marker.Sig
		// 告知は「新しい破損 sig」かつ「前回告知から一定時間経過」のときだけ出す。
		notify := !alreadyLogged &&
			(ses.approvalMarkerSuppressedAt.IsZero() ||
				detectedAt.Sub(ses.approvalMarkerSuppressedAt) >= approvalMarkerSuppressNotifyInterval)
		ses.approvalMarkerSuppressedSig = marker.Sig
		if notify {
			ses.approvalMarkerSuppressedAt = detectedAt
		}
		provider := ses.Provider
		s.sessionsMu.Unlock()
		if !alreadyLogged && s.logger != nil {
			s.logger.Warn("approval marker suppressed: corrupt block",
				"session_id", id,
				"provider", provider,
				"reason", reason,
				"sig", shortSig(marker.Sig),
				"lines", strings.Count(marker.Block, "\n")+1)
		}
		// UI へ告知する。無音で抑止すると承認待ちのまま原因が分からない
		// （bugfix_codex-approval-marker-vt-wrap-corruption_2026-07-31.md の「未対応」節）。
		// Block 本文は壊れているので送らない（誤った選択肢を描かせないため）。
		// broadcast は必ず sessionsMu を解放した後に呼ぶ。
		if notify {
			s.broadcast(proto.Message{
				Type:           "approval_marker_suppressed",
				SessionID:      id,
				Provider:       provider,
				ApprovalSig:    marker.Sig,
				ApprovalSource: approvalSourceGoVT,
				Reason:         reason,
				DetectedAt:     detectedAt.Format(time.RFC3339),
			})
		}
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
