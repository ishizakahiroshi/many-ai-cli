package config

import "testing"

// 子 plan 内部 C1 の完了条件: claude / codex だけが true。ほかの内蔵 provider と
// custom provider は false（打ち込みのまま）。
func TestLaunchPromptViaArgOnlyForMeasuredProviders(t *testing.T) {
	for _, provider := range []string{"claude", "codex"} {
		if !LaunchPromptViaArg(provider) {
			t.Errorf("LaunchPromptViaArg(%q) = false, want true", provider)
		}
	}
	for _, provider := range []string{"copilot", "cursor-agent", "opencode", "grok", "command-code", "shell", "gemini", "my-custom-cli", ""} {
		if LaunchPromptViaArg(provider) {
			t.Errorf("LaunchPromptViaArg(%q) = true, want false", provider)
		}
	}
}
