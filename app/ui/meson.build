# The outputs are listed file by file, so that Meson rebuilds the resources
# whenever a blueprint changes.
blueprints = custom_target(
  'blueprints',
  input: files('calendar_view.blp', 'conflict_dialog.blp', 'day_view.blp', 'note_view.blp', 'project_dialog.blp', 'project_view.blp', 'projects_page.blp', 'task_list_view.blp', 'tasks_page.blp', 'window.blp'),
  output: ['calendar_view.ui', 'conflict_dialog.ui', 'day_view.ui', 'note_view.ui', 'project_dialog.ui', 'project_view.ui', 'projects_page.ui', 'task_list_view.ui', 'tasks_page.ui', 'window.ui'],
  command: [blueprint_compiler, 'batch-compile', '@OUTDIR@', '@CURRENT_SOURCE_DIR@', '@INPUT@'],
)
