package wrapper

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"strings"
)

var claudeSessionFlagPrefixes = []string{
	"--session-id=",
	"--resume=",
}

// prepareClaudeSessionArgs gives a fresh Claude invocation a stable native
// transcript ID. Resume/continue modes must keep their provider-selected
// session, so they are deliberately left untouched.
func prepareClaudeSessionArgs(provider string, args []string) ([]string, string, error) {
	if provider != "claude" || claudeSessionFlagPresent(args) {
		return args, "", nil
	}
	id, err := newClaudeSessionID()
	if err != nil {
		return nil, "", fmt.Errorf("generate Claude session id: %w", err)
	}
	out := make([]string, 0, len(args)+2)
	out = append(out, "--session-id", id)
	out = append(out, args...)
	return out, id, nil
}

func claudeSessionFlagPresent(args []string) bool {
	for _, arg := range args {
		switch arg {
		case "--session-id", "--resume", "-r", "--continue", "--fork-session":
			return true
		}
		for _, prefix := range claudeSessionFlagPrefixes {
			if strings.HasPrefix(arg, prefix) {
				return true
			}
		}
	}
	return false
}

func newClaudeSessionID() (string, error) {
	var raw [16]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "", err
	}
	// RFC 4122 version 4, variant 1.
	raw[6] = (raw[6] & 0x0f) | 0x40
	raw[8] = (raw[8] & 0x3f) | 0x80
	hexValue := hex.EncodeToString(raw[:])
	return fmt.Sprintf("%s-%s-%s-%s-%s", hexValue[:8], hexValue[8:12], hexValue[12:16], hexValue[16:20], hexValue[20:]), nil
}
