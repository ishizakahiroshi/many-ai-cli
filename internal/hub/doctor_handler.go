package hub

import (
	"net/http"

	"many-ai-cli/internal/doctor"
)

// handleDoctor returns the same non-mutating checks exposed by `many-ai-cli
// doctor`. Authentication is supplied by the server's common API middleware.
func (s *Server) handleDoctor(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	// クライアント切断時に外部プロセス probe を中断できるよう、
	// context.Background() ではなく r.Context() を伝播する。
	writeJSON(w, doctor.Run(r.Context(), s.snapshotCfg()))
}
