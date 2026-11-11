import '../l10n/locale_controller.dart';
import 'dart:async';
import 'package:flutter/material.dart';
import '../api/models.dart';
import 'page_graph_view.dart';
import 'status_kit.dart';
import 'theme_tokens.dart';

/// The graph view, in a dialog. Loads ONCE when opened: the layout is a
/// deterministic one-shot pass, so re-running it per rebuild would be pure waste
/// and would also make the picture jump.
class GraphDialog extends StatefulWidget {
  const GraphDialog({
    super.key,
    required this.load,
    required this.onOpen,
    this.currentViewId,
  });

  final Future<PageGraph> Function() load;
  final void Function(String viewId) onOpen;
  final String? currentViewId;

  @override
  State<GraphDialog> createState() => _GraphDialogState();
}

class _GraphDialogState extends State<GraphDialog> {
  late final Future<PageGraph> _graph = widget.load();

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    return Dialog(
      insetPadding: const EdgeInsets.all(24),
      child: SizedBox(
        width: 900,
        height: 640,
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(20, 16, 12, 12),
              child: Row(
                children: [
                  Text(
                    context.l10n.graphTitle,
                    style: TextStyle(
                      fontSize: 16,
                      fontWeight: FontWeight.w600,
                      color: tokens.text.primary,
                    ),
                  ),
                  const Spacer(),
                  IconButton(
                    onPressed: () => Navigator.of(context).pop(),
                    icon: const Icon(Icons.close, size: 20),
                  ),
                ],
              ),
            ),
            Divider(height: 1, color: tokens.border.subtle),
            Expanded(
              child: FutureBuilder<PageGraph>(
                future: _graph,
                builder: (context, snapshot) {
                  if (snapshot.connectionState != ConnectionState.done) {
                    return const Center(child: CircularProgressIndicator());
                  }
                  // A failed load must NOT render as an empty graph — that would
                  // state "nothing links to anything" as a fact.
                  if (snapshot.hasError) {
                    return Center(
                      child: Padding(
                        padding: const EdgeInsets.all(24),
                        child: ErrorBanner('${snapshot.error}'),
                      ),
                    );
                  }
                  return PageGraphView(
                    graph: snapshot.data ?? PageGraph.empty,
                    currentViewId: widget.currentViewId,
                    onOpen: widget.onOpen,
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }
}
