package hub

import (
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

func TestChildAdmissionHumanAndAutomaticLimits(t *testing.T) {
	for _, tc := range []struct {
		kind             string
		configured, want int
	}{
		{"spawn", 0, 10}, {"spawn", 10, 10}, {"spawn", 256, 256},
		{"spawn-ui", 4, 256}, {"spawn-confirmation", 4, 256},
		{"relay-start", 10, 10}, {"relay-child", 10, 10},
		{"relay-strong", 10, 10}, {"relay-resume", 10, 10},
		{"timeout-respawn", 10, 10},
	} {
		t.Run(fmt.Sprintf("%s-%d", tc.kind, tc.configured), func(t *testing.T) {
			s := newTestServer()
			parent := registerTestSession(s, 1, "codex")
			s.cfg.Orchestration.MaxChildrenPerParent = tc.configured
			// A saved legacy total limit must allow the configured capacity.
			s.cfg.Orchestration.MaxTotalSessions = 16
			for i := 0; i < tc.want; i++ {
				if _, err := s.reserveOrchestrationChildren(parent.ID, 1, s.cfg.Orchestration, tc.kind); err != nil {
					t.Fatalf("slot %d: %v", i+1, err)
				}
			}
			_, err := s.reserveOrchestrationChildren(parent.ID, 1, s.cfg.Orchestration, tc.kind)
			var limit errOrchestrationLimit
			if !errors.As(err, &limit) || limit.Limit != "children_per_parent" || limit.Used != tc.want || limit.Max != tc.want {
				t.Fatalf("overflow = %v, want child count %d", err, tc.want)
			}
		})
	}
}

func TestChildAdmissionCountsOpenChildrenAndReservations(t *testing.T) {
	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	s.cfg.Orchestration.MaxChildrenPerParent = 10
	for i := 0; i < 9; i++ {
		child := registerTestSession(s, i+2, "codex")
		child.ParentSessionID = parent.ID
	}
	// A pending human decision still occupies a slot for automatic creation.
	pending := registerTestSpawnConfirmation(t, s, parent, spawnChildRequest{Role: "pending", Provider: "codex"})
	if _, err := s.reserveOrchestrationChildren(parent.ID, 1, s.cfg.Orchestration, "spawn"); err == nil {
		t.Fatal("automatic creation exceeded open children + reservations")
	}
	s.releaseSpawnConfirmationAdmission(pending)
	if _, err := s.reserveOrchestrationChildren(parent.ID, 1, s.cfg.Orchestration, "spawn"); err != nil {
		t.Fatalf("released slot not reusable: %v", err)
	}
}

func TestAutomaticChildGlobalLimitReportsActualCount(t *testing.T) {
	s := newTestServer()
	s.cfg.Orchestration.MaxChildrenPerParent = 2
	s.cfg.Orchestration.MaxTotalSessions = 5
	for i := 1; i <= 5; i++ {
		registerTestSession(s, i, "codex")
	}
	_, err := s.reserveOrchestrationChildren(1, 1, s.cfg.Orchestration, "spawn")
	var limit errOrchestrationLimit
	if !errors.As(err, &limit) || limit.Limit != "total_sessions" || limit.Used != 5 || limit.Max != 5 {
		t.Fatalf("global limit = %v", err)
	}
	if _, err := s.reserveOrchestrationChildren(1, 1, s.cfg.Orchestration, "spawn-ui"); err != nil {
		t.Fatalf("legacy aggregate limit blocked human request: %v", err)
	}
}

func TestOrchestrationChildLimitSettingsValidationAndPersistence(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	for _, tc := range []struct {
		value any
		want  int
	}{
		{10, 200}, {256, 200}, {1, 200}, {0, 400}, {-1, 400}, {257, 400}, {1.5, 400},
	} {
		before := s.cfg.Orchestration.MaxChildrenPerParent
		rr := httptest.NewRecorder()
		s.handleOrchestrationConfig(rr, orchestrationRequest(http.MethodPost, "/api/orchestration-config", map[string]any{
			"board_notify_mode": "soft-notify", "spawn_confirm_mode": "on",
			"spawn_confirm_providers": []string{}, "child_timeout_seconds": 900,
			"timeout_respawn": false, "max_children_per_parent": tc.value,
		}))
		if rr.Code != tc.want {
			t.Fatalf("value %v: status=%d body=%s", tc.value, rr.Code, rr.Body.String())
		}
		if tc.want == 400 {
			if s.cfg.Orchestration.MaxChildrenPerParent != before {
				t.Fatal("rejected request changed config")
			}
			continue
		}
		loaded, err := config.LoadOrCreate()
		if err != nil {
			t.Fatal(err)
		}
		if loaded.Orchestration.MaxChildrenPerParent != tc.value {
			t.Fatalf("persisted limit=%d, want %v", loaded.Orchestration.MaxChildrenPerParent, tc.value)
		}
		rr = httptest.NewRecorder()
		s.handleOrchestrationConfig(rr, orchestrationRequest(http.MethodGet, "/api/orchestration-config", nil))
		if !strings.Contains(rr.Body.String(), fmt.Sprintf(`"max_children_per_parent":%v`, tc.value)) {
			t.Fatalf("GET does not return saved limit: %s", rr.Body.String())
		}
	}
}
