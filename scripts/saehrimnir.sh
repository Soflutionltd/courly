#!/usr/bin/env sh
# Bridge: promote saehrimnir (mock-server) work staged in research/saehrimnir
# to the installed mock binary the sync-harness runs against.
#
# The spec-loop edits saehrimnir ONLY in ratatoskr/research/saehrimnir - inside
# the harness root, so Opus agents (and the orchestrator) can write there.
# research/saehrimnir and ../sæhrimnir are two clones of the same repo
# (github.com/folknor/saehrimnir). This script pushes the staged commit from
# research/saehrimnir, pulls it into ../sæhrimnir, then reinstalls the
# saehrimnir binary so brokkr's sync-harness picks up the change.
#
# Orchestrator-only. It round-trips through GitHub (network), so it can NEVER
# run inside a codex step (codex's sandbox is network-isolated). Committing in
# research/saehrimnir is also the orchestrator's job; this script assumes the
# work is already committed there.
#
# The install goes through `brokkr install`, run from the saehrimnir checkout so
# it resolves that tree's own config. A raw `cargo install` does not survive
# brokkr's machine-wide rustc guard: the guard refuses a compile brokkr did not
# start, and any brokkr run in another project SIGKILLs it as a stray cargo.
# `brokkr install` also passes --locked, so the mock is built from the lockfile
# its own checks validated rather than a fresh re-resolve.
set -e

cd /home/folk/Programs/ratatoskr/research/saehrimnir
git push

cd /home/folk/Programs/sæhrimnir
git pull

brokkr install --release

echo "saehrimnir reinstalled at:"
git rev-parse HEAD
