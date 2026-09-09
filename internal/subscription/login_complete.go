package subscription

import "strings"

// LoginFinished reports whether vendor-CLI login output shows that sign-in
// completed. Used to close login sessions that otherwise stay in standby
// (observed: `grok login` prints "Signed in as …" and does not exit).
//
// Phrases are kept specific so "not logged in" / "not authenticated" do not
// count as success. The function never extracts an account name.
func LoginFinished(out string) bool {
	lower := strings.ToLower(out)
	for _, phrase := range loginFinishedPhrases {
		if strings.Contains(lower, phrase) {
			return true
		}
	}
	return false
}

var loginFinishedPhrases = []string{
	"signed in as",
	"successfully logged in",
	"login successful",
	"logged in using",
	"logged in as",
	"you are logged in",
}
