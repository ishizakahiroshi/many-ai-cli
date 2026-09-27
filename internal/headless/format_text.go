package headless

// format_text.go is the generic parser: take stdout as it comes and let the
// exit code decide the outcome (親 plan D9 / 親 plan からの差分 2).
//
// It is what makes the definition table worth having. A CLI with any kind of
// print mode becomes an unattended child by writing four lines into
// config.yaml — no Go code, no release — because "I do not understand this
// provider's output" is a complete and useful answer as long as the process
// still exits with a code.

type textParser struct{}

// Parse turns one line of stdout into one bare output event. An empty line is
// kept: blank lines are part of how a CLI lays out its own output.
func (textParser) Parse(line []byte) []Event {
	return []Event{{Type: EventOutput, Text: string(line)}}
}
