# Files

- [Approval UI and Marker Filtering](approval-ui.md) - The browser-side approval action bar and batch-question panel, the single-source candidateKey+sourceEpoch identity model in approval-answered.ts, and the tag-stripping hub-marker-filter.ts that must tolerate CLI-owned alternate screens.
- [Web Frontend Architecture](architecture.md) - The unbundled, per-file esbuild pipeline behind the TypeScript Web UI, the classic-script global-scope module style it targets, and how it reaches the browser as a go:embed asset.
- [Session Views: Multi-Pane, Detached Grid, and Mobile](session-views.md) - The alternate ways the Hub UI renders live sessions beyond one-terminal-at-a-time — the multi-pane grid, pop-out detached windows, the sidebar's single-tree placement rule, and the mobile-specific lite/home views.
