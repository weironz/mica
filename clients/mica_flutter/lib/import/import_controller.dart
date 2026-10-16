import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/foundation.dart';

import '../api/models.dart';
import '../editor/file_names.dart' show fileNameFromUrl;

/// The logic half of a server-side workspace import, extracted verbatim from
/// `_WorkspaceShellState` (the god-class controller pilot in
/// docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md). Owns the job-poll
/// state machine and the second-pass image re-host. Everything that needs a
/// [BuildContext] — dialogs, snackbars, the busy wrapper — stays in the State
/// and is wired in through the hooks below.
///
/// All remote calls arrive as injected functions, so the controller has no
/// dependency on `MicaApi` and tests script it directly.
class ImportController extends ChangeNotifier {
  ImportController({
    required this.jobStatus,
    required this.listWorkspaces,
    required this.rehostImage,
    required this.cancelJob,
    required this.fetchImageBytes,
  });

  final Future<ImportJobStatus> Function(String jobId) jobStatus;
  final Future<List<Workspace>> Function() listWorkspaces;
  final Future<void> Function(
    String token,
    String workspaceId,
    String documentId, {
    required String blockId,
    required String fileName,
    required Uint8List bytes,
  }) rehostImage;
  final Future<void> Function(String token, String jobId) cancelJob;

  /// Fetches one external image. Returns null when the fetch fails — on web
  /// this is subject to CORS in ways the desktop build is not (see the re-host
  /// pass doc below); the caller treats null as "still a link".
  final Future<Uint8List?> Function(String url) fetchImageBytes;

  /// The running import, for the sidebar progress row and its cancel button.
  /// Null when nothing is in flight.
  String? jobId;

  /// The last `done/total` the server reported, while [jobId] is set.
  ({int done, int total})? progress;

  /// Mirrors the "re-host external images" setting; the State keeps it current.
  bool rehostEnabled = true;

  // ── Presentation hooks (implemented by the State) ─────────────────────────
  //
  // The controller never touches a context; these are the only way it reaches
  // the user. onLanded is awaited before the second image pass, matching the
  // original ordering (workspace selected first, then images pulled).

  /// A failure string for the message bus (the old `_message` field).
  void Function(String message)? onError;

  /// An import stopped on request: how many pages had landed. The import is
  /// NOT rolled back — say so, don't treat it as a no-op.
  void Function(int landed)? onCancelled;

  /// The refreshed workspace list after any terminal state.
  void Function(List<Workspace> workspaces)? onWorkspacesLoaded;

  /// A successful import: select the target workspace (awaited).
  Future<void> Function(Workspace target)? onLanded;

  /// After [onLanded] and the second image pass. Only fires when pages landed
  /// (`job.done > 0`); [stillLinked] is what BOTH passes could not bring in.
  void Function(ImportJobStatus job, List<ImportImageFailure> stillLinked)?
      onFinished;

  /// Record a started job and show its progress row.
  void attach(String id) {
    jobId = id;
    notifyListeners();
  }

  /// Ask the server to stop the running import.
  Future<void> cancel(String token) async {
    final id = jobId;
    if (id == null) return;
    try {
      await cancelJob(token, id);
    } catch (_) {
      // The poll loop is what reports the outcome; a failed cancel just means
      // the import keeps going, which the progress row already shows.
    }
  }

  /// Second attempt at every image the server could not bring in — fetched
  /// from THIS machine, immediately, as part of the import.
  ///
  /// Not left to `mica-cli rehost-images` afterwards. "I'll re-host them
  /// later" is exactly the plan that lost a whole AppFlowy export's images:
  /// the server cannot reach AppFlowy (it blocks datacenter IPs), so every
  /// image stayed a link while the import reported success — and the source
  /// workspace was deleted a few minutes later, because why keep it once the
  /// content is in. By the time "later" came there was nothing left to fetch.
  /// The moment this is guaranteed to work is the one right after the import,
  /// and that is the only moment worth spending.
  ///
  /// Returns what is STILL a link, which is what the user has to be told about.
  ///
  /// ON WEB THIS IS WEAKER THAN ON DESKTOP, and measured, not assumed: the
  /// fetch is a browser fetch, so a source that sends no
  /// `Access-Control-Allow-Origin` blocks it — verified against a local host
  /// the server could not reach, where the browser failed with
  /// `blocked by CORS policy` until the fixture sent the header. Desktop has
  /// no such limit. So the desktop app is the one that can actually rescue an
  /// export from a host that fences out servers, and the web app may still
  /// have to report images it can see but not read.
  Future<List<ImportImageFailure>> rehostFailures(
    String? token,
    List<ImportImageFailure> failures,
    String? workspaceId,
  ) async {
    if (token == null || workspaceId == null) return failures;
    final remaining = <ImportImageFailure>[];
    // Fetched once per URL, posted once per BLOCK: the same link on twenty
    // pages is twenty blocks to repoint, but only one download. A url that
    // could not be fetched is remembered as null so the next of its twenty
    // occurrences does not pay for the timeout again.
    final fetched = <String, Uint8List?>{};
    for (final failure in failures) {
      if (!failure.isRetryable) {
        remaining.add(failure);
        continue;
      }
      if (!fetched.containsKey(failure.url)) {
        fetched[failure.url] = await fetchImageBytes(failure.url);
      }
      final bytes = fetched[failure.url];
      if (bytes == null || bytes.isEmpty) {
        remaining.add(failure);
        continue;
      }
      try {
        await rehostImage(
          token,
          workspaceId,
          failure.documentId,
          blockId: failure.blockId,
          fileName: fileNameFromUrl(failure.url),
          bytes: bytes,
        );
      } catch (_) {
        // Store/patch refused it. Still a link, so still reported — swallowing
        // this would put it back in the class of failure this whole feature
        // exists to end.
        remaining.add(failure);
      }
    }
    return remaining;
  }

  /// Poll a running import to completion, updating [progress].
  ///
  /// Owns its error reporting: a failure here still has to reach the user (via
  /// [onError]), but must not pretend the whole app is busy. [failedMessage]
  /// supplies the localized fallback when the server sent no error text.
  Future<void> watch(
    String token,
    String id,
    String? workspaceId,
    String failedMessage, {
    Duration pollDelay = const Duration(milliseconds: 600),
  }) async {
    try {
      ImportJobStatus job;
      while (true) {
        job = await jobStatus(id);
        // The server reports done/total all along; surfacing it is what keeps
        // a 96-page import from reading as one indeterminate spinner.
        if (job.total > 0) {
          progress = (done: job.done, total: job.total);
          notifyListeners();
        }
        if (job.status != 'running') break;
        await Future<void>.delayed(pollDelay);
      }
      if (job.status == 'error') {
        throw ApiException(job.error ?? failedMessage);
      }
      // Stopped on request. Say how much did land — the import is NOT rolled
      // back, and treating this like a no-op would leave the user believing
      // their workspace is untouched when it now holds part of the archive.
      if (job.status == 'cancelled') {
        onCancelled?.call(job.done);
        final workspaces = await listWorkspaces();
        onWorkspacesLoaded?.call(workspaces);
        return;
      }
      final workspaces = await listWorkspaces();
      onWorkspacesLoaded?.call(workspaces);
      final targetId = job.workspaceId ?? workspaceId;
      for (final w in workspaces) {
        if (w.id == targetId) {
          await onLanded?.call(w);
          break;
        }
      }
      // Second pass at the images the server could not fetch, over THIS
      // machine's network, before anything is called done. Runs inside the try
      // so [progress] stays up — `finally` is what clears it — rather than the
      // app looking finished while it is still pulling images in.
      var stillLinked = job.imageFailures;
      if (rehostEnabled && stillLinked.isNotEmpty) {
        stillLinked = await rehostFailures(token, stillLinked, targetId);
      }
      if (job.done > 0) {
        onFinished?.call(job, stillLinked);
      }
    } on ApiException catch (error) {
      onError?.call(error.message);
    } catch (error) {
      onError?.call('$error');
    } finally {
      progress = null;
      jobId = null;
      notifyListeners();
    }
  }

  /// Workspace name from an archive filename: drop the extension and the
  /// ID noise Notion adds — `Export-<uuid>.zip` (suffix) and
  /// `<uuid>_Export.zip` (prefix) both clean up to "Export".
  static String cleanArchiveName(String fileName) {
    const hex32 = r'[0-9a-fA-F]{32}';
    const uuid =
        r'[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}'
        r'-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}';
    var name = fileName
        .replaceAll(RegExp(r'\.zip$', caseSensitive: false), '')
        .trim();
    name = name
        .replaceFirst(RegExp('[ \\-_]+($hex32|$uuid)\$'), '')
        .replaceFirst(RegExp('^($hex32|$uuid)[ \\-_]+'), '');
    return name;
  }
}
