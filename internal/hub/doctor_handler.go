package hub

import (
	"context"
	"net/http"

	"many-ai-cli/internal/doctor"
)

// handleDoctor returns the same non-mutating checks exposed by `many-ai-cli
// doctor`. Authentication is supplied by the server's common API middleware.
func (s *Server) handleDoctor(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	writeJSON(w, doctor.Run(context.Background(), s.snapshotCfg()))
}
