#!/bin/sh
# Post-install: compile the GSettings schema and refresh caches, unless the
# packaging system handles it (e.g. distro RPM post scripts).
set -e

if [ -z "${DESTDIR}" ]; then
    schemadir="${MESON_INSTALL_PREFIX}/share/glib-2.0/schemas"
    echo "Compiling GSettings schemas in ${schemadir}…"
    glib-compile-schemas "${schemadir}" || true

    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "${MESON_INSTALL_PREFIX}/share/applications" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -qtf "${MESON_INSTALL_PREFIX}/share/icons/hicolor" || true
    fi
fi
