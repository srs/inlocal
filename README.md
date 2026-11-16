# inlocal

A local, open-source translation editor for [inlang](https://inlang.com) projects.

Run it in a repo with a `project.inlang` folder. It reads and writes your
translation files directly: no account, no sync, no cloud. You commit the
changes with git as usual.

> Unofficial. Not affiliated with inlang or Opral.

## Front ends

| | Command | Built with |
| --- | --- | --- |
| Terminal | `inlocal` (or `inlocal tui`) | ratatui |
| Browser | `inlocal serve` | React |

Both share the same core, so they find the same issues and write files the
same way:

- Only edited values change on disk. Every other byte of the file is preserved.
- Checks for missing translations, missing plural forms and broken `{variables}`.
- Live reload when files change on disk. Unsaved work is never overwritten.

Supported formats (read from `project.inlang/settings.json`):

| Plugin setting | Layout |
| --- | --- |
| `plugin.inlang.messageFormat` (Paraglide JS) | flat keys, `{var}`, variants under `match` |
| `plugin.inlang.i18next` | nested keys, `{{var}}`, `_one`/`_other` plurals, namespaces |
| any other `plugin.*` with a `pathPattern` | nested keys, `{var}` |

## Command line

```sh
inlocal                          # open the terminal editor
inlocal check                    # list issues; exits 1 if there are any
inlocal check -l de -l sv        # only some locales
inlocal check --format github    # annotations on pull requests in GitHub Actions
inlocal check --format json
inlocal stats                    # coverage per locale and namespace
inlocal missing de               # missing message ids for a locale, one per line
inlocal info                     # detected format, locales and files
inlocal -p path/to/project       # use a project somewhere else
```

### In CI

```yaml
- run: cargo install --git https://github.com/srs/inlocal inlocal
- run: inlocal check --format github
```

## Terminal editor keys

| Where | Keys |
| --- | --- |
| List | `j`/`k` move, `enter` edit, `/` search, `1`–`4` filter, `n` namespace, `v` compare view, `q` quit |
| Compare | `f`/`t` pick languages, `w` swap, `m` only missing, `c` copy from the left column |
| Fields | `j`/`k` field, `J`/`K` next/previous message, `enter` edit, `c` copy from source, `x` clear, `esc` back |
| Editing | `enter` keep, `esc` cancel, `alt+enter` new line, `ctrl+u` clear |
| Anywhere | `ctrl+s` save, `u` undo all unsaved edits, `r` reload from disk |

## Development

Requirements: Rust (stable), and Bun (or Node) for the web UI.

```sh
# Build the web UI once; it is embedded in the binary.
cd ui && bun install && bun run build && cd ..

cargo run -- -p examples/paraglide             # terminal editor
cargo run -- serve -p examples/paraglide       # web UI
cargo test

cargo install --path .                         # put `inlocal` on your PATH
```

### Features

The web UI is a Cargo feature, on by default. It adds about 1.6 MB to a
3.8 MB release binary.

```sh
cargo build --release --no-default-features   # CLI and terminal editor only
```

### Layout

```
src/
  core/               shared by every front end
    project.rs          settings.json, formats, reading and writing messages
    jsonedit.rs         format-preserving JSON editor
    checks.rs           lint rules
    session.rs          unsaved edits, issues and filters for editors
  cli/                one module per command
  tui/                ratatui terminal editor
  server.rs           web UI backend (feature `web`)
  browser.rs          reuses an open browser tab on macOS
ui/                 React web UI
examples/           sample Paraglide and i18next projects
```

### Security

The web server binds to `127.0.0.1` only, rejects requests with a foreign
`Host` header, and requires an `x-inlocal` header on writes, so other
websites in your browser can't modify your files.

## License

Apache License 2.0. See [LICENSE](LICENSE).
