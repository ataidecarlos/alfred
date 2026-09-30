# AI-Agent-Driven Development

A portable playbook for driving a large change with a GitHub issue ledger, an overseer agent
that never implements, and one disposable agent per issue.

The worked example is the Alfred rewrite (`ataidecarlos/alfred`). Alfred is the evidence, not
the subject: every mechanism below is generic.

## 1. What this practice is, and when it fits

1. An **overseer** splits the work into units and writes one GitHub issue per unit. It does not build.
2. Each unit gets a **worker** that owns exactly one issue, builds it, runs its acceptance
   command, and comments the command, its raw output, and the SHA.
3. The overseer **re-runs the acceptance independently**, then closes the issue.
4. Dependencies are explicit (`Blocked by:`), so agents build in order and test against what
   the previous issue actually shipped.

The ledger is the source of truth; agent context is disposable. **It fits** when the change is
too large for one context, when units can be built against a shared interface, and when each
unit has a command that proves it works. **It is overkill** when:

| Situation | Do instead |
|---|---|
| One file, one behaviour change | Just make the change. |
| No runnable acceptance exists | Fix the test story, then plan. |
| All units touch the same files | One issue; the graph buys nothing. |
| A spike or exploration | Timebox it; only write issues for decided work. |

The Alfred rewrite used 26 issues — one gate and 25 work items, eighteen planned up front and the
rest filed mid-run from defects workers found — because the deletions were not independently
mergeable; a one-file change does not need twenty-six issues.

## 2. Roles

| Role | Owns | Never does |
|---|---|---|
| **Overseer** | Taxonomy, issue text, dependency order, dispatch, verification, closing. | Implement. |
| **Worker** | Exactly one issue, start to finish. | Touch a second issue or unassigned files. |

The separation is what makes verification meaningful. A worker that builds and verifies grades
its own homework: it reports "done" with no independent check. Because the overseer never wrote
the code, its re-run is genuinely independent. Workers are leaves; they do not spawn.

**That independence has a limit worth naming.** The overseer did not write the code, so its re-run
is independent of the *implementation*. It did write the spec — and in the Alfred run **every**
defect found after dispatch was in the spec rather than the code: an acceptance command that could
not run, stale baselines, a cross-reference to the wrong issue, a field listed without its required
value. A worker that faithfully implements a wrong spec produces a passing verification of the
wrong thing, and the overseer is the last person likely to notice. For anything irreversible,
security-adjacent, or derived from a plan rather than from the code, get a second reader who sees
only the issue text and the repository, never the author's reasoning.

## 3. Step 0 — the capability and credential gate

Before the first work issue, prove the environment can do every operation the run will need. In
Alfred this was issue **#3**, and it blocked everything else.

| Check | Evidence | Why first |
|---|---|---|
| CLI installed and authenticated | `gh auth status` | Every later step is `gh`. |
| Token scopes cover later operations | scopes in `gh auth status` | Missing scope fails at use, not setup. |
| Contents write proven | create/delete a scratch ref | Read-only looks fine until the first push. |
| Commit identity exists | `git config user.name` / `user.email` | A token authenticates; it does not identify a commit. Git refuses to commit without one. |
| Shell permission config | agent config allow rules | Default-deny silently blocks operations. If workers use `git worktree` outside the project root, also grant `external_directory` for that path pattern — otherwise every external read or write stops to ask a human, which defeats an unattended run. |
| **Subagents can execute and write** | probe: temp-write, repo-write, shell-exec, gh-api | If workers cannot run, nothing is dispatchable. |
| Labels and milestones exist | `gh label list`, `gh api .../milestones` | Issue creation refers to them; retrofitting edits every issue. |
| License and third-party notices | `LICENSE`, `THIRD-PARTY-NOTICES.md` | Anything distributing code needs them before the first public commit. |
| Pre-change test baseline | `cargo test` → **101 passed, 0 failed** | Without a baseline a later failure is not attributable. |

Two scope traps: **writing a CI workflow file needs the `workflow` scope** (`Contents:write` is
rejected for `.github/workflows/`), and **branch protection needs `Administration: Read and
write`**. Alfred's Step 0 got HTTP 403 on protection because the fine-grained PAT lacked it;
that was recorded as a known limitation, and "never push to `main`" stayed an `AGENTS.md`
convention only.

If a check cannot be satisfied, record it as an explicit known limitation with the exact retry
command — do not silently drop it. **A gate failure discovered at issue 14 is a wasted week.**
Find it at issue 0, where the fix is a config edit.

## 4. The ledger is GitHub, and context is finite

Durable state living *outside* the context window is the point, not a nicety. A long-running
worker compacts or restarts; an overseer running 20 issues cannot hold every diff in mind. The
ledger carries everything needed to resume:

- Issue text: goal, files, behaviour, acceptance, dependencies.
- Comments: what each worker reported and what the overseer verified.
- Branches `issue/<number>-<slug>` and commit trailers `Agent-Issue: #n`.

A fresh session resumes from the ledger alone: `gh issue list --state open` shows remaining
work, `Blocked by:` reconstructs the graph, and the latest verified comment marks the frontier.
Because the graph is plain issue text it survives without a vendor API (`gh issue view <n> --comments`).

## 5. Issue anatomy

Every work issue uses the same sections, in this order. Uniformity is what makes an issue
dispatchable by any worker.

| # | Section | Content |
|---|---|---|
| 1 | `Blocked by:` | Comma-separated issue numbers, or `none`. The only authoritative dependency pointer. |
| 2 | `## Goal` | One paragraph: the outcome, not the technique. |
| 3 | `## Files` | Explicit paths the worker may touch. The concurrency contract. |
| 4 | `## Behaviour` | Exact semantics: types, methods, schema, edge cases. Rename per domain (`Transport behaviour`, `Types`, `Schema`, `Delete`) when clearer. |
| 5 | `## Acceptance` | A fenced, runnable command and its expected result. |
| 6 | `## Error handling` | Named failure paths and how each is recorded. |
| 7 | `## Notes` / `## Constraints` / `## Reference` | Scope guards, deliberate stubs, contract documents. |

Abridged real example (from issue #5; the Pi double is a compiled binary today):

```markdown
**Blocked by:** #4
## Goal
Add the Pi subprocess boundary, and the test fixture that lets every later issue be
validated without an API key.
## Files
- src/pi/mod.rs, src/pi/client.rs, src/pi/invocation.rs
- src/bin/fake-pi.rs (the compiled test double), tests/pi_rpc.rs
## Acceptance
    cargo test pi_rpc
Expected: green, including a framing case with an embedded U+2028 in a JSON string.
## Error handling
A line that fails to parse is logged at warn and skipped, never fatal.
```

**Rule: an issue whose acceptance cannot be re-run by a different person is not dispatchable.**
"Works as expected" is not acceptance; `cargo test job_runner` with named cases is. Write it as
one pasteable line and prefer a single test target over a manual procedure.

**Run it once before dispatch.** An acceptance command that has never executed is a guess, not a
criterion. Authoring one is easy to get wrong: `cargo test memory prompt` reads as reasonable and
fails, because cargo accepts a single `TESTNAME` positional and multiple libtest filters must
follow `--`. Validate every new acceptance at authoring time, and fix the issue text rather than
letting a worker improvise a passing substitute.

### 5.1 Before you dispatch: check the spec

The spec is the highest-risk artifact in the process, and the overseer is its only author. In the
Alfred run, eight defects were found after dispatch and **all eight were in the issue text**:

- An acceptance command that could not execute (`cargo test memory prompt`).
- A cross-reference to the wrong issue (`#4` where the assembly work was `#6`).
- Two issues told to keep an endpoint that a later issue had deleted.
- A baseline copied from an earlier brief rather than computed (34 quoted, 47 real) — twice.
- A behaviour field named **without its required value**, so the worker chose one: a five-minute
  compaction default where the requirement was twelve hours.
- An assumption written as fact (that the test binary ships in release artifacts — it does not).
- A diagnosis written as fact (that CRLF caused the `cargo fmt` failures — it did not).
- An acceptance whose only "command" was a shell comment with nothing to run.

Run this checklist on every issue before dispatch:

| Check | How |
|---|---|
| Does the acceptance execute? | Run it once, now, on the branch you are cutting. |
| Is the baseline right? | Compute it from that branch. Never copy it between briefs. |
| Do the referenced paths and endpoints exist? | `rg` every path in `## Files` and every route in `## Acceptance`. |
| Are the *values* stated, not only the fields? | A field named without its default or required value is a decision you have delegated by accident. |
| Does every `#n` resolve to the issue you mean? | Renumbering between a plan and the ledger is the usual cause. |
| Is any sentence an assumption rather than a measurement? | Measure it or mark it. "This ships in the artifact" is a measurement. |
| Is there a command, or only prose? | Bullet acceptances are acceptable only when the command is unambiguous, such as `cargo test`. |

Fixing a spec costs minutes. Discovering it at issue 14 costs a dispatch, a re-verification, and
sometimes every issue built on top of it.

## 6. The dependency graph

- `Blocked by:` text is the source of truth. GitHub's native dependency fields are optional;
  because the graph is prose, it survives without a vendor API.
- **Create issues in topological order.** Blockers get lower numbers, so a `Blocked by: #4`
  resolves the moment it is written; authoring out of order means editing numbers later.
- **The frontier** is the open issues whose blockers are all closed — the dispatch queue.
- **Do not trust inline prose cross-references.** Issue #11 said "assembled persona (#4)" when
  the assembly work was #6 — a renumbering artifact, caught while verifying this document and
  corrected. The prose drifted; the `Blocked by:` line did not.
- **Concurrency is capped by file ownership, not by tooling.** The `## Files` sections of two
  concurrent issues must be disjoint. Two agents editing the same files on two branches cannot
  merge, however clean the interface looked.
- **When no split is mergeable, make it one issue.** Alfred's prune was a single issue on
  purpose: deleting `src/llm/` broke `config_watch.rs`, `main.rs` and `server/mod.rs`, and no
  intermediate state compiled, so parallel branches could not merge in any order.
- **State the critical path** as the longest chain of `Blocked by:` edges (`#3 → #4 → #7 → #8 → #11 → #13 → #16 → #17`, the longest chain in that run). It determines the finish and shows where an extra agent would not help. By the time #11 runs, #5, #6 and #8 are closed, so it can test what it just built.
- **Find the hub files first.** A handful of files are touched by nearly everything —
  `main.rs`, `config.rs`, `server/mod.rs` in this project. They, not the graph's width, set the
  real concurrency ceiling and produce a serial tail: four of the last five issues had to run
  alone because they all edited `main.rs`. Identify them at planning time, not at the fourth
  conflict.
- **Evidence goes stale on a long run.** #40's reachability evidence was gathered several issues
  earlier, by which time five modules had been added, so the worker had to re-measure before
  deleting anything. Re-verify an issue's evidence and recompute its baseline before dispatch,
  especially when many issues have closed in between.
- **Amending the graph mid-run is part of the job.** Re-blocking an issue on a newly found
  prerequisite, splitting one out, or re-scoping one is normal overseer work. #11 and #12 were
  re-blocked on a cross-platform test double only after #10 revealed that their fixture-driven
  acceptances were being silently *skipped*; four downstream issues and a test suite depended on
  catching that. The cost of skipping the amendment is a worker building against a false premise.

## 7. The dispatch protocol

- One agent per issue; dispatch only issues whose blockers are closed.
- The worker reports **verbatim**: the exact acceptance command, its **raw output** (not a
  summary), and the commit SHA.
- The overseer **re-runs the acceptance independently**, then comments and closes. Alfred's
  closing comment on #4 is a table: `cargo build` exit 0, `cargo test` 13 passed / 0 failed,
  stale-reference grep 0 hits, `--tui` and `scheduler list` exit 2, `main` untouched at
  `acd2de9`, diff +291 / −9,658. The report was re-run, not trusted.
- If the acceptance misses a risk the overseer can see, add a check. #4 had no server test, so
  the overseer booted the server with an isolated `USERPROFILE` (the real database untouched)
  and verified `/health`, `/api/info`, the tables and the indexes.

**Fatigue-proof rule: a claim is not evidence. A comment without a command and its raw output
does not close an issue.** On failure, report it as-is, leave the issue open, and do not
summarise the failure away.

**When a worker flags a forced deviation** from the issue, the overseer reviews it, records it
in the closing comment as an accepted cascade decision, and confirms a later issue covers the
gap. #4 lists three: the scheduler module became a placeholder (#8), memory routes were removed
(#6, #10), and the Telegram connector kept only its non-agent-loop path (#12).

### 7.1 Verification is adversarial, not mechanical

Re-running the acceptance is necessary and not sufficient. There are four ways for it to pass
while proving nothing:

- **`ignored` is a tell.** Four issues had fixture-driven acceptances that were silently *skipped*
  on the host, so the command "passed" having exercised nothing. Treat `0 ignored` as part of the
  gate, exactly like `0 failed`. A count that changes unexpectedly is information.
- **The acceptance can miss the risk entirely.** #4 had no server test, so no acceptance could
  have caught a broken boot; the overseer added one and booted the server.
- **Prefer an acceptance that fails when the change is reverted.** The strongest verifications in
  this run were *negative probes*: deleting the new matrix entry made the installer/matrix guard
  fail with exactly the message it promises, and planting a decoy binary made the packaging guard
  fail. A check whose failure you have never observed is not yet a check.
- **Hermetic, or it is not repeatable.** Until `ALFRED_DATA_DIR` existed, every CLI acceptance
  wrote to the developer's real database and workers cleaned up by hand. An acceptance that
  mutates real state cannot be safely re-run — which is precisely what the overseer must do.

So: ask **"what would this acceptance not catch?"** and add the missing check. Then **read the
critical logic** rather than inferring it from green tests. The cron field order, the verdict
regex, the delivery policy, the installer's idempotency guard and the database-path precedence
were each read line by line here, and each was a place a passing test would not have been enough.

**Re-run after any rebase.** A branch that was green before being rebased has not been tested in
its merged form, and a clean rebase is not evidence that behaviour survived it.

### 7.2 The overseer's own operations are part of the system

The overseer's tooling fails too. From this run: removing a worktree while the shell's cwd was
inside it; a missing quote that silently no-opped an entire verification script; a guard that
matched the wrong text and reported "already done"; and a check run in one worktree that was read
as applying to another.

- **Print the tree and the revision with every check.** A result without its context is not a result.
- **Make the merge a scripted gate, not a judgement:** proceed only if the build is clean,
  `failed == 0`, `ignored == 0`, and the diff is confined to the issue's declared files. The scope
  check is what catches a worker quietly editing outside its `## Files`.
- **Never run git in a worktree an agent owns.** Use `-C <path>` deliberately, or finish your own
  operations before dispatching the next worker.
- **When a check contradicts the worker, suspect the check first.** Twice here the worker was right
  and the verification was wrong.

### 7.3 Discovered-but-unowned work is an output, not noise

A worker that finds an adjacent defect must **report it and leave it alone** — `AGENTS.md` already
forbids the fix. The overseer's half of that rule is to *file it*. This run's workers surfaced a
path mismatch that would silently have stopped every generated skill from loading, a checksum
verification that had never verified anything, an unwired manual-run path, a config example
overriding a corrected default, a missing release artifact, 270 lines of dead code, and two extra
`AppState` constructors. Seven follow-up issues exist only because of that loop. Discarding those
reports "to keep the diff scoped" throws away the cheapest defect discovery in the process.

## 8. The rules file (`AGENTS.md`)

Once more than one agent shares a working tree, a rules file is mandatory: it is the only thing
preventing agents from destroying each other's work.

- **Stage explicit paths only** (`git add src/main.rs src/config.rs`). Never `git add -A` or
  `git add .` — those stage another agent's work.
- **Never run destructive git**: `git reset --hard`, `git checkout .`, `git clean -fd`,
  `git stash`; also `git commit --no-verify`.
- **Never push the default branch.** Work on `issue/<number>-<slug>`.
- **Commit format** carries provenance — the `Agent-Issue: #n` trailer links a commit to its
  ledger entry after the branch is gone:
  ```
  <type>(<area>): <summary>

  Agent-Issue: #<number>
  <the acceptance command you ran>
  ```
- **The report format** (§7): command, raw output, SHA.
- **Non-goals**: list what was deliberately removed so agents do not reintroduce it. Alfred
  names an in-process LLM provider layer, a coding agent, a TUI, and OS-level cron, and says to
  raise a plan conflict rather than implement.
- **The test gate**: the suite must be green before any issue is closed.

## 9. Secrets hygiene

A real incident from this run: a **personal access token was embedded in `.git/config` as the
remote URL in cleartext**. Any `git remote -v` or a read of `.git/config` exposed it, and the
token was a classic PAT with broad scopes and no expiry.

| Rule | Why |
|---|---|
| Credential helper backed by the OS keyring. | The secret never lands in a file on disk. |
| Minimal-scope token (fine-grained: Contents, Issues, Metadata). | A leaked broad token is a much larger blast radius. |
| No token in a remote URL. | It is written to `.git/config` in cleartext and printed by `git remote -v`. |
| No token in an agent prompt or issue comment. | Prompts and comments are stored and echoed. |
| Pass provider secrets to a subprocess by **environment**, not by flag. | `--api-key` is visible in `ps`. Alfred reads the variable named by `[pi].api_key_env`. |
| Rotate on any exposure. | A token in cleartext is compromised. |

Step 0 records this as evidence: the PAT was removed from `.git/config`, rotated to a fine-grained token, and stored in the OS keyring.

## 10. Failure modes and mitigations

| Failure | Mitigation |
|---|---|
| **Context exhaustion** | The ledger. Resume from `gh issue list` plus `Blocked by:`; never rely on in-context memory. |
| **False "done"** | The overseer re-runs the acceptance independently. No command and raw output, no closure. |
| **Branch collision** — several agents cannot hold different branches in one working tree. | Give each agent its own `git worktree` (`git worktree add ../proj-4 issue/4-slug`), or serialise the agents. Alfred's tree has two worktrees for exactly this. |
| **A worker runs in the wrong tree.** A subagent inherits the *dispatcher's* working directory, not the worktree you intended. | Put the absolute worktree path in the brief, require the worker to confirm its cwd before its first `git` command, and forbid touching the other trees. Without this, a worker told "work on branch X" stages and commits into someone else's checkout. The trap cuts both ways: an overseer's own verification command can silently run against the wrong tree too, so record which tree each check ran in before believing its result — a check that contradicts the worker's report is more likely to be the broken one. |
| **Worktrees outside the project root trip an external-directory approval prompt.** | Grant `external_directory` for the worktree path pattern (for example `~/projects/proj*`) in the agent permission config, or keep worktrees inside the project. A permission prompt mid-run is not a failure the worker can recover from; it silently converts an unattended run into one that needs a human present. |
| **Permission allowlist denies an operation.** | Test the whole operation set in Step 0. A default-deny block omitted the `subagent` action here and blocked an entire dispatch. Default-deny means unlisted is denied, not warned. |
| **Fuzzy acceptance criteria** | Do not dispatch. Rewrite until the acceptance is one runnable command with named cases. |
| **The acceptance command is invalid.** | Execute every new issue's acceptance once during authoring. `cargo test memory prompt` fails — cargo takes one `TESTNAME`; multiple filters need `--`. A worker that "improves" the command instead of reporting it hides an authoring bug. |
| **Diagnosing from the symptom instead of testing the hypothesis.** | When a check flags many files, test the suspected cause before naming it. `cargo fmt --check` flagged ~90 locations, which looked like CRLF noise; a controlled experiment showed rustfmt tolerates CRLF and the files were genuinely unformatted. The wrong diagnosis would have produced a line-endings-only fix and still left ~32 files failing the check. |
| **A line-ending bug read from the working copy instead of the blob.** | `git ls-files --eol <path>` separates the committed blob (`i/`) from the working tree (`w/`). With `core.autocrlf=true` and no `.gitattributes`, a Windows working tree gets CRLF and shell scripts fail to parse **locally**, while every real Linux/macOS clone is fine. Read `i/` before calling it critical, and pin `*.sh` to `eol=lf` so the local failure cannot happen at all. |
| **A prune verified by file list, not by reachability.** | After deleting the enumerated files, check the survivors for callers: `rg` each module path outside its own directory. #4 removed exactly what its issue listed and still left `src/workspace` (139 lines, zero callers) plus `src/agent` and `src/types` (123 lines, reachable only through an event channel nothing publishes to). A delete list is a plan, not a proof. |
| **A stale baseline is copied between briefs.** | Compute the expected test count from the branch you are actually cutting, not from an earlier issue's brief. Two workers in this run had to reconcile a wrong baseline (34 quoted, 47 real); both were right and the brief was wrong. State the baseline as an observation, not a target — and if a worker reports a mismatch, check the brief before the code. |
| **Merge conflicts on shared files** | The file-ownership cap: concurrent issues must have disjoint `## Files`. |
| **Scope creep inside a worker** | `## Constraints` plus `AGENTS.md` ("keep the diff scoped; do not refactor neighbours"). A forced deviation is flagged and reviewed, not hidden. |
| **Stale inline cross-references** | Keep the authoritative pointer on `Blocked by:`; treat prose `(#n)` as hints. |
| **A blocker ships a different interface** | Update the dependent issue before dispatch. |
| **A spec error masquerades as a code error.** | Every defect found in the Alfred run was in the issue text, not the code. Run the §5.1 checklist before dispatch and read the acceptance as a stranger would. |
| **Discovered-but-unowned work is dropped.** | The worker reports it and leaves it alone; the overseer files it as an issue. Both halves are required, or the finding dies in a closing comment. |
| **Evidence and baselines go stale over a long run.** | Re-measure before dispatch, not after. #40's reachability evidence predated five new modules; two baselines were copied rather than computed. |
| **An acceptance is non-hermetic.** | A command that mutates the developer's real state cannot be re-run. Give the project an override (a data directory, a temp home) before the first CLI acceptance depends on it. |

## 11. Pacing

Run the loop sequentially with full verification first: one issue, one worker, one independent
re-run, close. Do not fan out until the loop is proven end to end — a protocol bug corrupts
every parallel branch at once. Once proven, fan out only across disjoint files with closed
blockers; the graph's width is the ceiling.

| Verification mode | Benefit | Cost |
|---|---|---|
| Per issue | Errors caught at the smallest diff. | Overseer serialises the run. |
| At a phase boundary | Higher throughput. | Several issues may inherit one root cause; attribution is harder. |

If you batch, batch within a phase (Alfred's milestones A–E), not across phases, and still re-run every acceptance command at the boundary.

## 12. Closure is not verification

An empty issue list does not mean the project works. The Alfred run closed 26 of 26 issues and
still had four asks that **could not be executed in that environment**: building the Docker image
(no Docker on the host), a successful first install (no release tagged since the rewrite), a real
Telegram send (no bot token), and a real CI run (no tag push). Each was stated honestly at the
issue level and was invisible at the project level.

Keep two registers, and separate them in the final report:

| Register | Meaning |
|---|---|
| **Verified by execution** | A command ran, against a named revision, and its output is recorded. |
| **Unverifiable here** | No command exists in this environment. Name the environment that could verify it, and the exact command. |

The per-issue acceptance is where work is *closed*; the not-verified register is where the
project's true state is *stated*. Keep it in one place — a "not verified" section in `ROADMAP.md`
is enough — because scattered across closing comments it is not readable. Never let "0 open" stand
in for "everything works".

## 13. Adoption checklist for a new project

1. Create the repository and push an initial commit.
2. Add `LICENSE` and `THIRD-PARTY-NOTICES.md` if the project distributes code.
3. Install and authenticate the CLI (`gh auth status`); confirm scopes, adding `workflow` for
   CI files and `Administration` for branch protection.
4. Configure a commit identity (`git config user.name` / `user.email`) and store git auth in
   the OS keyring helper; never a token in a remote URL.
5. Create the taxonomy **before the first issue**: phase labels (`phase:a-…`), area labels
   (`area:pi`, `area:jobs`, `area:delivery`, `area:docs`), workflow labels (`blocked`,
   `in-progress`, `needs-verification`, `verified`), and milestones.
6. Write `AGENTS.md`: git discipline, commit format, report format, secrets rules, non-goals,
   test gate.
7. Record the pre-change baseline (the test counts) in the Step 0 issue.
8. Prove the agent environment: spawn a probe and verify temp-write, repo-write, shell-exec,
   gh-api.
9. Open the Step 0 gate issue and close it with evidence or a recorded known limitation, then
   write the first work issue in the §5 anatomy (`Blocked by: none`, exact acceptance command).
10. Run the **§5.1 spec checklist** on that issue before dispatching it. Correct the issue text,
    not the worker's interpretation of it.
11. Identify the hub files (the few touched by nearly everything) and compute the critical path;
    confirm the first dispatchable frontier.
12. Give the worker its own `git worktree`, dispatch, and require the report: command, raw
    output, SHA.
13. Re-run the acceptance independently — after any rebase too — and gate the merge on build
    clean, `0 failed`, `0 ignored`, and a diff confined to the declared files.
14. File anything the workers report out of scope (§7.3), and keep the not-verified register
    (§12) up to date as you go, not at the end.
