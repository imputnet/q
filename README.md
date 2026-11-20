# q

q is a fast implementation of [quilt](https://savannah.nongnu.org/projects/quilt).
It has the same commands and works on the same patches and `.pc/` state, so you
can switch between q and quilt in the same tree. Common stack operations run
between 5 and 85 times faster than quilt.

Binaries for x86_64 Linux, arm64 macOS and x86_64 Windows are published daily
when main changes. On Linux:

```sh
curl -fLo ~/.local/bin/q https://github.com/imputnet/q/releases/latest/download/q-x86_64-linux
chmod +x ~/.local/bin/q
ln -sf ~/.local/bin/{q,quilt}
```

On macOS, use `q-arm64-macos` instead. On Windows, download
[`q-x86_64-windows.exe`](https://github.com/imputnet/q/releases/latest/download/q-x86_64-windows.exe);
q runs `diff` and `patch`, so it needs them on `PATH`, for example from Git for
Windows.

With a Rust toolchain, you can build the latest commit instead:

```sh
cargo install --git https://github.com/imputnet/q --bin q
```

To use q in place of quilt, symlink `quilt` to it, or add `--bin quilt` to the
`cargo install` command.

## Speed

The table below compares q with quilt 0.68 on a stack of 80 patches over 200
files, where each patch changes 3 files. It was measured with hyperfine on a
Ryzen 9 9950X; `tools/benchmark.sh` reproduces it.

| Command | q | quilt | Speedup |
| --- | ---: | ---: | ---: |
| `push -a` | 20.8 ms | 492.6 ms | 24x |
| `pop -a` | 20.7 ms | 1.766 s | 85x |
| `refresh` | 2.4 ms | 43.5 ms | 18x |
| `push -a --refresh` | 76.4 ms | 3.869 s | 51x |
| `pop -a --refresh` | 60.0 ms | 5.086 s | 85x |
| `fold` | 1.9 ms | 8.3 ms | 4x |

How much faster q is on your patches depends on how many patches and files
you have and how large they are.

## Compatibility

q passes upstream quilt's own test suite, which `tools/run-upstream-tests.sh`
runs. q's tests also run against real quilt with `tools/run-compat-tests.sh`,
to check that they describe quilt's behavior.

q applies patches in-process using [p2](p2/), a reimplementation of GNU patch.
When p2 can't apply a patch cleanly, q falls back to GNU patch, so error
messages and rejects look the same as with quilt.

## What q adds

### `q absorb`

`q absorb` writes staged Git changes into the top patch. This lets you edit
files first and then pick the hunks that belong in the patch with
`git add -p`:

```sh
q new feature.patch
$EDITOR file.cc
git add -p file.cc
q absorb
```

Only staged changes are absorbed. Unstaged changes stay in the working tree,
and the absorbed paths are unstaged afterwards. If the top patch doesn't track
a file yet, q backs it up from the result of an earlier patch, from `HEAD`, or
as an empty file if the file is new.

Modified, added and deleted files are supported. Renames, copies, and files
under `patches/` or `.pc/` are refused. The Git repository's root must be the
same directory as the quilt root.

### Scratch repositories

When working in a throwaway tree, such as an unpacked tarball,
`q scratch init` creates a Git repository with a `base` commit and marks it
with `q.scratch=true`. In such a repository, `q absorb` also amends `HEAD`
so that `git status` stays clean.

Because amending rewrites history, an existing repository has to opt in with
`q scratch init --force`, which only sets the flag. `q absorb --no-sync-git`
skips the amend.

### Shell completion

```sh
source <(q completion bash)    # or zsh, fish
q completion fish | source
```

Completion covers command names and patch names. A patch can also be completed
by its basename when that is unique, so `platform/fix-build.patch` completes
from `fix-build`. Commands accept these basenames as well, and `q new`
completes existing patch directories.

> [!NOTE]
> Distributions may opt to ship the shell files (available in `src/completion`)
> separately to install them in the shells with no setup required.
> In that case, the `q completion` command should then be disabled via the
> `completion-scripts` Cargo feature flag to save on binary size.

### Other differences

- `q graph --lines` connects patches that change overlapping lines, rather
  than any patches that share a file.
- On a terminal, listing commands such as `series`, `diff` and `files` page
  their output through `$QUILT_PAGER`, `$GIT_PAGER`, `$PAGER` or `less -R`.
- q keeps some state of its own in `.pc/.q/`, which you shouldn't edit.
- The `quilt` binary is the same program as `q`, for tools that run `quilt`.
