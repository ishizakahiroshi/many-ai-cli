package wrapper

import (
	"reflect"
	"testing"
)

func TestNativePermissionModeArgs(t *testing.T) {
	tests := []struct {
		name           string
		permissionMode string
		want           []string
	}{
		{name: "empty keeps approvals", permissionMode: "", want: nil},
		{name: "default keeps approvals", permissionMode: "default", want: nil},
		{name: "auto is passed through", permissionMode: "auto", want: []string{"--permission-mode", "auto"}},
		{name: "bypass is passed through", permissionMode: "bypassPermissions", want: []string{"--permission-mode", "bypassPermissions"}},
		{name: "plan is passed through", permissionMode: "plan", want: []string{"--permission-mode", "plan"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := nativePermissionModeArgs(tt.permissionMode); !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("nativePermissionModeArgs(%q) = %v, want %v", tt.permissionMode, got, tt.want)
			}
		})
	}
}

func TestCopilotPermissionArgs(t *testing.T) {
	tests := []struct {
		name           string
		permissionMode string
		want           []string
	}{
		{name: "default keeps approvals", permissionMode: "default", want: nil},
		{name: "auto enables autopilot", permissionMode: "auto", want: []string{"--autopilot"}},
		{name: "bypass enables allow-all", permissionMode: "bypassPermissions", want: []string{"--allow-all"}},
		{name: "plan is not a copilot permission flag", permissionMode: "plan", want: nil},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := copilotPermissionArgs(tt.permissionMode); !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("copilotPermissionArgs(%q) = %v, want %v", tt.permissionMode, got, tt.want)
			}
		})
	}
}

func TestCursorAgentPermissionArgs(t *testing.T) {
	tests := []struct {
		name           string
		permissionMode string
		want           []string
	}{
		{name: "default keeps approvals", permissionMode: "default", want: nil},
		{name: "auto enables auto-review", permissionMode: "auto", want: []string{"--auto-review"}},
		{name: "bypass enables force", permissionMode: "bypassPermissions", want: []string{"--force"}},
		{name: "plan is not a cursor permission flag", permissionMode: "plan", want: nil},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := cursorAgentPermissionArgs(tt.permissionMode); !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("cursorAgentPermissionArgs(%q) = %v, want %v", tt.permissionMode, got, tt.want)
			}
		})
	}
}
