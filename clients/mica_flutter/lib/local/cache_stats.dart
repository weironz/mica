// Telling the on-device cache apart from the on-device originals.
//
// The local store holds two different things in one place, and only one of them
// is a cache:
//
//   * **Cloud mirrors** — pages and images copied down from a server so they can
//     be read offline. Deleting them costs a re-download and nothing else.
//   * **Local workspaces** — pages that exist ONLY on this device. No server has
//     ever seen them. Deleting them is data loss.
//
// Reporting one number over both would invite someone to "clear the cache" and
// lose the only copy of something, so the split has to be decidable rather than
// guessed. It is: the blob store keys the two by different id formats (see
// `local_offline_io.dart`) — a local image is content-addressed by its sha256,
// while a mirrored one keeps the server's UUID file id.
import 'dart:convert';


/// Whether a blob id in `blobs/` belongs to a cloud mirror (and is therefore
/// re-downloadable) rather than to a local-only document.
///
/// Decided by id SHAPE, which is what the store already relies on to let the two
/// schemes share one directory: `8-4-4-4-12` hex with dashes is a server file id;
/// 64 unbroken hex characters is a local sha256. Anything else counts as local —
/// the safe direction, because being wrong costs a re-download one way and a lost
/// original the other.
bool isCloudMirrorBlobId(String id) {
  if (id.length != 36) return false;
  const dashes = [8, 13, 18, 23];
  for (final i in dashes) {
    if (id[i] != '-') return false;
  }
  for (var i = 0; i < id.length; i++) {
    if (dashes.contains(i)) continue;
    if (!_isHexDigit(id.codeUnitAt(i))) return false;
  }
  return true;
}

bool _isHexDigit(int c) =>
    (c >= 0x30 && c <= 0x39) || // 0-9
    (c >= 0x61 && c <= 0x66) || // a-f
    (c >= 0x41 && c <= 0x46); // A-F

/// What the on-device store is holding, split so the cache part can be reported
/// — and one day reclaimed — without touching the originals.
typedef LocalCacheStats = ({
  /// Pages mirrored from a server: re-downloadable.
  int mirroredPages,

  /// Bytes of mirrored images: re-downloadable.
  int mirroredBytes,

  /// Pages that live only on this device. Reported so the total is honest about
  /// what the store holds, never offered for deletion.
  int localOnlyPages,

  /// Bytes of images belonging to local-only documents.
  int localOnlyBytes,
});

/// Outcome of [LocalOfflineApi.clearMirrorCache].
///
/// Issue #110: the durable outbox of a document with un-pushed edits is not a
/// cache — it is the only copy of that work. Clearing therefore keeps those
/// documents (outbox, snapshot, cursor, versions) and reports how many it
/// kept, so the UI can say so instead of claiming everything went.
typedef ClearMirrorResult = ({
  /// What the store holds after the clear (measured, not predicted).
  LocalCacheStats stats,

  /// Cloud-mirror documents retained because they still have un-pushed local
  /// updates. Their page/view rows may be gone; the document itself is whole.
  int retainedDocs,

  /// Origins for which a legacy (pre-#111, server-only-keyed) cache was found
  /// and left untouched. These rows belong to an ACCOUNT WE CANNOT VERIFY, so
  /// they are quarantined rather than shown to the current user or deleted.
  List<String> quarantinedOrigins,
});

/// The on-device origin key for ONE account's cloud cache on ONE server.
///
/// Issue #111: the mirror used to be keyed by the server URL alone, so account
/// A's private page tree, bodies and roles came back for account B on the same
/// server. Every cloud-cache read/write now goes through this function instead
/// of concatenating keys by hand.
///
/// The result is a JSON array `[normalizedServerUrl, accountId]` — an
/// unambiguous tuple, not a delimited string, so no character that a URL or an
/// account id may contain can make two different pairs collide. It is persisted
/// verbatim in SQLite (`origin`), in localStorage keys and in IndexedDB store
/// names, so the server URL is normalized first (lower-case scheme/host,
/// default port and trailing slashes dropped) and the account id is kept
/// case-sensitively.
///
/// Pre-#111 cache rows are keyed by the bare server URL and are deliberately
/// NOT claimed here — see [legacyCloudCacheOrigin]. We cannot tell which
/// account wrote them, and handing them to whoever logs in next is exactly the
/// bug.
String cloudCacheOrigin(String serverUrl, String accountId) {
  final account = accountId.trim();
  if (account.isEmpty) {
    throw ArgumentError.value(
      accountId,
      'accountId',
      'cloud cache is account-scoped; refusing an empty account',
    );
  }
  return jsonEncode([normalizeServerUrl(serverUrl), account]);
}

/// [cloudCacheOrigin] for a call site that already knows the account may be
/// absent (signed out). Throws the same way [cloudCacheOrigin] does when there
/// is no account: a cache operation without an account is a bug, not a default.
String requireCloudCacheOrigin(String serverUrl, String? accountId) {
  if (accountId == null || accountId.isEmpty) {
    throw StateError('cloud cache is unavailable while signed out');
  }
  return cloudCacheOrigin(serverUrl, accountId);
}

/// The pre-#111 cache key: the server URL with no account scope.
///
/// Read ONLY by the quarantine path that keeps old (possibly another
/// account's) rows on disk instead of deleting or claiming them. Nothing in
/// the live read/write path may use this as an origin.
String legacyCloudCacheOrigin(String serverUrl) =>
    normalizeServerUrl(serverUrl);

/// Canonical form of a server base URL, used as the first half of
/// [cloudCacheOrigin] and as the whole pre-#111 key.
///
/// `https://Mica.Example.com:443/mica/` and `https://mica.example.com/mica`
/// name the same server and must map to the same cache rows. Input that is not
/// an absolute http(s) URL is REJECTED rather than passed through: a fallback
/// would let a malformed value collide with another key's tuple.
String normalizeServerUrl(String serverUrl) {
  final uri = Uri.tryParse(serverUrl.trim());
  if (uri == null ||
      !(uri.scheme == 'http' || uri.scheme == 'https') ||
      uri.host.isEmpty) {
    throw ArgumentError.value(
      serverUrl,
      'serverUrl',
      'a cloud cache key needs an absolute http(s) server URL',
    );
  }
  final scheme = uri.scheme.toLowerCase();
  final host = uri.host.toLowerCase();
  final defaultPort =
      (scheme == 'http' && uri.port == 80) ||
      (scheme == 'https' && uri.port == 443);
  var path = uri.path;
  while (path.endsWith('/')) {
    path = path.substring(0, path.length - 1);
  }
  return '$scheme://$host${defaultPort ? '' : ':${uri.port}'}$path';
}

/// The title a Markdown import should use: the first heading, else the first
/// non-empty line (trimmed to a sane length), else [fallback].
///
/// Shared by the cloud import path and the local one so a file imported into
/// either world gets the same name (issue #108).
String titleFromMarkdown(String markdown, String fallback) {
  for (final line in markdown.split('\n')) {
    final trimmed = line.trim();
    final heading = RegExp(r'^#{1,6}\s+(.*)$').firstMatch(trimmed);
    if (heading != null && heading.group(1)!.trim().isNotEmpty) {
      return heading.group(1)!.trim();
    }
    if (trimmed.isNotEmpty) {
      return trimmed.length > 60 ? trimmed.substring(0, 60) : trimmed;
    }
  }
  return fallback;
}
