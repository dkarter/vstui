# vstui

`vstui` is a fast Ratatui-based terminal UI for viewing and managing macOS audio plugins.

## Features

- Scans standard macOS VST and Logic Pro Audio Unit locations:
  - `/Library/Audio/Plug-Ins/VST`
  - `/Library/Audio/Plug-Ins/VST3`
  - `/Library/Audio/Plug-Ins/Components`
  - `~/Library/Audio/Plug-Ins/VST`
  - `~/Library/Audio/Plug-Ins/VST3`
  - `~/Library/Audio/Plug-Ins/Components`
- Loads from a JSON cache on startup for fast launch.
- Refreshes manually with `r`.
- Refreshes automatically when known plugin directory mtimes change, including after uninstalling a plugin.
- Deletes either the selected bundle or every related VST2/VST3/AU bundle with confirmation.

## Controls

- `j` / `k`: move in the plugin list
- `Ctrl-d` / `Ctrl-u`: scroll the details pane
- `r`: refresh cache
- `d`: delete selected plugin bundle
- `D`: delete all related plugin bundles
- `q` / `Esc`: quit
- `y` / `n`: confirm or cancel deletion

System plugins under `/Library` may require elevated permissions to delete.

## Development

Run the app:

```bash
cargo run
```

Check the Rust build:

```bash
cargo check
```

Format Rust code:

```bash
cargo fmt
```
