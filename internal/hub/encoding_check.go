package hub

import (
	"net/http"
	"strings"
)

type encodingCheckResult struct {
	IsWindows      bool   `json:"is_windows"`
	IsPowerShell   bool   `json:"is_powershell"`
	InputCodepage  uint32 `json:"input_codepage"`
	OutputCodepage uint32 `json:"output_codepage"`
	IsUTF8         bool   `json:"is_utf8"`
}

func isPowerShellShell(shell string) bool {
	low := strings.ToLower(shell)
	return strings.Contains(low, "powershell")
}

func (s *Server) handleEncodingCheck(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	result := checkEncoding(s.parentShell)
	writeJSON(w, result)
}
