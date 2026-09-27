package hub

import (
	"errors"
	"net/http"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/nvidianim"
)

type nvidiaNIMSettingsStatus struct {
	Enabled       bool                `json:"enabled"`
	KeyConfigured bool                `json:"api_key_configured"`
	KeySource     nvidianim.KeySource `json:"api_key_source"`
}

func (s *Server) nvidiaNIMStatus() (nvidiaNIMSettingsStatus, error) {
	s.cfgMu.Lock()
	enabled := s.cfg.NVIDIANIM.Enabled
	s.cfgMu.Unlock()
	configDir, err := config.Dir()
	if err != nil {
		return nvidiaNIMSettingsStatus{}, err
	}
	key, source, err := nvidianim.ResolveAPIKey(configDir)
	if err != nil {
		return nvidiaNIMSettingsStatus{}, err
	}
	return nvidiaNIMSettingsStatus{
		Enabled:       enabled,
		KeyConfigured: key != "",
		KeySource:     source,
	}, nil
}

// handleNVIDIANIMSettings exposes only the enabled flag and whether a key is
// configured. A submitted key is written to the protected secret file and is
// never included in a response, log, or persisted config value.
func (s *Server) handleNVIDIANIMSettings(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPut) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		status, err := s.nvidiaNIMStatus()
		if err != nil {
			writeJSONError(w, http.StatusInternalServerError, "key_status_unavailable", "NVIDIA API key status is unavailable")
			return
		}
		writeJSON(w, status)
	case http.MethodPut:
		var body struct {
			Enabled bool   `json:"enabled"`
			APIKey  string `json:"api_key"`
		}
		if !decodeJSON(w, r, &body) {
			return
		}
		status, err := s.nvidiaNIMStatus()
		if err != nil {
			writeJSONError(w, http.StatusInternalServerError, "key_status_unavailable", "NVIDIA API key status is unavailable")
			return
		}
		if body.APIKey != "" {
			if status.KeySource == nvidianim.KeySourceEnv {
				writeJSONError(w, http.StatusConflict, "key_managed_by_environment", "NVIDIA_API_KEY is managed by the Hub environment")
				return
			}
			configDir, err := config.Dir()
			if err != nil {
				writeJSONError(w, http.StatusInternalServerError, "key_save_failed", "NVIDIA API key could not be saved")
				return
			}
			if err := nvidianim.SaveAPIKey(configDir, body.APIKey); err != nil {
				code := "key_save_failed"
				if errors.Is(err, nvidianim.ErrEmptyAPIKey) || errors.Is(err, nvidianim.ErrInvalidAPIKeyText) {
					code = "invalid_api_key"
					writeJSONError(w, http.StatusBadRequest, code, "NVIDIA API key is invalid")
					return
				}
				writeJSONError(w, http.StatusInternalServerError, code, "NVIDIA API key could not be saved")
				return
			}
		}
		s.cfgMu.Lock()
		s.cfg.NVIDIANIM.Enabled = body.Enabled
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", "NVIDIA NIM settings could not be saved")
			return
		}
		status, err = s.nvidiaNIMStatus()
		if err != nil {
			writeJSONError(w, http.StatusInternalServerError, "key_status_unavailable", "NVIDIA API key status is unavailable")
			return
		}
		writeJSON(w, status)
	}
}

func (s *Server) handleNVIDIANIMKeyDelete(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodDelete) {
		return
	}
	status, err := s.nvidiaNIMStatus()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "key_status_unavailable", "NVIDIA API key status is unavailable")
		return
	}
	if status.KeySource == nvidianim.KeySourceEnv {
		writeJSONError(w, http.StatusConflict, "key_managed_by_environment", "NVIDIA_API_KEY is managed by the Hub environment")
		return
	}
	configDir, err := config.Dir()
	if err != nil || nvidianim.DeleteAPIKey(configDir) != nil {
		writeJSONError(w, http.StatusInternalServerError, "key_delete_failed", "NVIDIA API key could not be deleted")
		return
	}
	status, err = s.nvidiaNIMStatus()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "key_status_unavailable", "NVIDIA API key status is unavailable")
		return
	}
	writeJSON(w, status)
}

func (s *Server) handleNVIDIANIMTest(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	configDir, err := config.Dir()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "key_status_unavailable", "NVIDIA API key status is unavailable")
		return
	}
	key, _, err := nvidianim.ResolveAPIKey(configDir)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "key_status_unavailable", "NVIDIA API key status is unavailable")
		return
	}
	if key == "" {
		writeJSONError(w, http.StatusBadRequest, "api_key_missing", "Configure an NVIDIA API key before testing the connection")
		return
	}
	test := s.nvidiaNIMTester
	if test == nil {
		test = nvidianim.CheckAPIKey
	}
	if err := test(key); err != nil {
		writeJSON(w, map[string]any{"ok": false, "code": nvidiaNIMTestErrorCode(err)})
		return
	}
	writeJSON(w, map[string]bool{"ok": true})
}

func nvidiaNIMTestErrorCode(err error) string {
	if errors.Is(err, nvidianim.ErrCatalogTimeout) {
		return "timeout"
	}
	var statusErr *nvidianim.CatalogHTTPError
	if errors.As(err, &statusErr) {
		switch statusErr.StatusCode {
		case http.StatusUnauthorized:
			return "unauthorized"
		case http.StatusPaymentRequired:
			return "payment_required"
		case http.StatusForbidden:
			return "forbidden"
		case http.StatusNotFound:
			return "not_found"
		case http.StatusRequestTimeout:
			return "timeout"
		case http.StatusTooManyRequests:
			return "rate_limited"
		default:
			if statusErr.StatusCode >= http.StatusInternalServerError {
				return "server_error"
			}
			return "request_rejected"
		}
	}
	return "connection_failed"
}
