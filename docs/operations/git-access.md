# Git access isolation guarantees

`diff_engine::git::GitRepo` (DIFF-001) is the engine's only door to git objects. It is a
read-only wrapper around gix that opens a repository in isolation so that results are identical
on a developer laptop and on a worker.

## What is guaranteed

- **No git configuration is consulted.** The repository is opened with
  `gix::open::Options::isolated()`: no system, global or user config, no `include.path`
  chains, no `core.attributesFile`, no diff drivers. `core.autocrlf=false` and `core.fsmonitor=`
  are forced as config overrides, so object content is always raw repository bytes.
- **No git environment is consulted.** `GIT_DIR`, `GIT_WORK_TREE`, `GIT_CONFIG`,
  `GIT_CONFIG_GLOBAL`, `HOME` and `GIT_ALTERNATE_OBJECT_DIRECTORIES` have no effect; isolated
  open permissions disable the environment layer entirely.
- **No subprocess, hook, filter or textconv runs.** The module never spawns `git` (enforced by
  the `module_never_spawns_subprocesses` test) and never applies `.gitattributes`, filters,
  clean/smudge, LFS or external diff drivers.
- **No escape from the opened root.** A `.git` file (gitdir link or linked worktree) that
  resolves outside the opened directory is refused with `GitError::SymlinkEscape`, and so is an
  `objects/info/alternates` entry whose resolved location lies outside the root. Alternates
  inside the root (the usual shallow-fetch/mirror layout) keep working.
- **Typed failures, never guesses.** Missing objects report `GitError::ObjectNotFound`, or
  `GitError::ShallowBoundary` when the repository is shallow, so the caller can deepen the
  mirror (INIT/IDX-004 own fetching and credentials) instead of silently treating the boundary
  as the root. Over-size blobs fail with `GitError::BlobTooLarge` before inflation.

## Read limits

`ReadLimits` defaults: `max_blob_bytes = 8 MiB`, `max_tree_entries = 200_000`. The limits are
part of the handle (`GitRepo::limits()`) so a hostile or corrupt mirror cannot exhaust memory.

## Observability

| Signal | Name |
| --- | --- |
| Span (debug) | `git.read_blob` (`bytes`), `git.merge_base` (`candidates`) |
| Counter | `git_object_reads_total{kind=blob\|tree\|commit}` |
| Histogram | `git_read_blob_duration_seconds` |

## Not in scope here

Cloning, fetching, mirror maintenance and credentials (`repository::checkout`, INIT/IDX-004),
tree diffs (INC-001 `repository::git::tree_changes`) and anything beyond the merge base in
history walking.
