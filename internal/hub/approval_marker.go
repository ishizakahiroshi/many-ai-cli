package hub

import (
	"crypto/sha256"
	"encoding/hex"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

// approvalMarkerSuppressNotifyInterval は破損ブロック抑止の告知を UI へ送る最小間隔。
// 破損形が 2 種類交互に現れると sig 比較だけでは毎チャンク告知になり、Web 側で
// バナーが積み上がる。時間スロットルを併用して 1 事象 1 本に抑える。
const approvalMarkerSuppressNotifyInterval = 30 * time.Second

type approvalMarkerBlock struct {
	Block string
	Sig   string
}

// extractApprovalMarkerBlock は末尾に最も近い完結したマーカーブロックを返す。
//
// 端末は「歴史」なので、同じ質問が描き直されると前の世代が scrollback 側に残る。
// 最初の OPEN から最初の CLOSE までを取ると、古い世代の OPEN と新しい世代の CLOSE を
// またいだブロックになり、内側に OPEN を抱えたまま自分で marker_leak と判定して
// 承認パネルを握り潰していた（非貪欲マッチでも開始位置は最初の OPEN になる）。
// 最後の CLOSE と、その手前にある最後の OPEN で挟めば最新世代だけが取れる。
//
// 実測（2026-08-13 / approval-corrupt ダンプ 138 件を記録寸法へ replay）:
// marker_leak と判定されていた 74 件のうち 64 件が、この抽出だけで正常なブロックに戻る。
func extractApprovalMarkerBlock(lines []string) *approvalMarkerBlock {
	if len(lines) == 0 {
		return nil
	}
	text := strings.Join(lines, "\n")
	end := strings.LastIndex(text, approvalMarkerClose)
	if end < 0 {
		return nil
	}
	start := strings.LastIndex(text[:end], approvalMarkerOpen)
	if start < 0 {
		return nil
	}
	block := text[start : end+len(approvalMarkerClose)]
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
		var dumpSnap approvalCorruptSnapshot
		if notify {
			ses.approvalMarkerSuppressedAt = detectedAt
			// 原因調査用の生データは告知と同じスロットルで 1 事象 1 件だけ取る。
			// ptyBuf はリングバッファなのでロック内で複製する（approval_corrupt_dump.go）。
			dumpSnap = snapshotApprovalCorrupt(ses, detectedAt)
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
			// ファイル IO なので sessionsMu 解放後に行う。
			s.dumpCorruptApprovalBlock(id, provider, reason, marker, dumpSnap, detectedAt)
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
	provider := ses.Provider
	candidateIdentity := approvalMarkerCandidateIdentity(provider, marker.Block)
	candidateKey := candidateIdentity.key
	sourceEpoch, answered := approvalCandidateEpochLocked(ses, candidateKey)
	if answered || (ses.approvalMarkerCandidateKey == candidateKey &&
		ses.approvalMarkerSourceEpoch == sourceEpoch) {
		s.sessionsMu.Unlock()
		return false
	}
	ses.approvalMarkerSig = marker.Sig
	ses.approvalMarkerCandidateKey = candidateKey
	ses.approvalMarkerCandidateShape = candidateIdentity.shape
	ses.approvalMarkerSourceEpoch = sourceEpoch
	s.sessionsMu.Unlock()

	s.broadcast(proto.Message{
		Type:                   "approval_marker",
		SessionID:              id,
		Provider:               provider,
		ApprovalSig:            marker.Sig,
		ApprovalCandidateKey:   candidateKey,
		ApprovalCandidateShape: candidateIdentity.shape,
		ApprovalSourceEpoch:    sourceEpoch,
		ApprovalSource:         approvalSourceGoVT,
		Block:                  marker.Block,
		DetectedAt:             detectedAt.Format(time.RFC3339),
	})
	return true
}
