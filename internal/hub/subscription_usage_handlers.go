package hub

import (
	"net/http"
)

// handleSubscriptionUsage returns only profile names and provider-reported
// usage metadata. Profile directories and authentication material never cross
// this boundary.
func (s *Server) handleSubscriptionUsage(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	store := s.refreshSubscriptionUsage()
	writeJSON(w, store.snapshot(s.snapshotCfg()))
}
