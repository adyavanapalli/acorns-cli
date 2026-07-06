<div align="center">
  <img src="assets/logo.png" alt="acorns-cli logo" width="160">
  <h1>acorns-cli</h1>
  <p><em>An unofficial command-line tool for managing your Acorns account.</em></p>
</div>

---

`acorns` talks to Acorns' private GraphQL API so you can check balances, move money,
manage round-ups, and pull tax documents, all from your terminal. Output is JSON, so
it pipes cleanly into `jq`.

> [!WARNING]
> **Unofficial and unaffiliated.** This project is not associated with Acorns in any
> way. It works by calling Acorns' private, undocumented API, which very likely
> violates their Terms of Use. It is provided for **educational purposes only**, with
> no warranty of any kind. Use it entirely **at your own risk**, including the
> possibility of account suspension or loss of funds.

## Install

```bash
cargo install --path .
```

## Usage

```console
$ acorns --help
Manage your Acorns account from the terminal

Usage: acorns [OPTIONS] <COMMAND>

Commands:
  auth         Authentication & session
  account      Account overview
  invest       Investing: balance, performance, deposits, round-ups, portfolio
  funding      Funding source & linked bank(s)
  tax          Tax documents & statements
  raw          Escape hatch: run any cataloged GraphQL operation by name
  completions  Generate a shell completion script (e.g. `acorns completions bash`)
  help         Print this message or the help of the given subcommand(s)

Options:
  -y, --yes      Skip confirmation prompts (money/destructive ops)
      --dry-run  Print the GraphQL request that would be sent; do not execute
  -v, --verbose  Verbose: log operation name, variables, timing
  -h, --help     Print help
  -V, --version  Print version
```

Run `acorns <command> --help` for a group's subcommands. Your session is stored
locally at `~/.config/acorns-cli/session.json` (chmod `600`).
</content>
