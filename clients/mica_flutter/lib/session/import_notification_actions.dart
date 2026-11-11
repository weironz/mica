import '../api/client.dart';

typedef ImportNotificationActions = ({
  Future<bool> Function() load,
  Future<void> Function(bool) save,
});

/// A settings route keeps the account and server it was opened for. Capturing
/// the API base also keeps an in-flight retry off a subsequently selected server.
ImportNotificationActions importNotificationActions({
  required ApiClient api,
  required String Function() accessToken,
  required bool Function() isCurrent,
}) {
  final bound = ApiClient()..baseUri = api.baseUri;
  void check() {
    if (!isCurrent()) throw StateError('Settings account changed');
  }

  return (
    load: () async {
      check();
      final enabled = await bound.getImportNotifications(accessToken());
      check();
      return enabled;
    },
    save: (enabled) async {
      check();
      await bound.putImportNotifications(accessToken(), enabled);
      check();
    },
  );
}
