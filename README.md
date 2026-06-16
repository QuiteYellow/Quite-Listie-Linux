# Quite Listie - Linux (GNOME)

Native GNOME (GTK4 + libadwaita + libshumate) port of Quite Listie for managing
lists, with file-based collaboration and Nextcloud sync. Shares a platform-agnostic
Rust `core` crate with the Apple apps.

> Also available on Apple platforms: **[Quite Listie for iOS, iPadOS and macOS](https://github.com/QuiteYellow/Quite-Listie)** (SwiftUI). Both apps share the `.listie` file format and Nextcloud sync, so lists move between them.

## Features

### Two Ways to Store Lists
- **File-Based Lists** - shareable `.listie` files opened directly from the filesystem (local disk or any mounted file provider)
- **Nextcloud** - native Nextcloud sync over WebDAV, offline-first with background sync and automatic conflict resolution

### List Management
- Two-pane navigation with sidebar
- Favourite lists for quick access
- Custom list icons
- Folders grouped into sections across all storage sources
- Unchecked item counts

### Items & Organisation
- Add, edit, delete items with quantity tracking
- Colour-coded labels with automatic contrast adjustment for readability
- Markdown notes on any item
- Recurring items
- Bulk operations (mark all complete/active)

### Views
- **List view** - standard checklist
- **Map view** - per-list map showing items pinned to locations, with label filtering and tap-to-add
- **Global Locations view** - a single map aggregating every pinned item across all lists
- **Kanban board** - drag items between label columns
- **Markdown preview** - rendered markdown view of the list

### Import & Export
- **Markdown import** - paste any markdown checklist, intelligently merges with existing items
- **Markdown export** - share lists as readable text
- Deeplink support for sharing lists via URLs

### Collaboration & Sync
- File-based collaboration via any file service
- Nextcloud sync with ETag-based change detection and three-way merge
- Offline-first: writes to local cache immediately, uploads in background
- Automatic conflict resolution using item timestamps
- Recent changes view

### Reminders
- Set due-date reminders on individual items
- "Today" and "Scheduled" smart boxes in the sidebar
- Desktop notifications over D-Bus

### Locations & Maps
- Pin a location to any item by pasting a Google Maps or Apple Maps link, or using the location picker
- Per-list and global map views; markers inherit the item's label colour and symbol
- Filter map pins by label or toggle visibility of completed items

### Smart Features
- Recycle bin with auto-delete
- Quantity tracking with increment/decrement
- Welcome list with interactive tutorial
- File type association (open `.listie` files)

## Technical Details
- Built with GTK4, libadwaita, and libshumate (vector map tiles)
- Shared Rust `core` crate (model + sync engine + utilities), no GUI dependencies
- Nextcloud Login Flow v2 - browser-based sign-in, no app passwords required
- ETag-based sync with three-way merge for conflict resolution
- Desktop notifications raised over D-Bus (zbus)

## Requirements
- A Linux desktop with the GNOME runtime (Flatpak: `org.gnome.Platform` 50)
- For a source build: GTK4 >= 4.14, libadwaita >= 1.6, libshumate >= 1.0, and a Rust toolchain

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

### Flatpak

```sh
# Vendor the Rust crates for the offline build sandbox (regenerate when Cargo.lock changes):
./build-aux/flatpak-cargo-gen.sh
# Build and install:
flatpak-builder --user --install --force-clean .flatpak-builder/build build-aux/com.quiteyellow.QuiteListie.yaml
flatpak run com.quiteyellow.QuiteListie
```

## App icons (private submodule)

The app icons are copyrighted artwork and live in a separate private submodule at
`gnome/data/icons/` (Linux hicolor PNGs under `linux/hicolor/`). A clone without
access to that submodule will not have them; `meson` installs them from
`gnome/data/icons/linux/hicolor/<size>/apps/`, so to build a packaged copy supply
equivalent PNGs at those paths. A plain `cargo build` does not need them.

The Flatpak build (`build-aux/com.quiteyellow.QuiteListie.yaml`) sources the working
tree, so check the submodule out (or supply the PNGs) before `flatpak-builder` runs.

## License
GPL-3.0-or-later - see the Apple repo's [LICENSE](https://github.com/QuiteYellow/Quite-Listie/blob/main/LICENSE) for the full text.
