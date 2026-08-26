package hub

import (
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"unicode"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionstore"
)

// sessionMetaPatch uses pointers so PATCH can distinguish an omitted property
// from an explicit reset to an empty value (for example, clearing a note).
type sessionMetaPatch struct {
	Label  *string `json:"label"`
	Pinned *bool   `json:"pinned"`
	Color  *string `json:"color"`
	Note   *string `json:"note"`
}

var sessionCardColors = map[string]struct{}{
	"": {}, "blue": {}, "green": {}, "orange": {}, "red": {}, "purple": {},
}

const autoTitleMaxRunes = 40

var (
	autoTitleCodeFencePrefix = regexp.MustCompile("^```(?:[A-Za-z0-9_+#.-]+)?\\s*")
	autoTitleWindowsPath     = regexp.MustCompile(`^[A-Za-z]:[\\/][^\s]+`)
	autoTitlePOSIXPath       = regexp.MustCompile(`^/(?:[^/\s]+/)+[^/\s]*`)
	autoTitleURL             = regexp.MustCompile(`(?i)^https?://[^\s]+`)
	autoTitleAttachment      = regexp.MustCompile(`^@[^\s]+`)
)

func normalizeSessionMetaText(value string, maxRunes int) string {
	value = strings.Map(func(r rune) rune {
		if unicode.IsControl(r) {
			return -1
		}
		return r
	}, value)
	value = strings.Join(strings.Fields(value), " ")
	if maxRunes <= 0 {
		return value
	}
	runes := []rune(value)
	if len(runes) > maxRunes {
		return string(runes[:maxRunes])
	}
	return value
}

// autoTitleFromInput keeps the first useful part of a confirmed user request.
// Spawned sessions often receive a path, URL, attachment reference, or code
// fence before the actual request. Those prefixes consume the entire 40-rune
// title budget without helping identify the work. If stripping prefixes would
// leave no request text, retain the legacy full-prefix fallback instead.
func autoTitleFromInput(value string) string {
	normalized := normalizeSessionMetaText(value, 0)
	if normalized == "" {
		return ""
	}

	candidate := normalized
	for {
		candidate = strings.TrimSpace(candidate)
		switch {
		case autoTitleCodeFencePrefix.MatchString(candidate):
			candidate = autoTitleCodeFencePrefix.ReplaceAllString(candidate, "")
		case autoTitleWindowsPath.MatchString(candidate):
			candidate = autoTitlePathRemainder(candidate, autoTitleWindowsPath)
		case autoTitlePOSIXPath.MatchString(candidate):
			candidate = autoTitlePathRemainder(candidate, autoTitlePOSIXPath)
		case autoTitleURL.MatchString(candidate):
			candidate = autoTitleURL.ReplaceAllString(candidate, "")
		case autoTitleAttachment.MatchString(candidate):
			candidate = autoTitleAttachment.ReplaceAllString(candidate, "")
		default:
			candidate = strings.TrimSpace(candidate)
			if strings.HasSuffix(candidate, "```") {
				candidate = strings.TrimSpace(strings.TrimSuffix(candidate, "```"))
			}
			if candidate == "" {
				return normalizeSessionMetaText(normalized, autoTitleMaxRunes)
			}
			return normalizeSessionMetaText(candidate, autoTitleMaxRunes)
		}
	}
}

// autoTitlePathRemainder drops directory components but keeps the final path
// component when there is a request after the path. Keeping the file name is
// useful context (for example, "plan_foo.md を実装して"). A path-only input
// returns an empty remainder so autoTitleFromInput can use its fail-safe.
func autoTitlePathRemainder(value string, pattern *regexp.Regexp) string {
	pathPart := pattern.FindString(value)
	if pathPart == "" {
		return value
	}
	remainder := strings.TrimSpace(value[len(pathPart):])
	if remainder == "" {
		return ""
	}
	base := pathPart[strings.LastIndexAny(pathPart, `/\\`)+1:]
	if base == "" {
		return remainder
	}
	return base + " " + remainder
}

func sessionMetaFor(ses *session) *proto.SessionMeta {
	return &proto.SessionMeta{
		Label:     ses.Label,
		Pinned:    ses.Pinned,
		Color:     ses.Color,
		Note:      ses.Note,
		AutoTitle: ses.AutoTitle,
	}
}

func sessionStoreMeta(ses *session) sessionstore.SessionCardMeta {
	return sessionstore.SessionCardMeta{
		Label:     ses.Label,
		Pinned:    ses.Pinned,
		Color:     ses.Color,
		Note:      ses.Note,
		AutoTitle: ses.AutoTitle,
	}
}

func (s *Server) handleSessionMetaAPI(w http.ResponseWriter, r *http.Request) {
	// Authenticate the route before parsing its dynamic path. Apart from keeping
	// malformed paths private, this makes the registered /api/session/ prefix
	// follow the same token contract as every other API route.
	if !s.requireToken(w, r) {
		return
	}
	parts := strings.Split(strings.Trim(strings.TrimPrefix(r.URL.Path, "/api/session/"), "/"), "/")
	if len(parts) != 2 || parts[1] != "meta" {
		writeJSONError(w, http.StatusNotFound, "not_found", "not found")
		return
	}
	id, err := strconv.Atoi(parts[0])
	if err != nil || id <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid session id")
		return
	}
	s.handleSessionMeta(w, r, id)
}

// handleSessionMeta updates only the supplied presentation fields.
func (s *Server) handleSessionMeta(w http.ResponseWriter, r *http.Request, id int) {
	if !s.guard(w, r, http.MethodPatch) {
		return
	}
	var patch sessionMetaPatch
	if !decodeJSON(w, r, &patch) {
		return
	}
	if patch.Label == nil && patch.Pinned == nil && patch.Color == nil && patch.Note == nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "at least one meta field is required")
		return
	}

	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		writeJSONError(w, http.StatusNotFound, "not_found", "session not found")
		return
	}
	if patch.Label != nil {
		ses.Label = normalizeSessionMetaText(*patch.Label, 120)
	}
	if patch.Pinned != nil {
		ses.Pinned = *patch.Pinned
	}
	if patch.Color != nil {
		color := strings.ToLower(strings.TrimSpace(*patch.Color))
		if _, ok := sessionCardColors[color]; !ok {
			s.sessionsMu.Unlock()
			writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid session color")
			return
		}
		ses.Color = color
	}
	if patch.Note != nil {
		ses.Note = normalizeSessionMetaText(*patch.Note, 160)
	}
	meta := sessionMetaFor(ses)
	persisted := sessionStoreMeta(ses)
	provider, display, cwd, branch, state := ses.Provider, ses.Display, ses.CWD, ses.Branch, ses.State
	s.sessionsMu.Unlock()

	if s.sessionStore != nil {
		if err := s.sessionStore.UpdateSessionCardMeta(id, persisted); err != nil {
			s.logger.Warn("session card meta persist failed", "session_id", id, "err", err)
			writeJSONError(w, http.StatusInternalServerError, "session_meta_failed", "failed to save session metadata")
			return
		}
	}
	msg := proto.Message{Type: "session_update", SessionID: id, Provider: provider, Display: display, CWD: cwd, Branch: branch, State: state, SessionMeta: meta}
	s.broadcast(msg)
	writeJSON(w, map[string]any{"ok": true, "session_id": id, "session_meta": meta})
}
