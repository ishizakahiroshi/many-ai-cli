package wrapper

import (
	"reflect"
	"testing"
)

func TestCopilotViaGhArgs(t *testing.T) {
	cases := []struct {
		name string
		args []string
		want []string
	}{
		{name: "no args", args: nil, want: []string{"copilot"}},
		{name: "passes through after delimiter", args: []string{"--model", "gpt-5"}, want: []string{"copilot", "--", "--model", "gpt-5"}},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := copilotViaGhArgs(tc.args); !reflect.DeepEqual(got, tc.want) {
				t.Fatalf("copilotViaGhArgs(%v) = %v, want %v", tc.args, got, tc.want)
			}
		})
	}
}
