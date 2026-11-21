part of 'editor.dart';

const List<_SlashOption> _slashOptions = [
  _SlashOption('Text', Icons.notes, 'paragraph'),
  _SlashOption('Heading 1', Icons.title, 'heading', {'level': 1}),
  _SlashOption('Heading 2', Icons.title, 'heading', {'level': 2}),
  _SlashOption('Heading 3', Icons.title, 'heading', {'level': 3}),
  _SlashOption('Bulleted list', Icons.format_list_bulleted, 'bulleted_list'),
  _SlashOption('Numbered list', Icons.format_list_numbered, 'numbered_list'),
  _SlashOption('To-do', Icons.check_box_outlined, 'todo', {'checked': false}),
  _SlashOption('Quote', Icons.format_quote, 'quote'),
  // Callouts (GFM alerts) reuse the quote block, tagged with `data.alert`.
  _SlashOption('Note', Icons.info_outline, 'quote', {'alert': 'note'}),
  _SlashOption('Tip', Icons.lightbulb_outline, 'quote', {'alert': 'tip'}),
  _SlashOption('Important', Icons.campaign_outlined, 'quote', {
    'alert': 'important',
  }),
  _SlashOption('Warning', Icons.warning_amber_outlined, 'quote', {
    'alert': 'warning',
  }),
  _SlashOption('Caution', Icons.report_outlined, 'quote', {'alert': 'caution'}),
  _SlashOption('Code', Icons.code, 'code_block'),
  _SlashOption('Math formula', Icons.functions, 'math_block'),
  _SlashOption('Whiteboard', Icons.draw_outlined, 'code_block', {
    'language': 'mica-board',
    'template': 'whiteboard',
  }),
  _SlashOption('Mind map', Icons.account_tree_outlined, 'code_block', {
    'language': 'mica-board',
    'template': 'mindmap',
  }),
  _SlashOption('Flowchart', Icons.schema_outlined, 'code_block', {
    'language': 'mica-board',
    'template': 'flowchart',
  }),
  _SlashOption('Table', Icons.grid_on, 'table'),
  _SlashOption('Divider', Icons.horizontal_rule, 'divider'),
  _SlashOption('Image', Icons.image_outlined, 'image'),
];
