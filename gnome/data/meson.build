datadir = prefix / get_option('datadir')

# --- GSettings schema ---
install_data(
  app_id + '.gschema.xml',
  install_dir: datadir / 'glib-2.0' / 'schemas',
)

# --- Desktop entry ---
desktop_conf = configuration_data()
desktop_file = configure_file(
  input: app_id + '.desktop.in',
  output: app_id + '.desktop',
  configuration: desktop_conf,
  install: true,
  install_dir: datadir / 'applications',
)

if desktop_file_validate.found()
  test(
    'validate-desktop',
    desktop_file_validate,
    args: [desktop_file],
  )
endif

# --- AppStream metainfo ---
metainfo_file = app_id + '.metainfo.xml'
install_data(
  metainfo_file,
  install_dir: datadir / 'metainfo',
)

appstreamcli = find_program('appstreamcli', required: false)
if appstreamcli.found()
  test(
    'validate-metainfo',
    appstreamcli,
    args: ['validate', '--no-net', '--explain', meson.current_source_dir() / metainfo_file],
  )
endif

# --- D-Bus service (desktop entry is DBusActivatable) ---
service_conf = configuration_data()
service_conf.set('bindir', bindir)
configure_file(
  input: app_id + '.service.in',
  output: app_id + '.service',
  configuration: service_conf,
  install: true,
  install_dir: datadir / 'dbus-1' / 'services',
)

# --- App icons ---
# Raster hicolor icons. Copyrighted artwork supplied by the `gnome/data/icons`
# submodule (private); the Linux hicolor set lives under its `linux/` subtree.
# A clone without access to that submodule must drop equivalent PNGs there.
foreach size : ['16', '32', '48', '64', '128', '256', '512']
  install_data(
    'icons' / 'linux' / 'hicolor' / (size + 'x' + size) / 'apps' / (app_id + '.png'),
    install_dir: datadir / 'icons' / 'hicolor' / (size + 'x' + size) / 'apps',
  )
endforeach
