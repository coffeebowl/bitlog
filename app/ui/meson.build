blueprints = custom_target(
  'blueprints',
  input: files('window.blp'),
  output: '.',
  command: [blueprint_compiler, 'batch-compile', '@OUTPUT@', '@CURRENT_SOURCE_DIR@', '@INPUT@'],
)
