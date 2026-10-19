package hub

import "many-ai-cli/internal/proto"

// SessionActivity is the orthogonal activity model exposed for each session.
// State remains a compatibility display label; consumers that need to decide
// whether input is safe must use IsIdle instead of inferring it from State.
type SessionActivity struct {
	OutputIdle       bool `json:"output_idle"`
	WorkflowActive   bool `json:"workflow_active"`
	AwaitingUser     bool `json:"awaiting_user"`
	AwaitingApproval bool `json:"awaiting_approval"`
}

// IsIdle is the non-interrupting-input predicate. Awaiting user input is
// deliberately not part of this predicate: a prompt can be safe to surface
// while output is quiet, but active workflow work must never be interrupted.
func (a SessionActivity) IsIdle() bool {
	return a.OutputIdle && !a.WorkflowActive
}

// DisplayState keeps the legacy one-dimensional API stable. Waiting takes
// precedence so approval/input prompts remain visible even when the workflow
// producing them is still recorded as active.
func (a SessionActivity) DisplayState() string {
	if a.AwaitingUser {
		return "waiting"
	}
	if a.WorkflowActive || !a.OutputIdle {
		return "running"
	}
	return "standby"
}

// Normalize preserves the invariant that approval is a kind of user wait.
func (a *SessionActivity) Normalize() {
	if a.AwaitingApproval {
		a.AwaitingUser = true
	}
}

func activityMessage(a SessionActivity) *proto.SessionActivity {
	return &proto.SessionActivity{OutputIdle: a.OutputIdle, WorkflowActive: a.WorkflowActive, AwaitingUser: a.AwaitingUser, AwaitingApproval: a.AwaitingApproval}
}
