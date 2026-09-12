package headless

import (
	"strconv"
	"strings"
)

// prompt.go carries the one thing a headless launch cannot know when its prompt
// is written: the child's own session id.
//
// An interactive child is told its id because the prompt is injected *after*
// the session registered. A headless child gets its prompt at startup, which is
// before it has an id at all — the Hub assigns one when the wrapper registers.
// Rather than invent a second way to deliver the prompt (and a race between
// writing the file and reading it), the Hub writes a placeholder where the id
// belongs and the wrapper expands it once, after register, before handing the
// prompt to the provider.
//
// The alternative — telling the child "your id is in an environment variable" —
// puts the work on the model, which is exactly the kind of instruction that is
// followed four times out of five.

// SessionIDPlaceholder is the token the Hub writes wherever a headless prompt
// needs the child's session id: the id line and the child's own progress file
// name. It is deliberately unlikely to occur in a person's prompt.
const SessionIDPlaceholder = "{{many-ai-cli:session-id}}"

// ExpandPrompt replaces every placeholder with the session id the Hub assigned.
// A prompt with no placeholder is returned unchanged, so a hand-written
// `--prompt-file` is never rewritten behind the author's back.
func ExpandPrompt(prompt string, sessionID int) string {
	if !strings.Contains(prompt, SessionIDPlaceholder) {
		return prompt
	}
	return strings.ReplaceAll(prompt, SessionIDPlaceholder, strconv.Itoa(sessionID))
}
