# Side-quests: changing the provider stack or the mock server

Courly leans on two bodies of code that are not courly proper:

- **conduit** - the provider stack. All mail, calendar, and contact protocol
  I/O, the unified `Account` surface, `AccountError` / `RecoveryClass`, and the
  sync engine. It lives in this repository under `crates/conduit/`, as a nested
  Cargo workspace that the root manifest excludes and reaches through path
  dependencies.
- **saehrimnir** - the mock provider server the sync harness runs against. A
  sibling repository and an installed binary, not a Cargo dependency.

When work here surfaces a need to change one of them - a missing capability, an
awkward surface, a wart that would otherwise force a workaround on our side -
that is a **side-quest**. This document is the procedure.

## The governing rule

Conduit and the mock exist to serve courly. Where their shape is sub-optimal
for us, they are fixed FIRST, before the courly work that needs it.

Courly is never contorted around a provider-stack wart. Provider-reality
differences (Gmail has no separate attachment-upload endpoint, IMAP has no
native send) are absorbed by conduit behind the uniform `Account` surface, or
expressed as `AccountCapabilities` flags we read declaratively. They never leak
in here as per-provider special-cases. A genuinely immutable provider limit
becomes a capability flag the UI consults, not a code branch.

Corollary worth stating because it comes up often: if a capability looks
mysteriously absent from conduit, that is usually not a design gap to engineer
around. Conduit was started by ripping courly's own provider code out and
unifying it, so an absence generally means that part was not carried over yet.
Git history is the reference for what it should already do.

## Conduit: edited in place

Conduit used to be a sibling repository, promoted into the build through a
research copy and a bridge script. It was merged into this repository with its
history, so none of that machinery remains:

- Edit it under `crates/conduit/`, following `crates/conduit/AGENTS.md` and the
  per-crate sheets in `crates/conduit/reference/` (`net.md`, `sync.md`,
  `error-model.md`, `jmap.md`, `imap.md`, `graph.md`, `google.md`, `smtp.md`,
  `caldav.md`, `carddav.md`, `sasl.md`). Start with the sheet for a crate's
  surface, then drop into the source.
- Commit the conduit change with the courly change that needs it, in this
  repository. There is nothing to push elsewhere and nothing to promote.
- It keeps its own workspace: its own `Cargo.toml`, lints, `brokkr.toml`, check
  sweeps and `Cargo.lock`. Validate a conduit change from inside
  `crates/conduit/`, then validate courly from the repository root. The app
  itself resolves conduit's dependencies through the root `Cargo.lock`; the
  nested one only pins conduit's own test runs.

The boundary that conduit is a SERVICE-side dependency is unchanged and still
machine-enforced by the `app-no-conduit` / `core-no-conduit` rules in
`brokkr.toml`.

## A conduit change is not done when its own tests pass

**It is done when courly's own harness gates pass against it.** This is the
single most expensive lesson of the provider migration and it has a specific
cause: conduit's own rules forbid integration and mock-server tests, so a
conduit change can be unit-green and wire-broken. One item hit that four
separate times - a LIST form a conforming server answers with the wrong
namespace, a hydration door that routed in batch but not singly (twice, once per
protocol crate), and a discovery seed gated on an optional chain. A fifth case
needed a live mailbox to show at all: a delta request Graph answers with HTTP
400 passed every hermetic test.

Two mitigations, both load-bearing:

- Extract each wiring decision into a pure, unit-pinnable function rather than
  leaving it inline.
- Prefer several narrow work items to one broad one.

The same failure shape recurs on the mock side, where it is even easier to miss:
a mock can serve a surface in the wrong SHAPE and every test still passes,
because the tests exercise the shape the mock implements rather than the one the
client drives. Assert the consumer's actual request form, and assert response
STATUS on batched sub-requests.

## Saehrimnir: where the repo lives

Saehrimnir exists in two places, serving two distinct purposes. Do not conflate
them.

| Path | Role |
| --- | --- |
| `./research/saehrimnir` | In-tree working copy. The reading reference AND the staging area where side-quest edits are made. |
| `../saehrimnir` | The live dependency. What `cargo install` builds the mock from. |

Keeping a copy in-tree is what lets agents read and edit the mock's source
without tripping up the harness.

## Reading is expected; writing is the prohibition

Side-quest work on the mock is never delegated to an implement run. Codex and
other implement agents must not EDIT `../saehrimnir` or the `./research/` copy -
even when the spec names the side-quest surface as a required prerequisite - and
every implement prompt that runs while a side-quest is pending states this
prohibition explicitly.

**The prohibition is on WRITES, and the distinction is load-bearing.** Reading
those trees is expected and often necessary. Phrase the guard as "must not
edit", never as "must not read or touch".

Both halves were learned the hard way. First an implement run edited a frozen
dependency checkout directly, which is what prompted the rule. Then the next run
stalled mid-brick, unable to resolve an accessor on a freshly-promoted surface,
because the guard had been over-tightened to forbid reading as well. Those are
opposite errors around the same boundary.

## The saehrimnir procedure

### 1. One agent, confined

Launch ONE Opus agent (the Agent tool, never codex) to do the work. Its prompt
must state, in unambiguous terms:

- It works EXCLUSIVELY inside `./research/saehrimnir`. It must not read, edit,
  or otherwise touch any part of courly proper, under any circumstance. If it
  finds itself blocked on something that would require a courly change, it
  STOPS WORK immediately and reports back - it never improvises a courly
  edit.
- It must `cd` into `./research/saehrimnir` before doing anything, and stay
  there. (A guardrail, not a wall - the fence is the instruction above, so
  state it plainly.)
- It must NOT commit. Committing is the orchestrator's job.
- It is told NOTHING about the bridge script and must never run it. Promotion
  is the orchestrator's job.
- It must not launch any sub-agents.
- It is doing a DIRECT implementation task, not an orchestration. The research
  copy's CLAUDE.md leads with a spec-loop cue ("when asked to orchestrate, read
  reference/orchestrate.md FIRST"); that cue is not for this agent. It must not
  orchestrate, must not read that repo's orchestrate.md, and must ignore the
  spec-loop machinery - it does the work itself, in place.

### 2. Review, validate, commit, promote - in that order

The orchestrator does all four, without pausing for the user:

1. **Review** the work in `./research/saehrimnir`.
2. **Validate it IN PLACE**: `cd` into `./research/saehrimnir` and run
   `brokkr check` (plus any focused `brokkr test`). This is the gate, and it
   runs BEFORE the commit.
3. **Commit it there.** The commit is the orchestrator's job, never the
   agent's.
4. **Promote** by running `bash scripts/saehrimnir.sh` from the main session.
   It pushes the staged commit to its shared remote, pulls it into the
   dependency path, and reinstalls the mock binary.

The bridge script round-trips through GitHub, so it is **orchestrator-only**
and can never run inside a codex step - that sandbox is network-isolated. The
push and the reinstall are routine steps needing no separate approval.

Bridge-script assumptions to hold: the work is already committed in
`./research/saehrimnir`, and that clone is on a branch tracking origin (not a
detached HEAD), so the script's bare `git push` succeeds.

### 3. Validate courly against the reinstalled mock

`saehrimnir.sh`'s `cargo install` compile-gates the mock; the behavioral gate is
a courly sync-harness run against the reinstalled binary.

## Freeze discipline

Whatever commit the bridge reports becomes the frozen reference for the work
item, and the mock stays pinned there for the item's full duration - including
the hours a single implement run can take.

## Working in the research copy

The repo's bash rules (no `git -C`, one command per invocation, no chaining) are
written for courly proper. `./research/saehrimnir` is a separate repo the
orchestrator legitimately manages, so the orchestrator is exempt there for the
review / validate / commit / discard it owns:

- **Git**: run `git -C <abs-path>/research/saehrimnir ...` directly. The Bash
  working directory also persists between calls, so a bare
  `cd ./research/saehrimnir` followed by a separate `git` / `brokkr` command
  works equally well - just `cd` back to the courly root afterward, since later
  steps assume it.
- **Validate in place**: this works because the research repo is its own
  standalone Cargo workspace root that brokkr resolves instead of walking up
  into courly's. It needed a bare `[workspace]` table in its `Cargo.toml` so
  cargo would not adopt the parent manifest.
- **The `CARGO_MANIFEST_DIR` gotcha**: brokkr builds a nested workspace with
  `CARGO_MANIFEST_DIR` anchored under brokkr's OWN install path, not the real
  source location. Any test resolving a committed fixture via
  `env!("CARGO_MANIFEST_DIR")` therefore reads a path that does not exist and
  fails. Such tests must resolve fixtures against the runtime working directory
  (`std::env::current_dir()`), which is the crate root under both `cargo test`
  and brokkr. The same applies to `crates/conduit/`, which is also a nested
  workspace.
