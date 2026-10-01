gnome.compile_resources(
  'bitlog',
  'bitlog.gresource.xml',
  gresource_bundle: true,
  source_dir: meson.project_build_root() / 'app' / 'ui',
  dependencies: blueprints,
  install: true,
  install_dir: pkgdatadir,
)

desktop_file = i18n.merge_file(
  input: '@0@.desktop.in'.format(app_id),
  output: '@0@.desktop'.format(app_id),
  type: 'desktop',
  po_dir: meson.project_source_root() / 'po',
  install: true,
  install_dir: get_option('datadir') / 'applications',
)

metainfo_file = i18n.merge_file(
  input: '@0@.metainfo.xml.in'.format(app_id),
  output: '@0@.metainfo.xml'.format(app_id),
  po_dir: meson.project_source_root() / 'po',
  install: true,
  install_dir: get_option('datadir') / 'metainfo',
)

install_data(
  '@0@.gschema.xml'.format(app_id),
  install_dir: get_option('datadir') / 'glib-2.0' / 'schemas',
)

install_subdir(
  'icons',
  install_dir: get_option('datadir'),
)

test(
  'validate-desktop',
  find_program('desktop-file-validate'),
  args: [desktop_file],
)

test(
  'validate-metainfo',
  find_program('appstreamcli'),
  # There is no homepage yet. Remove the override once bitlog.dev is online and linked here.
  args: [
    'validate', '--no-net', '--explain',
    '--override=url-homepage-missing=info',
    metainfo_file,
  ],
)

test(
  'validate-schema',
  find_program('glib-compile-schemas'),
  args: ['--strict', '--dry-run', meson.current_source_dir()],
)
