# Quite Listie (GNOME)

Native GNOME (GTK4 + libadwaita + libshumate) port of the Quite Listie list app,
sharing a platform-agnostic Rust `core` crate. Manages `.listie` files with
Nextcloud WebDAV sync.

## Workspace

- `core/` - `quite-listie-core`: model + engine + util, no GUI deps.
- `gnome/` - `quite-listie`: the GTK4 front end (the app).

## Build

Clone with submodules (or `git submodule update --init` after cloning) if you have
access to the private asset submodules; see [App icons](#app-icons-private-submodule).

```sh
cargo build -p quite-listie
mkdir -p target/gsettings
cp gnome/data/com.quiteyellow.QuiteListie.gschema.xml target/gsettings/
glib-compile-schemas target/gsettings/
GSETTINGS_SCHEMA_DIR="$PWD/target/gsettings" ./target/debug/quite-listie
```

## App icons (private submodule)

The app icons are copyrighted artwork and live in a separate private submodule at
`gnome/data/icons/` (Linux hicolor PNGs under `linux/hicolor/`). A clone without
access to that submodule will not have them; `meson` installs them from
`gnome/data/icons/linux/hicolor/<size>/apps/`, so to build a packaged copy supply
equivalent PNGs at those paths. A plain `cargo build` does not need them.

The Flatpak build (`build-aux/com.quiteyellow.QuiteListie.yaml`) sources the working
tree, so check the submodule out (or supply the PNGs) before `flatpak-builder` runs.
