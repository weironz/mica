// Where a profile picture lives.
//
// The server never sends an avatar URL — it sends an `avatar_version` (null when
// there is no picture) and the reader already knows the user id. Composing the
// address here, once, is what keeps that from becoming two answers to the same
// question: a URL in the payload for yourself and a hand-built one for everyone
// else in the member list, free to disagree the day the route moves.

/// The address of a user's profile picture, or null when they have none.
///
/// [version] is the server's `avatar_version`: null means no picture at all —
/// the caller should draw the initial instead, not request a URL that 404s.
///
/// The version rides along as `?v=`. The path is deliberately stable (it is just
/// the user id), so without this a replaced picture would keep resolving to
/// whatever the image cache already had — the one failure mode where everything
/// reports success and the user still sees their old face.
String? avatarUrl({
  required Uri base,
  required String userId,
  required String? version,
}) {
  if (version == null || version.isEmpty || userId.isEmpty) return null;
  return base
      .resolve('/api/users/$userId/avatar')
      .replace(queryParameters: {'v': version})
      .toString();
}
