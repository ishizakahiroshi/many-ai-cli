package shell

func InitScript() string {
	return `
if [ "${MANY_AI_CLI_AUTO:-0}" = "1" ]; then
  claude(){ many-ai-cli claude "$@"; }
  codex(){ many-ai-cli codex "$@"; }
  copilot(){ many-ai-cli copilot "$@"; }
  cursor-agent(){ many-ai-cli cursor-agent "$@"; }
  grok(){ many-ai-cli grok "$@"; }
fi
`
}
