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

// openless 再構成の適用条件。
//
//	openlessCloseBottomWindow: 終了マーカーが画面下端からこの行数以内にあること。
//	  CLI のチローム（区切り線・入力欄・ステータス行）の高さぶんの余裕。ここを超える
//	  位置に終了マーカーがあるなら、その下に別の出力が続いている＝「今出た質問の末尾」
//	  ではないので再構成しない。
//	openlessMinBlockLines: 再構成した本文の最小行数。画面が低い端末で、数行の断片から
//	  承認バーを組み立ててしまうのを防ぐ。
const (
	openlessCloseBottomWindow = 16
	openlessMinBlockLines     = 8
)

// extractApprovalMarkerBlockFromVT は VT ミラーから承認マーカーブロックを取り出す。
// 本番の検出経路はすべてこちらを通す（extractApprovalMarkerBlock は素の行列に対する
// 純粋な抽出で、テストと下記フォールバックの土台）。
func extractApprovalMarkerBlockFromVT(vt *vtBuffer) *approvalMarkerBlock {
	if vt == nil {
		return nil
	}
	lines := vt.TailLinesWithScrollback(vtTailLinesForMarker)
	if marker := extractApprovalMarkerBlock(lines); marker != nil {
		return marker
	}
	return reconstructOpenlessMarkerBlock(lines, vt.Rows())
}

// reconstructOpenlessMarkerBlock は「終了マーカーはあるが開始マーカーがどこにも無い」
// ブロックを、画面に残っている範囲から組み立て直す。
//
// なぜ必要か（2026-08-26 セッション #5 で実測）: Claude Code は代替画面バッファを Ink が
// 絶対座標で塗り直す。画面高に収まらない長さの回答では、あふれた行はスクロールアウト
// ではなく上書きで消えるため、vtBuffer の scrollback（newLine() で押し出された行だけを
// 積む）には 1 行も入らない。実測は 128x35 の端末で本文 26 行の承認ブロックが出た場面で、
// scrollback 0 行・開始マーカーは画面外・終了マーカーだけが画面に残る状態だった。
// extractApprovalMarkerBlock は開始マーカーが見つからず nil を返し、承認は無音で
// 検出されないまま standby の完了マーカー未検出フォールバック（done_summary.go）へ落ちる。
//
// 安全に再構成できる理由: ブロックが画面高を超えているなら、画面の先頭から終了マーカーまでは
// 定義上すべてブロックの本文である（本文の末尾が終了マーカーで、それが画面の下端付近にある）。
// そこで下の 3 条件をすべて満たすときだけ、画面先頭〜終了マーカーを本文とみなす。
//
//  1. 終了マーカーが「今の画面」にあり、下端から openlessCloseBottomWindow 行以内
//  2. 終了マーカーより後ろに開始マーカーが無い（＝次のブロックを描き始めていない）
//  3. 本文が openlessMinBlockLines 行以上
//
// 欠落した前置きは戻らないが、選択肢が欠けていれば classifyApprovalMarkerBlock が
// option_start で弾く（承認バーを黙って出さないのではなく、抑止の告知が UI へ出る）。
func reconstructOpenlessMarkerBlock(lines []string, screenRows int) *approvalMarkerBlock {
	if screenRows <= 0 || screenRows > len(lines) {
		return nil
	}
	screenStart := len(lines) - screenRows

	closeIdx := -1
	for i := len(lines) - 1; i >= screenStart; i-- {
		if strings.Contains(lines[i], approvalMarkerClose) {
			closeIdx = i
			break
		}
	}
	if closeIdx < 0 || len(lines)-1-closeIdx > openlessCloseBottomWindow {
		return nil
	}
	for i := closeIdx + 1; i < len(lines); i++ {
		if strings.Contains(lines[i], approvalMarkerOpen) {
			return nil
		}
	}

	body := append([]string(nil), lines[screenStart:closeIdx+1]...)
	// 終了マーカーと同じ行に後続の描画が混ざっている場合は行内で切る。
	last := body[len(body)-1]
	if i := strings.Index(last, approvalMarkerClose); i >= 0 {
		body[len(body)-1] = last[:i+len(approvalMarkerClose)]
	}
	for len(body) > 0 && strings.TrimSpace(body[0]) == "" {
		body = body[1:]
	}
	if len(body) < openlessMinBlockLines {
		return nil
	}

	block := approvalMarkerOpen + "\n" + strings.Join(body, "\n")
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
