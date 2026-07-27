import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';

/// `kPresencePalette`'s doc comment promises "avatar + remote caret share one per
/// connection". The presence bar had quietly broken that: it kept its OWN copy of
/// the six colours and indexed it by the collaborator's position in the list,
/// while the remote caret used `presenceColor(connectionId)` — a hash. Same
/// person, two colours; and with a single collaborator the avatar was always the
/// first palette entry no matter whose it was.
///
/// The colour must therefore be a function of the CONNECTION, never of where the
/// person happens to sit in a list.
void main() {
  PresenceUser user(String connId, {String name = 'x'}) =>
      PresenceUser(connectionId: connId, userId: 'u-$connId', name: name);

  test('a user carries the same colour the caret painter would use', () {
    final u = user('conn-abc');

    expect(u.color, presenceColor('conn-abc'));
  });

  test('the colour follows the connection, not the list position', () {
    // The exact bug: rendering [a, b] vs [b, a] must not repaint anyone.
    final a = user('conn-a');
    final b = user('conn-b');

    final forward = [a, b].map((u) => u.color).toList();
    final reversed = [b, a].map((u) => u.color).toList();

    expect(forward[0], reversed[1]);
    expect(forward[1], reversed[0]);
  });

  test('a lone collaborator is not forced to the first palette entry', () {
    // With index-based lookup every solo collaborator came out palette[0].
    // At least one connection id must land elsewhere, or the hash is not working.
    final solo = [for (var i = 0; i < 40; i++) user('conn-$i').color];

    expect(
      solo.toSet().length,
      greaterThan(1),
      reason: 'colour must vary with the connection id',
    );
  });

  test('the same id is stable across calls', () {
    // Reconnects aside, a repaint must not change anyone's colour.
    expect(presenceColor('conn-stable'), presenceColor('conn-stable'));
  });

  test('every produced colour comes from the shared palette', () {
    for (var i = 0; i < 40; i++) {
      expect(kPresencePalette, contains(user('conn-$i').color));
    }
  });
}
