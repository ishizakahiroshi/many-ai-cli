# Files

- [CLI Entrypoints and Subcommands](cli-entrypoints.md) - How the many-ai-cli binary dispatches its subcommands (serve, wrap, setup, doctor, tray, uninstall, and more), and what the supporting internal packages behind each one do.
- [Configuration and On-Disk Layout](configuration.md) - How config.yaml is loaded, defaulted, migrated, and atomically saved by internal/config, and what lives under ~/.many-ai-cli.
- [System Architecture Overview](overview.md) - The three-process model behind many-ai-cli — wrapped provider CLIs, the local Hub daemon, and the browser Web UI — and how they connect over one WebSocket endpoint.
