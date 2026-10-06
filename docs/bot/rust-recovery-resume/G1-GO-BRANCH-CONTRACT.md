# Fixed-Go G1 branch-refresh contract

> 最終更新: 2026-10-06(火) 14:15:48 UTC

Read-only source analysis, 2026-10-06. Source checkpoint `87fa9b735c7fa99b8776b8a58bb278290078d94b`. Oracle: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`. `git diff --exit-code <oracle> -- internal/hub internal/proto` returned 0, with no diff: all cited Go implementation/tests/protocol files match the oracle. No tests, builds, providers, devices, or application startup were run. The underlying analysis made no implementation changes. This published document and [JSON companion](G1-GO-BRANCH-CONTRACT.json) retain that fixed-Go contract; they do not certify the subsequent Rust implementation.

Bare filenames below are under `internal/hub/`; protocol references are under `internal/proto/`. Paths and line numbers bind the fixed-Go oracle. Implementation, rather than comments that disagree with it, is authoritative.

## Scheduling and every production caller

- `server.go:1797` launches `stateTicker(runCtx)`. `idle_state.go:69-84` evaluates every 200 ms and stops its ticker on context cancellation. Constants: `server.go:49-56`: 200 ms tick, 250 ms helper timeout, 2 s refresh period, 4 workers.
- The sole production caller of `queueBranchRefreshes` is `evaluateIdle`, `idle_state.go:110-118,159`. Under the sessions lock, every session with `now - branchCheckedAt >= 2 s` gets its timestamp advanced immediately and an `(id,CWD)` request. This happens before terminal-state filtering; ended/terminal sessions still refresh while present. No UI-presence/provider/state filter. Queueing follows state broadcasts and occurs outside the sessions lock. Due time measures the last enqueue, not Git completion.
- Fresh registration synchronously calls `gitBranch(reg.CWD)` (`wrapper_loop.go:126-128`), stores raw `reg.CWD`, and leaves `branchCheckedAt` zero (`:253-258`): first state tick is due. Project/stats begin unchecked.
- Reattach synchronously calls `gitBranch(req.CWD)` (`wrapper_loop.go:443`), constructs its replacement with raw `req.CWD` and `branchCheckedAt=now` (`:711-744`): first scheduled refresh is due around 2 s later. Warm reattach preserves ProjectID/projectChecked and gitChecked/counts (`reattach_state.go:127-128,156-159,258-259,292-295`; apply at `wrapper_loop.go:781-782`). Cold reattach starts those fields unchecked. Branch and branchCheckedAt are not preserved.
- The sole production caller of `refreshBranchForCWD` is the queued worker (`branch_refresh.go:91`). The only other production `gitBranch` callers are registration and reattach above. `gitChangeStats` and `projectRootLookup` are used by refresh only. `projectRootLookup` defaults to `gitProjectRoot` and is a test seam (`branch_refresh.go:27-29`).

## Queue, concurrency, and pending requests

Source: `branch_refresh.go:31-108`; initial state: `server.go:1383-1385`.

1. Empty requests return. Trim each requested cwd with `strings.TrimSpace`; discard empty; group IDs by the trimmed string. There is no absolute-path conversion, symlink resolution, case folding, separator cleaning, or repository-root grouping. Distinct aliases/subdirectories remain different work keys.
2. Under `branchRefreshMu`, lazily create in-flight and pending maps. If the cwd is busy (even if its goroutine is still waiting for a worker slot), merge IDs into its pending list, preserving first insertion order and suppressing duplicates there. Initial grouped IDs are not deduplicated. Map iteration/start order across cwd keys is unspecified.
3. Otherwise mark the cwd in-flight, copy IDs, and launch `safeGo`. The four-slot semaphore bounds concurrently executing refreshes, not spawned waiting goroutines or queue length. Slot acquisition precedes all Git commands.
4. On exit, including panic unwinding, release the semaphore slot, lock the queue, delete the in-flight key, and, if pending IDs exist, delete that pending entry and start another worker for them immediately. Pending requests are never silently dropped; they do not wait for another 2 s tick. They can coalesce requests for the same already-running session as well as newly registered sessions.
5. The semaphore covers lookup, state application, and broadcasts. It is not released immediately after Git. There is no queue cancellation/select on server context; each Git helper uses its own background-derived timeout. `safeGo` recovers/logs panics (`safe_go.go:9-24`).

## Exact Git subprocess contract

All helpers use direct `exec.CommandContext(..., "git", "-C", cwd, ...)` with `.Output()`. No shell, no explicit subprocess cwd, no explicit environment, no stderr interpretation, and no `runGit` wrapper. They inherit parent cwd/PATH/environment/Git config. In particular, branch-refresh does not add `core.quotePath=false`, `GIT_OPTIONAL_LOCKS=0`, an isolated Git environment, or Git API's 5 s timeout. Compare the separate API helper at `git_common.go:139-141,212-231`.

Each helper first checks whether trimming cwd would be empty; if nonempty it passes the original supplied cwd to Git. The asynchronous queue already trims its cwd; register/reattach call gitBranch on raw cwd. Each helper creates a separate 250 ms deadline. Its first and optional second subprocess share that deadline; it is not 250 ms per subprocess. A refresh can therefore use roughly three helper budgets, plus queue/broadcast time. The timeout is Go CommandContext's process cancellation, not a hard entire-worker latency guarantee.

### Branch (`server.go:544-566`)

- Execute `git -C <cwd> rev-parse --abbrev-ref HEAD`.
- Failure returns empty string. Trim successful stdout. Anything other than literal `HEAD` is returned directly (including empty stdout).
- If literal `HEAD`, execute `git -C <cwd> rev-parse --short HEAD` within the same deadline. Error/empty trimmed output returns empty; otherwise `detached:<hash>`.
- An unborn repository has no successful HEAD resolution, so returns empty. Error-derived empty values can replace an older known branch at the next application; there is no branch success latch.

### Change stats (`server.go:573-616`)

- Execute `git -C <cwd> status --porcelain`. Failure returns `(0,0,0)` without running diff.
- `files` is the number of newline-separated stdout lines whose TrimSpace result is nonempty. It is a porcelain-record/line count, not a set of filesystem files. Default Git porcelain can represent an entire untracked directory with one line; quoted filenames/renames count as their printed records. No `-z`, `-uno`, or explicit untracked mode.
- Execute `git -C <cwd> diff --numstat HEAD` using the same stats deadline. **If diff fails, preserve files and return `(files,0,0)`**. The comment claiming any failure returns all zeros is wrong. This is especially relevant before the initial commit.
- On successful diff: split lines by newline, TrimSpace each, skip empty, SplitN(tab,3), require at least 2 fields; parse each of the first two independently with `strconv.Atoi` and add each successfully parsed integer. Binary `-` contributes zero, as do malformed/overflowing fields. There is no nonnegative clamp. The third field/path is unused.
- Ordinary Git semantics: status includes staged/unstaged/untracked changes; diff against HEAD measures the net tracked worktree difference against HEAD, including staged changes, and excludes untracked content. Thus files can be nonzero while additions/deletions are zero, including binaries and unborn repos. Commands/config are the exact contract; no file-content counting is performed by the Hub.

### Main-root/project identity (`project_id.go:40-92`)

- Blank cwd returns `("",true)` without a command.
- First execute `git -C <cwd> rev-parse --path-format=absolute --git-common-dir`.
- On any error, retry `git -C <cwd> rev-parse --git-common-dir` within the same deadline (older-Git fallback). The final/fallback failure determines the outcome.
- On successful output: TrimSpace; empty output is `("",false)`. `filepath.Clean(common)`; if not absolute, `filepath.Clean(filepath.Join(cwd,common))`. If `filepath.Base(common)==".git"` exactly, return Clean(parent), resolved=true; otherwise common itself, resolved=true (e.g. bare or separately named common directory).
- Normal main repo, its subdirectories, normal linked worktrees, and relay worktrees all resolve to main repository root via common `.git`, not worktree `--show-toplevel`. Bare repositories retain their common dir itself, unless its literal basename is `.git` (actual implementation applies the basename rule unconditionally).
- Uses host-native filepath semantics: Windows output gets platform path cleaning; no explicit drive-letter case normalization, canonicalization, filesystem existence check, or symlink evaluation. Fallback joining does not guarantee absolute output when the input cwd itself is relative, despite the function's absolute-path comment.
- Failure classification first checks `ctx.Err()!=nil` => unresolved, even if exit error/not-found also applies. Otherwise `exec.ErrNotFound` => resolved empty; any `*exec.ExitError` => resolved empty; any other launch/process error => unresolved. It does not inspect stderr or require a particular exit code, so 'definitive nonrepo' includes other Git-reported errors. Missing cwd is not separately classified (existing Go test permits OS-dependent resolved flag). Since no `cmd.Dir` is set, a nonexistent `-C` directory normally produces a Git ExitError and resolved empty. A raw process-spawn ENOENT (e.g. executable disappeared after lookup or interpreter missing) is not `exec.ErrNotFound` and remains unresolved; ordinary failed PATH lookup is `exec.ErrNotFound` and resolved empty.

## Applying results, latches, races, wire payload

Source: `branch_refresh.go:111-179`.

- Always obtain branch then stats first, even if all captured IDs were deleted or changed cwd. Then lock sessions and ask whether at least one captured ID still exists, has **exact** CWD equality with worker cwd, and lacks projectChecked. Only then query project root once for this worker. Release the sessions lock during all Git calls.
- Re-lock and visit captured IDs. Missing sessions or different exact CWD are skipped. Eligibility is ID+CWD only: no pointer, generation, provider, wrapper connection, lifecycle state, or reattach epoch check.
- branchChanged = stored branch differs. gitChanged = !gitChecked OR any count differs. projectChanged = projectNeeded AND projectResolved AND !session.projectChecked.
- On projectChanged, store returned project root (including definitive empty) and set projectChecked. **An unresolved lookup never latches or overwrites ProjectID.** Subsequent scheduled refreshes retry root until resolved. Once checked, project root is not re-run for that session; checked sessions in mixed groups keep their existing identity even if the shared root differs. Branch/stats keep refreshing even for definitive nonrepos or missing Git.
- If none of the three changed flags is true, emit nothing. Otherwise store branch; set gitChecked=true even after command failures; store all counts; append one `session_update`. The first stats observation therefore emits even when branch/root/counts are empty/zero. Resolving empty project identity emits once even when no visible value changes. An unchanged later observation emits nothing.
- Payload fields populated, exactly: Type=`session_update`, SessionID, Provider, Display, CWD, Branch, ProjectID, Label, Model, Route, State, LastOutputAt, StartedAt, FirstMessage, LastMessage, GitChecked=true, GitFiles, GitAdded, GitDeleted. Metadata is sampled from the session at application time. No Activity, transcript fields, provider revision, subscription fields, wrapper info, or store update is populated by this path.
- Wire `git_checked` and stats have `omitempty`; true checked is present; zero counts omitted (`proto/messages.go:361-367`). Branch/project_id and other optional empty string fields likewise omit. The marker distinguishes an observed zero stat from an update carrying no stats. Session snapshot serializes public session fields; gitChecked/counts are private session fields and do not become snapshot fields (`server.go:219-226`; `ui_broadcast.go:247-257`).
- `broadcast` does not enrich the payload; it suppresses a message if its current SessionID still names a UsageProbe session, queues it for priming/draining UI connections, otherwise sends to each UI with a fresh 5 s deadline (`ui_broadcast.go:425-456`). Scheduling/application still run for those probe sessions.
- Messages are collected under sessions lock and broadcast after unlocking. Deleting/changing/replacing a session before application prevents outdated-CWD application; deleting it after message construction does not cancel an already prepared broadcast. Deletion does not purge queue work (`server.go:2522-2549`).
- Same-ID/same-CWD reattach replacement can receive an older worker's result; this is the oracle's behavior. Same-ID/different-CWD replacement is skipped by the old worker. No explicit branch-refresh generation exists. Warm reattach preservation itself is unconditional, so ProjectID/projectChecked can survive a changed cwd if the preserved-state path is reached. With an existing wrapper, reattach's identity check requires PID/provider/CWD match or renumbers; with no wrapper, a surviving session at accepted ID still gets preserved state (`wrapper_loop.go:391-398,485-500,603-604,717,781-782`). Do not claim the Go code resets projectChecked on every cwd change; it does not.
- Queue trimming + exact stored-CWD comparison means surrounding whitespace in stored CWD can prevent asynchronous application. Different casing/aliases remain separate work keys on Windows as well. These are exact oracle quirks, not recommendations to change Go during parity work.

## Existing Go regression coverage (located, not run)

- `project_id_test.go:34` TestGitProjectRootReturnsRepositoryRoot
- `:46` TestGitProjectRootFromSubdirectory
- `:65` TestGitProjectRootFromWorktree (explicitly rejects worktree itself)
- `:85` TestGitProjectRootOutsideRepository (empty, resolved=true)
- `:99` TestGitProjectRootWithMissingOrEmptyPath (blank resolved=true; missing path only checks empty value)
- `project_id_latch_test.go:40` TestRefreshBranchKeepsRetryingUnresolvedProjectRoot
- `:64` TestRefreshBranchLatchesResolvedEmptyProjectRoot (root lookup called exactly once across two refreshes)
- `:87` TestQueueBranchRefreshesQueuesRequestForBusyCWD ([5] + [5,6] => pending [5,6])
- `:106` TestBranchRefreshDrainsPendingRequests (block first lookup, enqueue second session, release, assert second resolved)
- These project tests have no OS build tag or Unix guard; the real-Git helper invokes executable `git` and fails on nonzero exit (`normal_worktree_test.go:60-65`). `samePath` in root tests cleans paths and compares EqualFold, including Windows drive-letter variation (`project_id_test.go:29-32`).
- Repository test search found no direct tests naming gitBranch/gitChangeStats/gitLookupAnswered, no dedicated four-worker-cap or stale-CWD refresh test, and no branch/project preservation assertions in the general reattach logical-state test. Those are coverage gaps, not evidence of incorrect behavior.

## Strongest minimal Rust regression set

1. **Real Git on Linux and actual Windows**: one fixture with committed main repo, nested cwd, linked worktree outside it, and path containing spaces/non-ASCII. Verify ordinary branch, detached `detached:<short>`, main-root identity for main/nested/worktree, cleaned host paths, and git_executable availability as a required CI prerequisite. Real Windows execution must be recorded separately; Linux cross-compilation is not Windows execution.
2. **Real stats semantics**: tracked file with staged and unstaged net changes, deletion, binary, untracked file plus untracked directory. Compare expected tuple to literal oracle commands. Separately unborn repo: status count survives diff/HEAD failure. Nonrepo: empty branch, zero stats, project resolved empty. No shell-script-only fake Git in Windows-capable tests.
3. **Project latch table**: unresolved->success retry, unresolved preserves previous ProjectID, resolved-empty latches once, checked session skips lookup, mixed checked+unchecked same-cwd preserves checked identity. Distinguish timeout, missing executable, normal nonzero exit, other spawn error, and empty-success output. Test fallback command and deadline sharing using an injectable runner or portable helper executable.
4. **Pending/concurrency**: block first cwd refresh; enqueue second session and repeated IDs; assert no concurrent work for that cwd, unique pending IDs, immediate second pass, and the new session resolves. With five independent cwds, assert max four active workers and eventual fifth completion.
5. **Result race/wire**: delete ID and change cwd during lookup => no application/update; same-ID/same-cwd replacement => old result applies per oracle. Initial all-zero stats emits checked=true, unchanged result suppresses, project-only resolution emits, later Git failures can clear old branch/stats. Verify precise sparse wire fields and current session metadata.
6. **Scheduler/reattach**: controllable time verifies fresh registration due on first tick; subsequent refresh due at 2 s; terminal sessions still due; timestamp advances before busy-cwd queueing; warm reattach preserves root/counts and resets due time to now; cold reattach starts unchecked but also waits 2 s. Do not turn stronger generation rejection or cwd-reset assumptions into claimed oracle parity.

Keep deterministic runner/latch/scheduler tests fast and separate from actual-Git platform integration. They answer different questions; neither replaces native Windows Git tests.
