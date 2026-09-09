# Files

- [PTY Wrapper and Session Lifecycle](session-lifecycle.md) - How internal/wrapper starts a provider CLI in a real OS PTY, the idempotent input-sequence protocol that prevents a resend from double-submitting a prompt, and the reconnect-grace state machine that keeps a session alive through a Hub crash or restart.
