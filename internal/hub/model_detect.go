package hub

// model_detect.go: server.go から分離した「PTY 出力からのモデル検出」の関数群。
//
// C4 追加分割 (plan_audit_score_s_promotion_2026-07-05.md): server.go の関心事別
// 分割の第四弾。以下 4 関数は「/model 変更検出」「起動バナーからの初期モデル
// 検出」「検出値のセッション反映」を扱う一塊で、他の関心事から明確に分離できる。
// 挙動は移動前と完全に同一・全て package-private・呼び出し元は変更なし。

import (
	"strings"

	"many-ai-cli/internal/proto"
)

func (s *Server) detectModelChange(id int, data []byte, cleanText string) {
	if !ptyChunkContainsAny(data, modelChangeTokens) {
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	provider := ses.Provider
	s.sessionsMu.Unlock()

	var match []string
	switch provider {
	case "claude":
		match = reSetModelTo.FindStringSubmatch(cleanText)
	case "codex":
		match = reCodexModelChanged.FindStringSubmatch(cleanText)
	default:
		return
	}
	if match == nil {
		return
	}
	newModel := strings.TrimSpace(match[1])
	if newModel == "" {
		return
	}
	s.applyDetectedModel(id, provider, newModel, false)
}

// detectInitialModel は VT バッファのレンダリング済み行から起動バナーの
// モデル名を抽出し、Model が空のセッションに反映する。
// /model 変更（detectModelChange）と違い既存値は上書きしない。
func (s *Server) detectInitialModel(id int, provider, cwd string, vtLines []string) {
	model := extractBannerModel(provider, cwd, vtLines)
	if model == "" {
		return
	}
	s.applyDetectedModel(id, provider, model, true)
}

// extractBannerModel は起動バナー / ステータス行からモデル名を抽出する
// （見つからなければ ""）。cwd は cursor-agent のステータス行アンカーに使う。
func extractBannerModel(provider, cwd string, lines []string) string {
	switch provider {
	case "claude":
		for _, line := range lines {
			idx := strings.Index(line, claudeBannerLogoRow2)
			if idx < 0 {
				continue
			}
			rest := strings.TrimSpace(line[idx+len(claudeBannerLogoRow2):])
			// " · Claude Max" 等のプラン表記を落とす
			if before, _, found := strings.Cut(rest, "·"); found {
				rest = strings.TrimSpace(before)
			}
			rest = reClaudeBannerEffort.ReplaceAllString(rest, "")
			if rest != "" {
				return rest
			}
		}
	case "codex":
		for _, line := range lines {
			m := reCodexBannerModel.FindStringSubmatch(line)
			if m == nil {
				continue
			}
			model := strings.TrimSpace(m[1])
			if model != "" && !strings.EqualFold(model, "loading") {
				return model
			}
		}
	case "copilot":
		// 最下部の非空行（ステータス行）の右端セグメントを候補にする。
		// ラベルが無いため、モデル名らしさの検査で誤検出を防ぐ。
		for i := len(lines) - 1; i >= 0; i-- {
			line := strings.TrimSpace(lines[i])
			if line == "" {
				continue
			}
			segs := reCopilotStatusSplit.Split(line, -1)
			seg := strings.TrimSpace(segs[len(segs)-1])
			seg = reCopilotEffortSuffix.ReplaceAllString(seg, "")
			if seg == "Auto" || (len(seg) <= 40 && reCopilotModelLike.MatchString(seg)) {
				return seg
			}
			return "" // 最下部の非空行のみ見る（それより上はステータス行ではない）
		}
	case "cursor-agent":
		if cwd == "" {
			return ""
		}
		// "<cwd> · <branch>" 行を探し、その直上の非空行をモデル名とみなす。
		for i, line := range lines {
			t := strings.TrimSpace(line)
			if !strings.HasPrefix(t, cwd+" · ") && t != cwd {
				continue
			}
			for j := i - 1; j >= 0; j-- {
				above := strings.TrimSpace(lines[j])
				if above == "" {
					continue
				}
				above = reCursorPercentSuffix.ReplaceAllString(above, "")
				// プロンプト残骸（"→ ..." 等）は除外
				if above != "" && !strings.ContainsAny(above, "→❯") && len(above) <= 60 {
					return above
				}
				break
			}
		}
	}
	return ""
}

// applyDetectedModel はセッションの Model / Route を更新して session_update を
// broadcast する。onlyIfEmpty=true のときは Model 未設定のセッションのみ更新する
// （起動バナー検出が /model 変更や --model 指定を上書きしないため）。
func (s *Server) applyDetectedModel(id int, provider, newModel string, onlyIfEmpty bool) {
	newRoute := s.resolveRoute(provider, newModel)
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses.Model == newModel || (onlyIfEmpty && ses.Model != "") {
		s.sessionsMu.Unlock()
		return
	}
	ses.Model = newModel
	ses.Route = newRoute
	ses.initialModelScanDone = true
	update := proto.Message{
		Type:         "session_update",
		SessionID:    id,
		Provider:     ses.Provider,
		Display:      ses.Display,
		CWD:          ses.CWD,
		Branch:       ses.Branch,
		Label:        ses.Label,
		Model:        ses.Model,
		Route:        ses.Route,
		State:        ses.State,
		LastOutputAt: ses.LastOutputAt,
		FirstMessage: ses.FirstMessage,
		LastMessage:  ses.LastMessage,
	}
	s.sessionsMu.Unlock()
	s.broadcast(update)
}
