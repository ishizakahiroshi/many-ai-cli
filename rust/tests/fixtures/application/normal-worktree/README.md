# Ordinary-session worktree evidence

The behavioral oracle is Go commit
`21d0bc7935a2c4696fb89ccff2e324157a528c2d`, specifically
`internal/hub/normal_worktree.go` and `safeToken` in
`internal/hub/orchestration.go`. `worktree_identity.go` is the separate reuse
contract; the ordinary prepare/cleanup source does not invoke it.

`extract_oracle.py` reads those exact Git objects without changing the source
repository, extracts the actual normal helper and token function unchanged,
and runs the standard-library-only `observe.go` driver. Every Git mutation is
inside its fresh owned temporary directory. HOME, user profile, Git config,
identity and hooks are isolated. No remote, provider, account, existing user
worktree, or running Hub participates. The driver exits with its temporary
repositories removed. Full failures and toolchain receipts belong outside
this source-fixture directory.

Example, after loading the integration owner's pinned Rust/Go environment:

    python3 rust/tests/fixtures/application/normal-worktree/extract_oracle.py "$PWD" > /tmp/normal-worktree-observed.json 2> /tmp/normal-worktree-observer.log
    diff -u rust/tests/fixtures/application/normal-worktree/expected.json /tmp/normal-worktree-observed.json

`expected.json` contains observed Go results, not hand-written expected Git
output. OS/localized Git diagnostics are normalized to source error prefixes;
source-authored dirty/unmerged messages and names are compared in full.

## Source-to-Rust trace

- `prepareNormalWorktree` -> `NormalWorktreeLifecycle::prepare`.
  One process-wide mutex covers discovery, mkdir, best-effort exclude,
  collision selection, and `worktree add`. Input timestamp is captured by the
  caller before mutex acquisition. Shared `proto::time::Timestamp` and its
  caller-captured UTC offset preserve the local minute stamp.
- `validWorktreeCleanup` / `effectiveWorktreeCleanup` -> identically named
  snake-case Rust policy functions. Unknown/empty policy is effective manual;
  only empty/delete/keep/manual pass validation, without trimming.
- `excludeWorktreeDir` / `excludeGitPath` -> `exclude_git_path`.
  Resolve `--git-common-dir` (no alternate identity fallback), preserve existing
  bytes, recognize a trimmed complete entry, append a separating newline when
  needed, and ignore every filesystem/query error. Do not rewrite .gitignore.
- `cleanupNormalWorktree` -> `NormalWorktreeLifecycle::cleanup`.
  No-op unless created and delete policy; status must be empty, then recorded
  branch must be an ancestor of current parent HEAD, then plain non-force
  `git worktree remove`. Branch refs and the root directory remain.
- Git configuration -> canonical `child_launch::worktree::WorktreeGit`
  `command_plan`; execution -> existing `process::ManagedProcess`. No new
  session, child, process, or pending-registration registry.

## Git argv and deadline contract

All commands are argv-only with the explicit process cwd distinct from Git's
`-C` path, matching Go's inherited process cwd even for relative caller paths.

1. Prepare: `git -C <cwd> rev-parse --show-toplevel`, then
   `git -C <parent> rev-parse --git-common-dir`, share one 3-second budget.
2. Prepare: `git -C <parent> worktree add -b <branch> <path> HEAD` gets a fresh
   30-second budget and a genuine combined stdout/stderr pipe.
3. Cleanup: `git -C <tree> status --porcelain`, then
   `git -C <parent> merge-base --is-ancestor <branch> HEAD`, share one 5-second
   budget. Merge-base binds both output streams to the OS null device. Every
   merge-base failure produces the source unmerged message.
4. Cleanup: `git -C <parent> worktree remove <path>` gets a fresh 30-second
   budget and genuine combined output. Commands bind stdin to the null device.

The shared process owner's inherited one-second pipe-drain boundary remains.
As in the canonical `WorktreeGit` implementation, successful exit with forced
pipe closure/incomplete captured output is classified as `git output was
incomplete`, rather than allowing a partial status result to authorize removal.
This is an explicit inherited managed-process boundary, not a claimed Go
`Cmd.Wait` equivalence. These ordinary Git fixtures do not establish
detached-process/native acceptance.

## Preserved source quirks

- Dotted labels retain the complete basename, including the minute stamp.
- `safeToken` preserves interior `..`, and trimming happens before 80-byte
  truncation, so truncation can leave a trailing dot. This is the actual source
  behavior despite an adjacent Go comment suggesting otherwise. Git remains
  responsible for rejecting invalid branch names.
- Every existing stat-able candidate, including an ordinary file, advances the
  numeric collision suffix starting with `-2`. Branch collisions are not
  separately searched: a retained branch can make a later same-minute add fail.
- An unsuccessful add is not rolled back here. The root, best-effort exclude
  entry, and any branch ref Git left behind are not removed by this helper.
- The helper does not remove retained branch refs after successful cleanup.
- Ignored files do not make `status --porcelain` dirty. The pinned synthetic
  observation is: write an ignored file in a newly created tree; status is
  empty; the recorded branch is already an ancestor of parent HEAD; plain
  `git worktree remove` succeeds and the tree, including that ignored file,
  disappears. Thus this source policy does not preserve every kind of
  uncommitted user content. The migration keeps that observed ordering and
  behavior. A safer ignored-file policy would be a separate operator decision;
  no extra all-untracked scan or force flag is introduced here.
- Existing root/exclude modes are not chmodded. New root directories use 0700;
  exclude parent directories use 0755 and append-created exclude files 0644,
  subject to the process umask, as in Go.

## Functional cases

Rust tests consume pinned Go policy/token, dotted sequential/concurrent naming,
exclude append, clean parent, dirty/unmerged/merged cleanup, retained refs/root,
branch collision, absent-tree error, unborn repository and nonrepository
observations. Additional fresh-repo cases cover ordinary file collisions,
existing root modes, no-Git keep/manual/uncreated behavior, expired command
budget, staged/tracked dirty retention, ignored files, byte-preserved existing
exclude entry, best-effort failed exclude query, a linked parent/common-dir,
relative cwd, root creation failure, invalid token, and locked-tree removal
failure. The locked and ignored outcomes are observed by Go too.

## Integration and acceptance boundaries

This helper stays unexported until the integration owner selects it. The
existing ordinary spawn caller must record its returned `NormalWorktree` in
pending registration, switch cwd, and on launch failure clean up with delete
regardless of the session's requested policy. Registration must carry this
metadata into the existing session; dismissal must await cleanup under the
Hub's existing retained task ownership. Repeated source cleanup attempts can
legitimately report an absent-tree error; this helper does not invent idempotent
success for created metadata whose path has already disappeared.

Runtime/trial path policy belongs to those callers. This helper adds no new
repository sandbox, identity check, worktree reuse policy, or permission model.
Ordinary spawn, launch failure, registration, actual dismissal, HTTP disconnect,
Web UI, and supported-OS/native acceptance require separate caller receipts.
Formatting and an observed Go run do not establish Rust compilation or those
integration gates. The lead maintains the authoritative test receipts.
