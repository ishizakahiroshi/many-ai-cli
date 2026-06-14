package hub

// profiles_fetch.go — POST /api/profiles/fetch。
//
// 手元 PC から SSH-pull でリモートの `profile-export --json` を叩き、接続
// プロファイル追加フォームを自動補完するための値を返す（エージェントを使わない
// 人向けの取り込み導線。plan_server-profile-export-import.md C2）。
//
// 保存はしない: 取得した Profile（host はユーザーが実際に SSH した値・鍵は入力値で
// 補完し、既存と衝突しない名前を提案）と到達ホスト候補を返すだけで、永続化は
// 既存の POST /api/servers（replace-all）に委ねる。

import (
	"context"
	"net/http"
	"strings"
	"time"

	"many-ai-cli/internal/launcher"
)

// profileFetchTimeout bounds the SSH-pull. BatchMode=yes makes key-auth failure
// fail fast, but the TCP connect + remote shell startup still needs headroom.
const profileFetchTimeout = 25 * time.Second

type profileFetchRequest struct {
	Host         string `json:"host"`
	User         string `json:"user"`
	SSHPort      int    `json:"ssh_port"`
	IdentityFile string `json:"identity_file"`
	Binary       string `json:"binary"`
}

func (s *Server) handleProfilesFetch(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req profileFetchRequest
	if !decodeJSON(w, r, &req) {
		return
	}
	req.Host = strings.TrimSpace(req.Host)
	req.User = strings.TrimSpace(req.User)
	req.IdentityFile = strings.TrimSpace(req.IdentityFile)
	req.Binary = strings.TrimSpace(req.Binary)
	if req.Host == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "host is required")
		return
	}

	ctx, cancel := context.WithTimeout(r.Context(), profileFetchTimeout)
	defer cancel()

	exported, err := launcher.FetchRemoteProfile(ctx, launcher.FetchParams{
		Host:         req.Host,
		User:         req.User,
		SSHPort:      req.SSHPort,
		IdentityFile: req.IdentityFile,
		Binary:       req.Binary,
	})
	if err != nil {
		writeJSONError(w, http.StatusBadGateway, "fetch_failed", err.Error())
		return
	}

	// クライアント側で補完: host はユーザーが実際に到達した値を既定採用、鍵は
	// 入力値、名前は既存と衝突しないよう採番。
	pf, err := launcher.LoadProfiles()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "load_failed", err.Error())
		return
	}
	profile := exported.Profile
	profile.Name = launcher.UniqueProfileName(pf.Profiles, profile.Name)
	profile.Host = req.Host
	if req.User != "" {
		profile.User = req.User
	}
	if req.SSHPort > 0 {
		profile.SSHPort = req.SSHPort
	}
	profile.IdentityFile = req.IdentityFile

	// 補完後の値で再 Validate（injection 等を再検査してから返す）。
	if err := launcher.Validate(&launcher.ProfilesFile{Version: 1, Profiles: []launcher.Profile{profile}}); err != nil {
		writeJSONError(w, http.StatusBadRequest, "invalid_profile", err.Error())
		return
	}

	writeJSON(w, map[string]any{
		"ok":              true,
		"profile":         profile,
		"host_candidates": exported.HostCandidates,
	})
}
