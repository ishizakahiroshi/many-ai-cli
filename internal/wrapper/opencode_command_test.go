package wrapper

import (
	"reflect"
	"testing"
)

func TestOpenCodePermissionArgs(t *testing.T) {
	tests := []struct {
		name           string
		permissionMode string
		want           []string
	}{
		{name: "default keeps approvals", permissionMode: "default", want: nil},
		{name: "bypass enables auto approval", permissionMode: "bypassPermissions", want: []string{"--auto"}},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := openCodePermissionArgs(tt.permissionMode); !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("openCodePermissionArgs(%q) = %v, want %v", tt.permissionMode, got, tt.want)
			}
		})
	}
}
