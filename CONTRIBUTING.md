# Contributing

Thanks for considering contributing to the YTM-CLI client!

## Repository Rules & Hygiene

To maintain the security and cleanliness of this repository:

1. **No Secrets/Credentials**: Never commit or push personal access keys, user credentials (`discord.json`, `cookies.txt`, `browser.txt`), passwords, or configuration files containing credentials.
2. **No Development-Only Trackers**: Do not commit internal bug audits, concurrency trackers, or personal task lists. These should stay ignored locally.
3. **No Private/Internal Documentation**: Public documentation must only cover user-facing setup, help guides, and contributing procedures.

## Getting Started

1. Fork and clone the repo
2. Ensure you have Rust installed: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
3. Build the project: `cargo build`
4. Run the tests: `cargo test`

## Code Style

- Target Rust 2021 Edition
- Run `cargo fmt` to format your code before committing
- Run `cargo clippy` to check for common linting issues and follow its recommendations

## Commit Messages

This project uses conventional commits:

```
type: short description

Optional body explaining the why, not the what.
```

Types: `feat`, `fix`, `refactor`, `perf`, `docs`, `chore`, `test`, `build`

## Pull Requests

- Keep PRs focused on one thing
- Make sure all tests and formatting checks pass before opening
- Link related issues if applicable
- Squash commits if the history is noisy

## Running Tests

```bash
# Full suite
cargo test
```

## License

By contributing, you agree that your contributions will be licensed under the MIT license.
