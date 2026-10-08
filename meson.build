project(
  'bitlog',
  version: '26.10.2',
  license: 'GPL-3.0-or-later',
  meson_version: '>= 1.5.0',
)

i18n = import('i18n')
gnome = import('gnome')

app_id = 'dev.bitlog.BitLog'
gettext_package = meson.project_name()
localedir = get_option('prefix') / get_option('localedir')
pkgdatadir = get_option('prefix') / get_option('datadir') / meson.project_name()

dependency('gtk4', version: '>= 4.22')
dependency('libadwaita-1', version: '>= 1.9')
dependency('gtksourceview-5', version: '>= 5.20')

cargo = find_program('cargo')
blueprint_compiler = find_program('blueprint-compiler')

# Cargo reads these at compile time, see app/src/config.rs.
cargo_target = meson.project_build_root() / 'target'
cargo_env = environment()
cargo_env.set('CARGO_TARGET_DIR', cargo_target)
cargo_env.set('MESON_APP_ID', app_id)
cargo_env.set('MESON_VERSION', meson.project_version())
cargo_env.set('MESON_GETTEXT_PACKAGE', gettext_package)
cargo_env.set('MESON_LOCALEDIR', localedir)
cargo_env.set('MESON_RESOURCES_FILE', pkgdatadir / 'bitlog.gresource')
meson.add_devenv(cargo_env)

subdir('app')
subdir('data')
subdir('po')

gnome.post_install(
  glib_compile_schemas: true,
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
