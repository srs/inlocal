# inlocal

A local, open-source translation editor for [inlang](https://inlang.com) projects.

Run it in a repo with a `project.inlang` folder and it opens a fast, keyboard-first
editor in your browser. It reads and writes your translation files directly: no
account, no sync, no cloud. You commit the changes with git as usual.

> Unofficial. Not affiliated with inlang or Opral.

## Features

- Focus view: one message with every locale and plural form
- Compare view: two languages side by side (for example `en` → `de`), with "only missing" filtering, or all locales as a grid
- Checks for missing translations, missing plural forms and broken `{variables}`, with quick fixes
- Clean diffs: only edited values change, every other byte of the file is preserved
- Live reload when files change on disk
- `inlocal check` for CI (exits 1 on issues)

Supported formats (read from `project.inlang/settings.json`):

| Plugin setting | Layout |
| --- | --- |
| `plugin.inlang.messageFormat` (Paraglide JS) | flat keys, `{var}`, variants under `match` |
| `plugin.inlang.i18next` | nested keys, `{{var}}`, `_one`/`_other` plurals, namespaces |
| any other `plugin.*` with a `pathPattern` | nested keys, `{var}` |

## Usage

```sh
inlocal                     # find project.inlang near the current folder and open the editor
inlocal serve --port 4321 --no-open
inlocal check               # print issues and translation coverage
inlocal check --json
inlocal -p path/to/project.inlang
```

Shortcuts: `⌘K` search, `⌘S` save, `⌘\` switch between focus and compare view, `↑`/`↓` or `j`/`k` to move between messages.

## Development

Requirements: Rust (stable) and [Bun](https://bun.sh) (or Node).

```sh
# Build the UI once; it is embedded in the binary.
cd ui && bun install && bun run build && cd ..

cargo run -- -p examples/paraglide
cargo test
```

For UI work with hot reload, run the server and Vite side by side:

```sh
cargo run -- -p examples/paraglide --no-open   # API on :4321
cd ui && bun run dev                           # UI on :5173, proxies /api
```

If you build the Rust side before the UI, the binary serves a placeholder page
telling you to build the UI.

### Layout

```
src/main.rs       CLI (serve, check)
src/project.rs    settings.json, format detection, reading and writing messages
src/jsonedit.rs   format-preserving JSON editor (span-tracking parser)
src/checks.rs     lint rules
src/server.rs     axum API, file watcher (SSE), embedded UI
ui/               React + Vite UI
examples/         sample Paraglide and i18next projects
```

### Security

The server binds to `127.0.0.1` only, rejects requests with a foreign `Host`
header, and requires an `x-inlocal` header on writes so other websites in your
browser can't modify your files.

## License

MIT
