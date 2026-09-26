package doctor

import (
	"many-ai-cli/internal/config"
	"many-ai-cli/internal/nvidianim"
)

// nvidiaNIM reports local route/key configuration only. The connection test in
// Settings is the only Doctor-adjacent path that calls NVIDIA, and it checks
// the fixed /v1/models endpoint without returning response bodies.
func nvidiaNIM(cfg *config.Config) Check {
	if !cfg.NVIDIANIM.Enabled {
		return Check{"NVIDIA NIM", OK, "DISABLED: NVIDIA NIM route is turned off", ""}
	}

	configDir, err := config.Dir()
	if err != nil {
		return Check{"NVIDIA NIM", Warn, "API key status is unavailable; details are hidden", "Check the Hub settings directory permissions"}
	}
	key, _, err := nvidianim.ResolveAPIKey(configDir)
	if err != nil {
		return Check{"NVIDIA NIM", Warn, "API key status is unavailable; details are hidden", "Check the Hub settings directory permissions"}
	}
	if key == "" {
		return Check{"NVIDIA NIM", Warn, "NOT CONFIGURED: no NVIDIA API key is configured", "Configure a key in Hub Settings → NVIDIA NIM"}
	}

	return Check{"NVIDIA NIM", Warn, "Configured; API connectivity was not checked by Doctor", "Use Test connection in Hub Settings to check NVIDIA /v1/models"}
}
