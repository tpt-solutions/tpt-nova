# Contributing to TPT Nova

Thanks for your interest in TPT Nova! Contributions are accepted **as GitHub
issues only**. Please do not open pull requests; they will not be reviewed or
merged.

## Reporting a bug

Include:

- What you did, what you expected, and what happened instead.
- Your OS, GPU/driver, and the Rust toolchain version (`rustc --version`).
- The crate or example involved (see the crate table in `README.md`).
- Logs or telemetry output, if available.

## Suggesting a feature

Describe the problem you are trying to solve before the solution. Mention which
part of the engine it touches, and whether it fits the current roadmap in
`todo.md`.

## Security issues

If an issue involves untrusted input (the agent control file in
`tpt-nova-agent-api`, Rhai scripts in `tpt-nova-scripting-embedded`, or `.novapack`
archives in `tpt-nova-export`), say so in the issue title so it can be triaged
quickly. Do not include working exploit payloads in public issues.

## License

TPT Nova is dual-licensed under the MIT License or the Apache License, Version
2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
