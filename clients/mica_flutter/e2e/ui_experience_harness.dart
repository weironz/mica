// Local browser fixture: production widgets and editor, synthetic document only.
import 'dart:convert';
import 'dart:js_interop';

import 'package:flutter/material.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/main.dart' as app;
import 'package:mica_flutter/ui/appearance_preview.dart';
import 'package:mica_flutter/ui/page_title_field.dart';
import 'package:mica_flutter/ui/settings_shell.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

@JS('micaExperienceFocus')
external set _focusEditor(JSFunction value);
@JS('micaExperienceState')
external set _readState(JSFunction value);

void main() {
  // The same build also allows testing the full production settings wiring
  // against a Playwright-local mock API, never a production account.
  if (Uri.base.queryParameters['fixture'] == 'app') {
    app.main();
    return;
  }
  runApp(const _ExperienceApp());
}

class _ExperienceApp extends StatefulWidget {
  const _ExperienceApp();
  @override
  State<_ExperienceApp> createState() => _ExperienceAppState();
}

class _ExperienceAppState extends State<_ExperienceApp> {
  final _editorKey = GlobalKey();
  final _commands = EditorCommandHook();
  final _title = TextEditingController(
    text: '让知识沉淀，也让编辑更舒适 · A calm writing space',
  );
  final _titleFocus = FocusNode();
  final _nodes = <EditorNode>[
    EditorNode(
      id: 'start',
      kind: 'paragraph',
      text: '中文长文与 English 一起阅读，输入和选区应及时响应。',
    ),
    EditorNode(
      id: 'h1',
      kind: 'heading',
      text: '清晰的排版与稳定的编辑',
      data: {'level': 1},
    ),
    EditorNode(
      id: 'p',
      kind: 'paragraph',
      text: '段落有呼吸感，标题与正文有明确层次。少量颜色提示状态，工具栏在需要时出现。阅读代码、记录想法、整理长篇资料，都使用同一套编辑规则。',
    ),
    EditorNode(id: 'li1', kind: 'bulleted_list', text: '保留已有字体和阅读宽度偏好'),
    EditorNode(id: 'li2', kind: 'bulleted_list', text: '拖选结束后再显示格式工具栏'),
    EditorNode(id: 'h2', kind: 'heading', text: '技术资料', data: {'level': 2}),
    EditorNode(
      id: 'code',
      kind: 'code_block',
      text: 'fn main() {\n    println!("Hello, Mica");\n}',
      data: {'language': 'rust'},
    ),
    for (var i = 0; i < 200; i++)
      EditorNode(
        id: 'long-$i',
        kind: 'paragraph',
        text:
            '第${i + 1}段：编辑舒适感来自及时的文字反馈、稳定的光标定位与清晰的层次。This is a synthetic long-document sample.',
      ),
  ];
  bool _dark = false;
  double _font = 17;
  double _width = 800;
  MicaTokens get _tokens => _dark ? MicaTokens.dark_ : MicaTokens.light;
  EditorAppearance get _appearance =>
      EditorAppearance(fontScale: _font / 16, tokens: _tokens);

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _focusEditor = (() {
        _commands.focusFirstLine();
      }).toJS;
      _readState = (() {
        final state = <String, dynamic>{'dark': _dark, 'font': _font};
        void visit(Element element) {
          if (element.widget case final DocumentSurface surface) {
            final render =
                (element as RenderObjectElement).renderObject as RenderDocument;
            // This entrypoint is a browser test, excluded from production builds.
            // ignore: invalid_use_of_visible_for_testing_member
            state['layouts'] = render.debugLayoutCount;
            state['revision'] = surface.contentRevision;
            state['text'] = surface.nodes.first.text;
            state['nodes'] = surface.nodes.length;
            state['selection'] = surface.selection == null
                ? null
                : {
                    'node': surface.selection!.focus.node,
                    'offset': surface.selection!.focus.offset,
                    'collapsed': surface.selection!.isCollapsed,
                  };
            final caret = surface.selection == null
                ? null
                : render.caretRectFor(surface.selection!.focus);
            if (caret != null) {
              final global = render.localToGlobal(caret.topLeft);
              state['caret'] = {
                'x': global.dx,
                'y': global.dy,
                'height': caret.height,
              };
            }
          }
          element.visitChildElements(visit);
        }

        final context = _editorKey.currentContext;
        if (context is Element) visit(context);
        return jsonEncode(state).toJS;
      }).toJS;
    });
  }

  @override
  void dispose() {
    _title.dispose();
    _titleFocus.dispose();
    super.dispose();
  }

  void _settings(BuildContext context) {
    var selected = 0;
    showDialog<void>(
      context: context,
      builder: (_) => StatefulBuilder(
        builder: (context, update) => MicaTheme(
          tokens: _tokens,
          child: Theme(
            data: _tokens.toMaterialTheme(),
            child: MicaSettingsShell(
              title: '设置',
              selectedIndex: selected,
              onSelected: (value) => update(() => selected = value),
              closeLabel: '关闭',
              backLabel: '返回',
              onClose: () => Navigator.pop(context),
              destinations: [
                MicaSettingsDestination(
                  group: '通用',
                  title: '外观',
                  description: '调整阅读与编辑的舒适度',
                  icon: Icons.palette_outlined,
                  content: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Text('正文字号 · ${_font.round()} px'),
                      Slider(
                        value: _font,
                        min: 13,
                        max: 22,
                        divisions: 9,
                        onChanged: (value) {
                          setState(() => _font = value);
                          update(() {});
                        },
                      ),
                      Text('阅读宽度 · ${_width.round()} px'),
                      Slider(
                        value: _width,
                        min: 600,
                        max: 1200,
                        divisions: 6,
                        onChanged: (value) {
                          setState(() => _width = value);
                          update(() {});
                        },
                      ),
                      MicaAppearancePreview(
                        appearance: _appearance,
                        pageWidth: _width,
                        label: '排版预览',
                        title: '让内容成为主角',
                        body: '中文与 English 保持清晰的阅读节奏，字体设置即时生效。',
                        code: 'print("Hello, Mica")',
                      ),
                    ],
                  ),
                ),
                const MicaSettingsDestination(
                  group: '通用',
                  title: '快捷键',
                  description: '编辑与导航',
                  icon: Icons.keyboard_outlined,
                  content: Text('Ctrl+B  加粗\nCtrl+Z  撤销\nCtrl+Shift+Z  重做'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => MaterialApp(
    debugShowCheckedModeBanner: false,
    locale: const Locale('zh'),
    localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales,
    theme: _tokens.toMaterialTheme(),
    home: MicaTheme(
      tokens: _tokens,
      child: Builder(
        builder: (context) => Scaffold(
          appBar: AppBar(
            title: const Text('Mica · 编辑体验'),
            actions: [
              IconButton(
                tooltip: '切换主题',
                onPressed: () => setState(() => _dark = !_dark),
                icon: Icon(
                  _dark ? Icons.light_mode_outlined : Icons.dark_mode_outlined,
                ),
              ),
              IconButton(
                tooltip: '设置',
                onPressed: () => _settings(context),
                icon: const Icon(Icons.settings_outlined),
              ),
            ],
          ),
          body: LayoutBuilder(
            builder: (context, constraints) => SingleChildScrollView(
              padding: EdgeInsets.symmetric(
                horizontal: constraints.maxWidth < 600 ? 12 : 28,
                vertical: 12,
              ),
              child: Center(
                child: ConstrainedBox(
                  constraints: BoxConstraints(maxWidth: _width),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Padding(
                        padding: const EdgeInsets.only(
                          left: EditorTheme.gutter,
                        ),
                        child: PageTitleField(
                          controller: _title,
                          focusNode: _titleFocus,
                          appearance: _appearance,
                          hintText: '未命名',
                          onChanged: (_) {},
                          onEnterBody: _commands.focusFirstLine,
                          onArrowDown: _commands.focusFirstLine,
                        ),
                      ),
                      MicaEditor(
                        key: _editorKey,
                        rootBlockId: 'root',
                        nodes: _nodes,
                        version: 0,
                        canEdit: true,
                        appearance: _appearance,
                        commandHook: _commands,
                        onApplyOperations: (_) async {},
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    ),
  );
}
