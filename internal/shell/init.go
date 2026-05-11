package shell

func InitScript() string {
	return `
if [ "${ANY_AI_CLI_AUTO:-0}" = "1" ]; then
  claude(){ any-ai-cli claude "$@"; }
  codex(){ any-ai-cli codex "$@"; }
  gemini(){ any-ai-cli gemini "$@"; }
fi
`
}
