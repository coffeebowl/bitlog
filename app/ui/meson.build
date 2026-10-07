# The outputs are listed file by file, so that Meson rebuilds the resources
# whenever a blueprint changes.
blueprints = custom_target(
  'blueprints',
  input: files('calendar_view.blp', 'commit_dialog.blp', 'conflict_dialog.blp', 'day_view.blp', 'git_page.blp', 'help_dialog.blp', 'note_view.blp', 'notes_page.blp', 'preferences_dialog.blp', 'project_dialog.blp', 'project_view.blp', 'projects_page.blp', 'reports_page.blp', 'search_dialog.blp', 'shortcuts_dialog.blp', 'standup_dialog.blp', 'sync_conflict_dialog.blp', 'task_list_view.blp', 'tasks_page.blp', 'vault_check_dialog.blp', 'window.blp'),
  output: ['calendar_view.ui', 'commit_dialog.ui', 'conflict_dialog.ui', 'day_view.ui', 'git_page.ui', 'help_dialog.ui', 'note_view.ui', 'notes_page.ui', 'preferences_dialog.ui', 'project_dialog.ui', 'project_view.ui', 'projects_page.ui', 'reports_page.ui', 'search_dialog.ui', 'shortcuts_dialog.ui', 'standup_dialog.ui', 'sync_conflict_dialog.ui', 'task_list_view.ui', 'tasks_page.ui', 'vault_check_dialog.ui', 'window.ui'],
  command: [blueprint_compiler, 'batch-compile', '@OUTDIR@', '@CURRENT_SOURCE_DIR@', '@INPUT@'],
)
