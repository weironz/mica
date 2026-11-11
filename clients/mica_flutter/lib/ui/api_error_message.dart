import '../api/models.dart';
import '../l10n/app_localizations.dart';
import 'format_bytes.dart';

String apiErrorMessage(
  ApiException error,
  AppLocalizations strings,
  int maxUploadBytes,
) {
  return switch (error.code) {
    'import_no_markdown' => strings.importNoMarkdownHint,
    // The two ways an upload gets refused for "size". They are easy to
    // confuse and the fix differs: one file is too big (compress it) versus
    // the workspace is full (free space). Worse, the first fires while the
    // storage bar honestly shows room left, so relaying the server's English
    // sentence left the refusal looking like the bar was lying.
    // The cap is named only when it is actually known: `/usage` is fetched
    // lazily (settings dialog), so someone whose first action is an oversized
    // paste has never been told the number. Quoting a guess would be worse
    // than omitting it — the whole point of this copy is to be believable.
    'file_too_large' =>
      maxUploadBytes > 0
          ? strings.uploadFileTooLargeWithLimit(
              formatBytesBinary(maxUploadBytes),
            )
          : strings.uploadFileTooLarge,
    'workspace_quota_exceeded' => strings.uploadWorkspaceFull,
    'owner_quota_exceeded' => strings.uploadOwnerFull,
    _ => error.toString(),
  };
}
