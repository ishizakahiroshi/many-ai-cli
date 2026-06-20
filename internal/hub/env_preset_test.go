package hub

import "testing"

func TestEnvPresetForProxyWithConfiguredOllamaBase(t *testing.T) {
	baseURL := "http://192.168.11.50:11434"

	claude := EnvPresetForProxyWithOllamaBase("claude", RouteOllama, "", "", baseURL, "")
	if !containsEnv(claude, "ANTHROPIC_BASE_URL="+baseURL) {
		t.Fatalf("claude ollama env = %v, want configured base URL", claude)
	}

	codex := EnvPresetForProxyWithOllamaBase("codex", RouteOllama, "", "", baseURL, "")
	if !containsEnv(codex, "OPENAI_BASE_URL="+baseURL+"/v1") {
		t.Fatalf("codex ollama env = %v, want configured /v1 base URL", codex)
	}
}

func TestEnvPresetForProxyWithConfiguredLMStudioBase(t *testing.T) {
	baseURL := "http://192.168.11.50:1234"

	claude := EnvPresetForProxyWithOllamaBase("claude", RouteLMStudio, "", "", "", baseURL)
	if !containsEnv(claude, "ANTHROPIC_BASE_URL="+baseURL) {
		t.Fatalf("claude lm-studio env = %v, want configured base URL", claude)
	}
	if !containsEnv(claude, "ANTHROPIC_AUTH_TOKEN=lmstudio") {
		t.Fatalf("claude lm-studio env = %v, want ANTHROPIC_AUTH_TOKEN=lmstudio", claude)
	}

	codex := EnvPresetForProxyWithOllamaBase("codex", RouteLMStudio, "", "", "", baseURL)
	if !containsEnv(codex, "OPENAI_BASE_URL="+baseURL+"/v1") {
		t.Fatalf("codex lm-studio env = %v, want configured /v1 base URL", codex)
	}
	if !containsEnv(codex, "OPENAI_API_KEY=lmstudio") {
		t.Fatalf("codex lm-studio env = %v, want OPENAI_API_KEY=lmstudio", codex)
	}
}

func containsEnv(env []string, want string) bool {
	for _, got := range env {
		if got == want {
			return true
		}
	}
	return false
}
