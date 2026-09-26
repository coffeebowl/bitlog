# The outputs are listed file by file, so that Meson rebuilds the resources
# whenever a blueprint changes.
blueprints = custom_target(
  'blueprints',
  input: files('calendar_view.blp', 'day_view.blp', 'task_list_view.blp', 'tasks_page.blp', 'window.blp'),
  output: ['calendar_view.ui', 'day_view.ui', 'task_list_view.ui', 'tasks_page.ui', 'window.ui'],
  command: [blueprint_compiler, 'batch-compile', '@OUTDIR@', '@CURRENT_SOURCE_DIR@', '@INPUT@'],
)
