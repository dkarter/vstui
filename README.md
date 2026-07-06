# vstui

`vstui` is a fast Ratatui-based terminal UI for viewing and managing macOS audio plugins.

<img width="3220" height="2048" alt="Image" src="https://github.com/user-attachments/assets/46d4c7e3-eda8-4768-bd0b-abbb351b41f9" />

<img width="3222" height="2056" alt="Image" src="https://github.com/user-attachments/assets/da3ef7e9-fac9-4e77-a91b-e86e3caa96c2" />

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
