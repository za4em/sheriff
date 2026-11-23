# Sheriff

## Rust style

Run `python3 scripts/rust_style.py` from the repo root. It reads `src` and exits 1 when a rule below fails. Run `python3 scripts/rust_style.py --self-test` to check that the fixtures of the old mistakes still fail.

| Rule | What enforces it |
| --- | --- |
| Import a type. Do not write a path of two or more `::` outside a `use`. The check names the type to import. An enum variant keeps the variant and imports the type. | `python3 scripts/rust_style.py` |
| `assert!`, `assert_eq!`, and `assert_ne!` belong under `#[cfg(test)]`. | `python3 scripts/rust_style.py` |
| A function may contain `if`, `match`, and `for` / `while` / `loop`. It stays within 30 lines, counted from `fn` through the closing brace, when one of those sits inside another. | `python3 scripts/rust_style.py` |
| `run`, `screen`, and `Screen` are not names. `score_group` scores one group. `watch_pending` republishes stale decisions. `judge_window` asks the provider. `classify_windows` walks the windows. `WindowVerdict` is that answer. | `python3 scripts/rust_style.py` |

A vague name that is not in that list is a judgment. The check does not catch it.
