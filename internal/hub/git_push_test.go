package hub

import (
	"net/http"
	"slices"
	"testing"
)

func TestClassifyGitPushError(t *testing.T) {
	cases := []struct {
		name       string
		out        string
		wantCode   string
		wantStatus int
	}{
		{
			name:       "non fast forward",
			out:        "! [rejected] develop -> develop (non-fast-forward)",
			wantCode:   "rejected_non_fast_forward",
			wantStatus: http.StatusConflict,
		},
		{
			name:       "fetch first",
			out:        "Updates were rejected because the remote contains work that you do not have locally. fetch first",
			wantCode:   "rejected_non_fast_forward",
			wantStatus: http.StatusConflict,
		},
		{
			name:       "no upstream",
			out:        "fatal: The current branch feature has no upstream branch.",
			wantCode:   "no_upstream",
			wantStatus: http.StatusBadRequest,
		},
		{
			name:       "no configured destination",
			out:        "fatal: No configured push destination.",
			wantCode:   "no_upstream",
			wantStatus: http.StatusBadRequest,
		},
		{
			name:       "auth failed",
			out:        "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
			wantCode:   "auth_failed",
			wantStatus: http.StatusBadGateway,
		},
		{
			name:       "publickey",
			out:        "git@github.com: Permission denied (publickey).",
			wantCode:   "auth_failed",
			wantStatus: http.StatusBadGateway,
		},
		{
			name:       "unknown",
			out:        "fatal: remote end hung up unexpectedly",
			wantCode:   "git_command_failed",
			wantStatus: http.StatusInternalServerError,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			gotCode, gotStatus := classifyGitPushError(tc.out)
			if gotCode != tc.wantCode || gotStatus != tc.wantStatus {
				t.Fatalf("classifyGitPushError() = (%q, %d), want (%q, %d)",
					gotCode, gotStatus, tc.wantCode, tc.wantStatus)
			}
		})
	}
}

func TestGitPushNonInteractiveEnv(t *testing.T) {
	env := gitPushNonInteractiveEnv()
	if !slices.Contains(env, "GIT_TERMINAL_PROMPT=0") {
		t.Fatal("expected GIT_TERMINAL_PROMPT=0")
	}
	if !slices.Contains(env, "GIT_ASKPASS=echo") {
		t.Fatal("expected GIT_ASKPASS=echo")
	}
}
