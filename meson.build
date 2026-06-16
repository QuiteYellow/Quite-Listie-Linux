project(
  'quite-listie',
  version: '0.1.0',
  meson_version: '>= 1.0.0',
  license: 'GPL-3.0-or-later',
)

app_id = 'com.quiteyellow.QuiteListie'

dependency('gtk4', version: '>= 4.14')
dependency('libadwaita-1', version: '>= 1.6')
dependency('shumate-1.0', version: '>= 1.0')

cargo = find_program('cargo', required: true)
glib_compile_schemas = find_program('glib-compile-schemas', required: true)
desktop_file_validate = find_program('desktop-file-validate', required: false)

prefix = get_option('prefix')
bindir = prefix / get_option('bindir')

subdir('gnome/data')

# Build mode → cargo profile + target dir.
cargo_target_dir = meson.project_build_root() / 'cargo-target'
if get_option('buildtype') == 'release'
  cargo_profile_args = ['--release']
  cargo_out_dir = cargo_target_dir / 'release'
else
  cargo_profile_args = []
  cargo_out_dir = cargo_target_dir / 'debug'
endif

# Drive cargo from meson. The whole workspace lives at the source root; we build
# only the `quite-listie` binary (the GNOME front-end).
cargo_build = custom_target(
  'cargo-build',
  build_always_stale: true,
  output: 'quite-listie',
  console: true,
  install: true,
  install_dir: bindir,
  command: [
    find_program('build-aux/cargo-build.sh'),
    cargo,
    meson.project_source_root() / 'Cargo.toml',
    cargo_target_dir,
    cargo_out_dir,
    '@OUTPUT@',
    cargo_profile_args,
  ],
)

meson.add_install_script('build-aux/meson-postinstall.sh')
