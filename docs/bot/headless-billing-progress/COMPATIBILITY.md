# Claude headless billing compatibility boundary

> 最終更新: 2026-10-11(日) 02:09:48 JST

## Supported exact version

Only Claude Code 2.1.295 is enabled by the Rust headless billing gate. Unknown
versions stop before inference. No CLI update or installation is performed.
This is a source-reviewed compatibility target with synthetic Rust fixtures,
not a claim that an installed Claude process or live account has been tested.

## Reproducible source evidence

- Official launcher package: https://registry.npmjs.org/@anthropic-ai/claude-code/2.1.295
- linux-x64 distribution: https://registry.npmjs.org/@anthropic-ai/claude-code-linux-x64/-/claude-code-linux-x64-2.1.295.tgz
  - Verified registry integrity: `sha512-nC9RZ6ZArEavyAiFdEtGK6/XPhjfE79tP2tCh+PjmRUxSO/6W3IuXv/rmo8c6u2UzHyvgKodRj1z6UcBGlbnyA==`
  - Extracted executable SHA-256: `4503bfe11a6c7fcc1e0b39b5e0d347c04248f750b03b0977b3ad6b531fe6f358`
- win32-x64 distribution: https://registry.npmjs.org/@anthropic-ai/claude-code-win32-x64/-/claude-code-win32-x64-2.1.295.tgz
  - Verified registry integrity: `sha512-absbtyFKyhebVBSilCIj/JXVERLjXDAo4ID3nVRwNb52HCXfjd+g+b1THrq6NCNXCVcrpC+skkNfBqqy0N726A==`
  - Extracted executable SHA-256: `a30012741c3d0b55e2a6c252dab3781134b75be95896ae89d40a93e10f83bcd6`
- Embedded source build: `2026-10-08T16:50:59Z`, Git identifier
  `07e8f67ea3282bf154a9e05673a0942b1b173cef`.
- Official changelog: https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md
- Documentation: https://code.claude.com/docs/en/cli-reference and
  https://code.claude.com/docs/en/authentication

Both platform distributions were checksum-verified and opened as data only.
Readable bundled JavaScript is embedded after the native/bytecode sections;
source inspection did not execute the binaries. They were not installed or
updated. No real resolver, authentication material, model request, or billed API
was used. Distributions are not repository contents or product dependencies.

## Reviewed contracts

- Bare-mode key getters `DE` (Linux) / `Fk` (Windows), token getters `Wl` / `jl`,
  and helper selector `am`: bare authentication is limited to the API-key
  environment or explicit flag-settings helper, with no OAuth/keychain fallback.
  Rust refuses competing environments before launch and provides the pinned
  recipe through one owned settings file.
- Helper functions `d$` / `XK` invoke the resolver inside Claude and consume its
  trimmed stdout. Failure handling `ok` / `nk` can echo limited helper stderr
  into Claude stderr and debug logs. The resolver contract therefore forbids
  secrets in stderr. Rust API mode suppresses raw stderr, raw logs,
  init/error/tool payloads and user debug flags. This is not containment of a
  malicious resolver/provider or a guarantee about vendor-owned private logs.
- The parser registers bare, partial-message, settings and budget options.
  Budget predicates `KU` / `JU` compare current-run ledger cost (excluding
  restored cost) and fail closed on a nonnumeric total. The engine checks the
  budget between events; subagent launch also checks it. In-flight calls can
  still overshoot. This remains an estimated stopping threshold, not a hard cap.
- Budget exhaustion emits `error_max_budget_usd` with `is_error: true` in
  2.1.295. The earlier 2.1.81 source comparison emitted false, so the Rust
  parser also treats a non-success subtype as failure independently of exit.
  2.1.81 is comparison evidence only and is not enabled by this gate.
- Auth status emits JSON including loggedIn, authMethod, apiProvider,
  subscriptionType, configDirectory, and optional forcedLoginMethod and
  allowedProviders. Rust accepts only verified claude.ai pro/max status with
  first-party routing and no reported managed override. The vendor maps
  claude_max/claude_pro organization types to max/pro; max_5x and max_20x are
  separate rate-limit tier labels, not subscriptionType values. Team, enterprise,
  missing/unknown status and conflicts stop. Auth status never tries an API
  recipe. The vendor does not guarantee it never refreshes credentials.
- Managed sources include remote-settings.json, system managed-settings.json,
  macOS device/per-user MDM and Windows policy registry settings. Unknown
  managed overrides are rejected, not rewritten. WSL may inherit Windows
  managed sources and is conservatively refused by this initial inspection.

## Probe and launch root equivalence

The version/auth probes use the same resolved executable, working directory and
environment as the final process. They intentionally omit inference options.
Consequently, caller-supplied settings roots, parent managed settings, signed
client configuration and alternate execution roots are refused before either
probe. In particular, `--project-config-root` cannot point inference at a
project that the cwd-based auth scan and `auth status` did not inspect. This is
a conservative refusal, not support for arbitrary extra settings roots.

Source review of the checksum-verified 2.1.295 distributions found:

- Linux offsets 224930963 and 208776217: projectConfigRoot is activated before
  setup and becomes the project/local settings directory. Its option is
  declared at 224951142; the Windows declaration is at 229520264.
- Linux options at 224950924 and 224950663 (Windows 229520053 and 229519792)
  provide parent-managed settings and signed client configuration. These are
  separate auth/policy sources, so both are refused in either billing mode.
- Linux debug options at 224936138/224936276 and Windows
  229505268/229505405 include `-d`, `-d2e` and `--debug-to-stderr`. The guard
  covers equals forms, attached short values and boolean short-option clusters,
  as well as the known environment/settings-env switches for hosted auth and
  debug output. It leaves ordinary model/effort options and label values intact.
- The `-p, --print` declaration at Linux 224936588 is a boolean flag, not a
  value-taking option. Print-mode `loadInitialMessages` (`FL`, 241847277)
  restores worktree context for both resume and continue through `wC`
  (241846336) and `gIe`/`Ne` (223563976). That path calls `Bm` (206701942),
  which changes the process working directory after the initial auth check.
  Resume, continue and fork selectors are therefore refused in plan-login as
  well as API mode, including short attached and clustered spellings.
- The config-home getter `Ee` at Linux 206606963 is keyed by the current
  process CLAUDE_CONFIG_DIR. The SDK schema at 207761602 confirms that user
  or managed settings env blocks can change that directory. Nested
  settings.env.CLAUDE_CONFIG_DIR is refused; an initial process
  CLAUDE_CONFIG_DIR remains supported and is the directory inspected by Rust.

The regression fixture creates an isolated alternate project containing a
synthetic apiKeyHelper. Supplying that root in the final assembled argv now
returns the fixed auth-conflict error before either synthetic probe can run.
No actual CLI, resolver or credential is used.

## Verification boundaries

Synthetic tests cover configuration, selection, revision changes, permissions,
settings ownership/cleanup, auth conflicts and output handling. They do not
prove native Windows ACL behavior, actual installed CLI behavior, or a live
account's billing outcome. New exact versions need a fresh compatibility review;
a matching semver range or absent help flag is not evidence of compatibility.

## Synthetic checks

After the repository's locked Web build, run from `rust/`:

```sh
cargo test --locked --lib config::headless_billing
cargo test --locked --lib wrapper::launch::billing
cargo test --locked --lib billing_selection_is_ephemeral_pinned_and_reconfirmable
cargo test --locked --lib billing_files_are_denied_before_workspace_and_handoff_grants
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

The fixtures create isolated temporary profiles and synthetic shell processes;
they do not invoke the actual Claude executable or a real resolver. Native
Windows CI remains a separate compilation/test result and must be reported as
run, failed, or skipped against the implementation SHA, never inferred here.
