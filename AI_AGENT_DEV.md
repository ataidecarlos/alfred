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

The Alfred rewrite used 19 issues (one gate, 18 work items) because the deletions were not
independently mergeable; a one-file change does not need nineteen issues.

## 2. Roles

| Role | Owns | Never does |
|---|---|---|
| **Overseer** | Taxonomy, issue text, dependency order, dispatch, verification, closing. | Implement. |
| **Worker** | Exactly one issue, start to finish. | Touch a second issue or unassigned files. |

The separation is what makes verification meaningful. A worker that builds and verifies grades
its own homework: it reports "done" with no independent check. Because the overseer never wrote
the code, its re-run is genuinely independent. Workers are leaves; they do not spawn.

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

Abridged real example (issue #5):

```markdown
**Blocked by:** #4
## Goal
Add the Pi subprocess boundary, and the test fixture that lets every later issue be
validated without an API key.
## Files
- src/pi/mod.rs, src/pi/client.rs, src/pi/invocation.rs
- tests/fixtures/fake-pi.sh, tests/pi_rpc.rs
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
- **State the critical path** as the longest chain of `Blocked by:` edges (`#3 → #4 → #7 → #8 → #11 → #13 → #16 → #17`, the longest chain in that run). It determines the finish and shows where an extra agent would not help. By #11, #5, #6 and #8 are closed, so it can test what it just built.

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
| **A prune verified by file list, not by reachability.** | After deleting the enumerated files, check the survivors for callers: `rg` each module path outside its own directory. #4 removed exactly what its issue listed and still left `src/workspace` (139 lines, zero callers) plus `src/agent` and `src/types` (123 lines, reachable only through an event channel nothing publishes to). A delete list is a plan, not a proof. |
| **A stale baseline is copied between briefs.** | Compute the expected test count from the branch you are actually cutting, not from an earlier issue's brief. Two workers in this run had to reconcile a wrong baseline (34 quoted, 47 real); both were right and the brief was wrong. State the baseline as an observation, not a target — and if a worker reports a mismatch, check the brief before the code. |
| **Merge conflicts on shared files** | The file-ownership cap: concurrent issues must have disjoint `## Files`. |
| **Scope creep inside a worker** | `## Constraints` plus `AGENTS.md` ("keep the diff scoped; do not refactor neighbours"). A forced deviation is flagged and reviewed, not hidden. |
| **Stale inline cross-references** | Keep the authoritative pointer on `Blocked by:`; treat prose `(#n)` as hints. |
| **A blocker ships a different interface** | Update the dependent issue before dispatch. |

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

## 12. Adoption checklist for a new project

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
10. Compute the critical path and confirm the first dispatchable frontier.
11. Give the worker its own `git worktree`, dispatch, and require the report: command, raw
    output, SHA.
12. Re-run the acceptance independently, comment the result, and close.
