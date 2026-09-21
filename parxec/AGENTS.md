# Coding guidelines

- Functions shorter than 30 lines must not contain blank lines.
- Do not put blank lines between `use` imports.
- For fallible backend work, prefer `anyhow::Result<T>` and add useful context with `.context(...)`. Always qualify the result type as `anyhow::Result`.
- Handle recoverable runtime errors explicitly. Reserve `unwrap` and `expect` for app bootstrap invariants, unrecoverable failures, unreachable states, and programming bugs.
- Prefer declaring constants at the top level of a file.
- Prefer defining code before its first use.
- Keep related code close together in the same file.
