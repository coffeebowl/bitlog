subdir('ui')

cargo_options = [
  '--manifest-path', meson.project_source_root() / 'Cargo.toml',
  '--package', 'knotbook-gtk',
]
if get_option('buildtype') == 'release'
  cargo_options += ['--release']
  rust_target = 'release'
else
  rust_target = 'debug'
endif

custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: 'knotbook-gtk',
  console: true,
  install: true,
  install_dir: get_option('bindir'),
  env: cargo_env,
  command: [
    cargo, 'build', cargo_options,
    '&&',
    'cp', cargo_target / rust_target / 'knotbook-gtk', '@OUTPUT@',
  ],
)
