# CLAUDE.md

This file only serves one purpose: to tell Codex CLI where to find the project documentation.

All project architecture, coding standards, and development guidelines are documented in:

**AGENTS.md** - Located at the project root.

Please read AGENTS.md for:
- Project overview and directory structure
- Build and development commands
- Code style guidelines (TypeScript/RReact, Rust, Styling)
- Data storage architecture
- System tray menu integration patterns

## Local Tooling Notes

- Some host-level tools can fail differently inside the default sandbox because
  they depend on OS credentials, keyrings, user profiles, or network state that
  the sandbox cannot fully access.
- If a command that should be read-only and already configured on the host fails
  in the sandbox with an authentication, keyring, credential, permission, or
  network-style error, retry the same minimal diagnostic command outside the
  sandbox before asking the user to reconfigure the tool.
- Known example on this machine: `gh` may report `The token in default is
  invalid` or GitHub GraphQL `HTTP 401: Requires authentication` inside the
  sandbox, while the same `gh auth status -h github.com` and `gh repo view ...`
  commands succeed outside the sandbox by reading the Windows keyring.
