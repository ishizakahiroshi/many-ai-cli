package subscription

import "testing"

func TestLoginFinishedRecognizesVendorSuccess(t *testing.T) {
	cases := []struct {
		name string
		out  string
		want bool
	}{
		{"grok signed in", "Signed in as someone@example.com\n", true},
		{"codex chatgpt", "Logged in using ChatGPT\n", true},
		{"codex success", "Successfully logged in.\n", true},
		{"claude as", "Logged in as someone@example.com\n", true},
		{"login successful", "Login successful.\n", true},
		{"already logged", "You are logged in with grok.com.\n", true},
		{"not logged in", "You are not logged in.\n", false},
		{"not authenticated", "You are not authenticated.\n", false},
		{"not signed in", "You are not signed in.\n", false},
		{"empty", "", false},
		{"unrelated", "Open this URL to sign in:\n", false},
	}
	for _, tc := range cases {
		if got := LoginFinished(tc.out); got != tc.want {
			t.Errorf("%s: LoginFinished(%q) = %v, want %v", tc.name, tc.out, got, tc.want)
		}
	}
}
