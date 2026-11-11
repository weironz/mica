// Isolated UI acceptance: actual module implementations, no external account.
import 'dart:convert';
import 'dart:js_interop';
import 'package:flutter/material.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/deferred_load.dart';
import 'package:mica_flutter/ui/ai_dialog.dart' deferred as ai;
import 'package:mica_flutter/ui/graph_dialog.dart' deferred as graph;
import 'package:mica_flutter/ui/version_history_dialog.dart'
    deferred as history;

@JS('micaDeferredState')
external set _state(JSFunction value);
void main() => runApp(const _Harness());

class _Harness extends StatefulWidget {
  const _Harness();
  @override
  State<_Harness> createState() => _HarnessState();
}

class _HarnessState extends State<_Harness> {
  String? active;
  GlobalKey? dialogKey;
  @override
  void initState() {
    super.initState();
    _state = (() => jsonEncode({
      'active': active,
      'dialogMounted': active != null && dialogKey?.currentContext != null,
    }).toJS).toJS;
  }

  Future<void> open(BuildContext context, String kind) async {
    final load = switch (kind) {
      'ai' => ai.loadLibrary,
      'history' => history.loadLibrary,
      _ => graph.loadLibrary,
    };
    if (!await loadUiModule(context, load)) return;
    if (!context.mounted) return;
    final key = GlobalKey();
    dialogKey = key;
    setState(() => active = kind);
    try {
      await showDialog<void>(
        context: context,
        builder: (_) => switch (kind) {
          'ai' => ai.AiDialog(
            key: key,
            canEdit: false,
            hasWorkspace: false,
            onStream: (_, {system}) => const Stream<String>.empty(),
            onNewPage: (_) async {},
            onCurrentPage: null,
            currentPage: null,
            onNewWorkspace: (_) async {},
          ),
          'history' => history.VersionHistoryDialog(
            key: key,
            onList: () async => [],
            onCreate: (_) async {},
            onRestore: (_) async {},
            onLoadContent: (_) async =>
                (rootBlockId: 'root', blocks: <Map<String, dynamic>>[]),
          ),
          _ => graph.GraphDialog(
            key: key,
            load: () async => PageGraph.empty,
            onOpen: (_) {},
          ),
        },
      );
    } finally {
      if (mounted) setState(() => active = null);
    }
  }

  @override
  Widget build(BuildContext context) => MaterialApp(
    locale: const Locale('zh'),
    localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales,
    home: Scaffold(
      body: Builder(
        builder: (context) => Column(
          children: [
            for (final kind in ['ai', 'history', 'graph'])
              TextButton(
                onPressed: () => open(context, kind),
                child: Text(kind),
              ),
          ],
        ),
      ),
    ),
  );
}
