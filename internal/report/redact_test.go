package report

import (
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

func TestRedactKnownSecretPatterns(t *testing.T) {
	tests := []struct {
		name   string
		input  string
		secret string
	}{
		{"query token", "http://127.0.0.1:47777/?token=abcdef0123456789", "abcdef0123456789"},
		{"access token query", "https://example.test/cb?access_token=syntheticAccess123&ok=1", "syntheticAccess123"},
		{"bearer", "Bearer syntheticBearerValue123", "syntheticBearerValue123"},
		{"authorization bearer", "Authorization: Bearer syntheticAuthValue123", "syntheticAuthValue123"},
		{"OpenAI key", "sk-syntheticOpenAIKey123456", "sk-syntheticOpenAIKey123456"},
		{"Anthropic key", "sk-ant-api03-syntheticAnthropic123", "sk-ant-api03-syntheticAnthropic123"},
		{"GitHub classic PAT", "ghp_SyntheticClassicPAT123456", "ghp_SyntheticClassicPAT123456"},
		{"GitHub fine-grained PAT", "github_pat_SyntheticFineGrained123456", "github_pat_SyntheticFineGrained123456"},
		{"GitHub OAuth token", "gho_SyntheticOAuthToken123456", "gho_SyntheticOAuthToken123456"},
		{"Slack bot token", "xoxb-synthetic-slack-token-123456", "xoxb-synthetic-slack-token-123456"},
		{"AWS long-term key", "AKIASYNTHETIC1234567", "AKIASYNTHETIC1234567"},
		{"AWS temporary key", "ASIASYNTHETIC1234567", "ASIASYNTHETIC1234567"},
		{"Google API key", "AIzaSyntheticGoogleKey123456", "AIzaSyntheticGoogleKey123456"},
		{"GitLab PAT", "glpat-syntheticGitLabToken123", "glpat-syntheticGitLabToken123"},
		{"Hugging Face token", "hf_SyntheticHuggingFace12345", "hf_SyntheticHuggingFace12345"},
		{"npm token", "npm_SyntheticRegistryToken123", "npm_SyntheticRegistryToken123"},
		{"PyPI token", "pypi-SyntheticRegistryToken123", "pypi-SyntheticRegistryToken123"},
		{"xAI token", "xai-SyntheticXAIToken123456", "xai-SyntheticXAIToken123456"},
		{"Groq token", "gsk_SyntheticGroqToken123456", "gsk_SyntheticGroqToken123456"},
		{"JWT", "eyJhbGciOiJIUzI1NiJ9.c3ludGhldGljLXBheWxvYWQ.c3ludGhldGljLXNpZw", "eyJhbGciOiJIUzI1NiJ9.c3ludGhldGljLXBheWxvYWQ.c3ludGhldGljLXNpZw"},
		{"OpenAI config field", "openai_api_key: syntheticOpenAIConfig123", "syntheticOpenAIConfig123"},
		{"Anthropic config field", "anthropic_api_key=syntheticAnthropicConfig123", "syntheticAnthropicConfig123"},
		{"GitHub config field", "github_token: syntheticGitHubConfig123", "syntheticGitHubConfig123"},
		{"xAI config field", "xai_api_key: syntheticXAIConfig123", "syntheticXAIConfig123"},
		{"Groq config field", "groq_api_key: syntheticGroqConfig123", "syntheticGroqConfig123"},
		{"Ollama config field", "ollama_api_key=syntheticOllamaConfig123", "syntheticOllamaConfig123"},
		{"generic API key field", "API_KEY=syntheticGenericAPIKey123", "syntheticGenericAPIKey123"},
		{"password field", "db_password: syntheticDatabasePassword123", "syntheticDatabasePassword123"},
		{"client secret field", "client_secret=syntheticClientSecret123", "syntheticClientSecret123"},
		{"credential URL", "https://example-user:syntheticPassword123@example.test/db", "syntheticPassword123"},
		{"private key", "-----BEGIN PRIVATE KEY-----\nsynthetic-private-material\n-----END PRIVATE KEY-----", "synthetic-private-material"},
	}

	if len(tests) < 20 {
		t.Fatalf("secret fixture count = %d, want at least 20", len(tests))
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := Redact(tt.input)
			if strings.Contains(got, tt.secret) {
				t.Fatalf("Redact() retained synthetic secret %q in %q", tt.secret, got)
			}
			if !strings.Contains(got, redactedSecret) {
				t.Fatalf("Redact() = %q, want secret marker", got)
			}
		})
	}
}

func TestRedactMixedTokenURL(t *testing.T) {
	input := "https://example.test/path?token=syntheticOne123&next=/cb&access_token=syntheticTwo456#done"
	got := Redact(input)
	for _, secret := range []string{"syntheticOne123", "syntheticTwo456"} {
		if strings.Contains(got, secret) {
			t.Fatalf("Redact() retained %q in %q", secret, got)
		}
	}
	if strings.Count(got, redactedSecret) != 2 {
		t.Fatalf("Redact() = %q, want two secret markers", got)
	}
}

func TestNormalizeHomeDir(t *testing.T) {
	tests := []struct {
		input string
		want  string
	}{
		{`C:\Users\example-user\project\file.txt`, `~/project\file.txt`},
		{`C:/Users/example-user/project/file.txt`, `~/project/file.txt`},
		{`/home/example-user/project/file.txt`, `~/project/file.txt`},
		{`/Users/example-user/project/file.txt`, `~/project/file.txt`},
	}
	for _, tt := range tests {
		if got := normalizeHomeDir(tt.input); got != tt.want {
			t.Errorf("normalizeHomeDir(%q) = %q, want %q", tt.input, got, tt.want)
		}
	}
}

func TestRedactPrivatePaths(t *testing.T) {
	tests := []string{
		`C:\dev\kb\companies\synthetic.csv`,
		`C:\dev\.ssh\synthetic-key.pem`,
		`C:\dev\github\private\synthetic\config.yaml`,
		`/dev/github/private/synthetic/config.yaml`,
	}
	for _, input := range tests {
		if got := Redact(input); got != redactedPrivatePath {
			t.Errorf("Redact(%q) = %q, want %q", input, got, redactedPrivatePath)
		}
	}
	if got := Redact(`C:\dev\github\public\many-ai-cli`); got != `C:\dev\github\public\many-ai-cli` {
		t.Errorf("public dev path changed: %q", got)
	}
}

func TestRedactNetworkAndIdentity(t *testing.T) {
	input := "loopback=127.0.0.1 v6loop=::1 private=192.0.2.10 v6=2001:db8::1234 public=ishizakahiroshi.dev@gmail.com personal=person@example.test host=ishiz.synthetic.lan"
	got := Redact(input)
	for _, keep := range []string{"127.0.0.1", "::1", "ishizakahiroshi.dev@gmail.com"} {
		if !strings.Contains(got, keep) {
			t.Errorf("Redact() did not preserve %q: %q", keep, got)
		}
	}
	for _, remove := range []string{"192.0.2.10", "2001:db8::1234", "person@example.test", "ishiz.synthetic.lan"} {
		if strings.Contains(got, remove) {
			t.Errorf("Redact() retained %q: %q", remove, got)
		}
	}
}

func TestExtractAllowedConfig(t *testing.T) {
	cfg := &config.Config{}
	cfg.Hub.Port = 47777
	cfg.Hub.LogDir = `C:\Users\example-user\.many-ai-cli\logs`
	cfg.Token = "syntheticHubToken123"
	cfg.AuthCookieSecret = "syntheticCookieSecret123"
	cfg.Ollama.BaseURL = "http://192.0.2.20:11434"
	cfg.UserPrefs.Spawn.LastModel = map[string]string{
		"claude":  "claude-synthetic-model",
		"codex":   "sk-syntheticModelValue123",
		"unknown": "private-model-name",
	}

	got := ExtractAllowedConfig(cfg)
	want := map[string]string{
		"hub_port":     "47777",
		"providers":    "claude,codex",
		"model.claude": "claude-synthetic-model",
		"model.codex":  redactedSecret,
	}
	if len(got) != len(want) {
		t.Fatalf("ExtractAllowedConfig() = %#v, want %#v", got, want)
	}
	for key, value := range want {
		if got[key] != value {
			t.Errorf("ExtractAllowedConfig()[%q] = %q, want %q", key, got[key], value)
		}
	}
	serialized := make([]string, 0, len(got))
	for key, value := range got {
		serialized = append(serialized, key+"="+value)
	}
	joined := strings.Join(serialized, "\n")
	for _, forbidden := range []string{cfg.Token, cfg.AuthCookieSecret, cfg.Hub.LogDir, cfg.Ollama.BaseURL, "private-model-name"} {
		if strings.Contains(joined, forbidden) {
			t.Errorf("ExtractAllowedConfig() leaked disallowed value %q in %#v", forbidden, got)
		}
	}
}

func TestExtractAllowedConfigNil(t *testing.T) {
	if got := ExtractAllowedConfig(nil); len(got) != 0 {
		t.Fatalf("ExtractAllowedConfig(nil) = %#v, want empty map", got)
	}
}
