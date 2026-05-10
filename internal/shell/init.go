package shell

func InitScript() string {
	return `
if [ "${AI_CLI_HUB_AUTO:-0}" = "1" ]; then
  claude(){ ai-cli-hub claude "$@"; }
  codex(){ ai-cli-hub codex "$@"; }
  gemini(){ ai-cli-hub gemini "$@"; }
fi
`
}
