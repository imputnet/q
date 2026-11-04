# p2

p2 is a reimplementation of GNU `patch`, usable both as a command-line tool and
as a Rust library. q uses the library to apply patches in-process instead of
running `patch` for each one.

## Testing

p2 aims to behave exactly like GNU patch. `tests/parity.rs` runs GNU `patch`
and p2 on the same input and checks that the exit status and resulting files
match; new behavior should get a case there.

`scripts/compat.sh` runs wider checks. Run `scripts/compat.sh --help` to see
the environment variables it needs.

| Mode | What it runs |
| --- | --- |
| `gnu-tests [TEST...]` | GNU patch's own test suite against p2 |
| `p2-fixtures` | the parity fixtures against GNU patch and p2 |
| `p2-fixtures-on-patch` | the parity fixtures with GNU patch as the implementation, to check the fixtures themselves |
| `q-tests-against-p2` | q's test suite with p2 standing in for `patch` |
| `all` | both fixture directions and the GNU suite |

## Library

```rust
let parsed = p2::parse_apply_options(args)?; // patch-style arguments
let patch = p2::parse_patch(&input)?;
let report = p2::apply_patch(root, &patch, &parsed.options)?;
```

The returned `ApplyReport` describes what happened to each file and hunk.
Callers should use it rather than parsing printed messages.

## Limitations

p2 does not support binary patches, RCS/SCCS retrieval (`--get`), interactive
prompts or debug tracing. Some cases of fuzzy matching, context and normal
diffs, merging, timestamp formats, backup naming and reject formatting are not
handled yet, and some diagnostic messages differ from GNU patch's.
