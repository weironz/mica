import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/avatar_url.dart';

/// The address is composed here and nowhere else, so these are the rules the
/// account tile, the member list and the settings panel all inherit.
void main() {
  final base = Uri.parse('https://mica.example.com');
  const uid = '55b3e5ff-4117-4d65-9434-0b17922d8e87';

  test('no version means no picture — not a URL that will 404', () {
    // The server sends null for every account that never set one, which is most
    // of them. Returning a URL anyway would fire a request per member row and
    // paint a broken image where an initial belongs.
    expect(avatarUrl(base: base, userId: uid, version: null), isNull);
    expect(avatarUrl(base: base, userId: uid, version: ''), isNull);
  });

  test('the URL is the public read route under the given server', () {
    final url = avatarUrl(base: base, userId: uid, version: 'abc123')!;
    expect(url, startsWith('https://mica.example.com/api/users/$uid/avatar'));
  });

  test('a new picture produces a different URL', () {
    // The path is stable by design, so this query is the only thing that can
    // tell an image cache the face changed.
    final before = avatarUrl(base: base, userId: uid, version: 'aaaa1111');
    final after = avatarUrl(base: base, userId: uid, version: 'bbbb2222');
    expect(before, isNot(after));
    expect(after, contains('v=bbbb2222'));
  });

  test('a base with a port or a path still resolves to the server root', () {
    // Desktop points at http://127.0.0.1:8080; the route is absolute, so a base
    // carrying a path must not end up as /somewhere/api/users/….
    final dev = avatarUrl(
      base: Uri.parse('http://127.0.0.1:8080'),
      userId: uid,
      version: 'v1',
    )!;
    expect(dev, startsWith('http://127.0.0.1:8080/api/users/'));
    final withPath = avatarUrl(
      base: Uri.parse('https://host.example/app/'),
      userId: uid,
      version: 'v1',
    )!;
    expect(withPath, startsWith('https://host.example/api/users/'));
  });

  test('an empty user id yields nothing rather than a malformed URL', () {
    expect(avatarUrl(base: base, userId: '', version: 'abc'), isNull);
  });
}
