# The outputs are listed file by file, so that Meson rebuilds the resources
# whenever a blueprint changes.
blueprints = custom_target(
  'blueprints',
  input: files('day_view.blp', 'window.blp'),
  output: ['day_view.ui', 'window.ui'],
  command: [blueprint_compiler, 'batch-compile', '@OUTDIR@', '@CURRENT_SOURCE_DIR@', '@INPUT@'],
)
