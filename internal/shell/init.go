package shell

import (
	"regexp"
	"strings"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/provider"
)

func InitScript() string {
	return InitScriptForProviders(provider.BuiltinProviderIDs)
}

// InitScriptForConfig keeps the shell aliases on the same provider snapshot as
// the command dispatcher. IDs that cannot be represented safely as a shell
// function name are intentionally omitted; they remain available through
// `many-ai-cli wrap <id>`.
func InitScriptForConfig(cfg *config.Config) string {
	ids := append([]string(nil), provider.BuiltinProviderIDs...)
	if cfg != nil {
		for _, custom := range config.EffectiveCustomProviders(cfg.CustomProviders) {
			ids = append(ids, custom.ID)
		}
	}
	return InitScriptForProviders(ids)
}

func InitScriptForProviders(ids []string) string {
	var aliases strings.Builder
	for _, id := range ids {
		if !shellFunctionIDPattern.MatchString(id) {
			continue
		}
		aliases.WriteString("  ")
		aliases.WriteString(id)
		aliases.WriteString("(){ many-ai-cli ")
		aliases.WriteString(id)
		aliases.WriteString(` "$@"; }`)
		aliases.WriteByte('\n')
	}
	return `
if [ "${MANY_AI_CLI_AUTO:-0}" = "1" ]; then
` + aliases.String() + `fi
`
}

var shellFunctionIDPattern = regexp.MustCompile(`^[a-zA-Z_][a-zA-Z0-9_-]*$`)
