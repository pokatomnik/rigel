# Terminal

This module contains domain-neutral terminal presentation infrastructure shared
by asynchronous workflows. `loader.rs` provides the spinner guard and
`WithLoader` future extension used to show bounded work on interactive `stderr`
without changing the wrapped operation's result.

The loader is hidden for tests, redirected output, and non-interactive stderr.
Interactive command parsing and concrete terminal I/O belong to controller runtime
adapters rather than this shared module.
