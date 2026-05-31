package wrapper

func copilotViaGhArgs(args []string) []string {
	out := []string{"copilot"}
	if len(args) > 0 {
		out = append(out, "--")
		out = append(out, args...)
	}
	return out
}
