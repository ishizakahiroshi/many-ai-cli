package report

import (
	"runtime"
	"sort"
	"strings"

	"many-ai-cli/internal/config"
)

// CollectOptions contains only values approved for inclusion in a bug report.
// Callers must not pass serialized configuration or session logs here.
type CollectOptions struct {
	Version   string
	Provider  string
	Model     string
	UserAgent string
	Config    *config.Config
}

// Environment is the allowlisted, editable environment section of a report.
type Environment struct {
	Version       string
	OS            string
	Architecture  string
	GoVersion     string
	Provider      string
	Model         string
	UserAgent     string
	AllowedConfig map[string]string
}

// Collect gathers only allowlisted runtime and configuration metadata.
func Collect(opts CollectOptions) Environment {
	provider := strings.TrimSpace(opts.Provider)
	if !isAllowedProvider(provider) {
		provider = ""
	}
	return Environment{
		Version:       cleanValue(opts.Version),
		OS:            runtime.GOOS,
		Architecture:  runtime.GOARCH,
		GoVersion:     runtime.Version(),
		Provider:      provider,
		Model:         cleanValue(opts.Model),
		UserAgent:     cleanValue(opts.UserAgent),
		AllowedConfig: ExtractAllowedConfig(opts.Config),
	}
}

func cleanValue(value string) string {
	return strings.TrimSpace(Redact(value))
}

func isAllowedProvider(provider string) bool {
	i := sort.SearchStrings(allowedProviders, provider)
	if i < len(allowedProviders) && allowedProviders[i] == provider {
		return true
	}
	for _, allowed := range allowedProviders {
		if provider == allowed {
			return true
		}
	}
	return false
}
