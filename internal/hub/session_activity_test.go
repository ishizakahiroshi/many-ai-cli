package hub

import "testing"

func TestSessionActivityDisplayStateAndIdle(t *testing.T) {
	tests := []struct {
		name  string
		value SessionActivity
		state string
		idle  bool
	}{
		{"active workflow", SessionActivity{OutputIdle: true, WorkflowActive: true}, "running", false},
		{"producing output", SessionActivity{OutputIdle: false}, "running", false},
		{"awaiting input", SessionActivity{OutputIdle: true, AwaitingUser: true}, "waiting", true},
		{"approval during workflow", SessionActivity{OutputIdle: false, WorkflowActive: true, AwaitingUser: true, AwaitingApproval: true}, "waiting", false},
		{"standby", SessionActivity{OutputIdle: true}, "standby", true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := tt.value.DisplayState(); got != tt.state {
				t.Fatalf("DisplayState() = %q, want %q", got, tt.state)
			}
			if got := tt.value.IsIdle(); got != tt.idle {
				t.Fatalf("IsIdle() = %t, want %t", got, tt.idle)
			}
		})
	}
}

func TestSessionActivityNormalize(t *testing.T) {
	a := SessionActivity{AwaitingApproval: true}
	a.Normalize()
	if !a.AwaitingUser {
		t.Fatal("approval must imply awaiting user")
	}
}
