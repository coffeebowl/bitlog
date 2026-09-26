# The outputs are listed file by file, so that Meson rebuilds the resources
# whenever a blueprint changes.
blueprints = custom_target(
  'blueprints',
  input: files('calendar_view.blp', 'day_view.blp', 'open_tasks.blp', 'window.blp'),
  output: ['calendar_view.ui', 'day_view.ui', 'open_tasks.ui', 'window.ui'],
  command: [blueprint_compiler, 'batch-compile', '@OUTDIR@', '@CURRENT_SOURCE_DIR@', '@INPUT@'],
)
