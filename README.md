# RiProxy GUI

`riproxy-gui` is an egui desktop frontend for Leaf and Rog backends.

## Build

Default build includes both backends:

```sh
cargo build --release
```

Backend-specific builds:

```sh
cargo build --release --no-default-features --features backend-leaf
cargo build --release --no-default-features --features backend-rog
cargo build --release --no-default-features --features backend-leaf,backend-rog
```

## Runtime Config

Proxy profile config files are stored under:

```text
~/.config/riproxy/
```

The default profile uses `config.conf` for Leaf and `config.toml` for Rog. Other profiles use
`<profile>.config.conf` or `<profile>.config.toml`.

## Release Packages

On macOS, extract the release archive and move `RiProxy.app` to `/Applications`.
Launching the `.app` opens the GUI directly instead of starting a Terminal window.

## Linux Notes

The system tray build uses GTK/AppIndicator. Debian/Ubuntu systems need:

```sh
sudo apt install libgtk-3-dev libxdo-dev libayatana-appindicator3-dev
```
