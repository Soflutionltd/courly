# Vendored sources

Code Courly builds from source rather than from crates.io. Each directory
keeps its own license files, which must stay with the code.

| Directory | Copied from | Commit |
|---|---|---|
| `iced/` | `Soflutionltd/iced`, branch `sluggrs` | `9b74a21dadb0359f32ee3f9e6143b960530476a3` |
| `sluggrs/` | `Soflutionltd/sluggrs`, branch `courly` | `b51be5c7044c801182cafd702b831a9f3edf708c` |
| `blitz-dom/` | see its own `Cargo.toml` | - |

The copies are plain snapshots without history. The only local edit to
`iced/` is the path of its `sluggrs` dependency (`../sluggrs`); `sluggrs/`
drops its own working notes and agent instructions.

## Taking a newer upstream version

1. Clone the source repository outside this one and check out the new commit.
2. Produce the change between the commit recorded above and the new one:
   `git diff <recorded commit> <new commit> > update.patch`.
3. Apply it here with `git apply --directory=vendor/<dir> update.patch`,
   resolve any conflict with local edits, build, and run the checks.
4. Record the new commit in this table in the same commit.
