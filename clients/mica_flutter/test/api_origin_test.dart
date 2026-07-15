import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/client.dart';

// Image blob links are copied out and pasted into other apps, so the origin
// they carry is user-visible. `Uri.port` always answers with a number, and
// building the origin from it verbatim shipped `https://host:443/…`.
void main() {
  test('https on the default port drops it', () {
    expect(apiOrigin(Uri.parse('https://mica.cloudcele.com')),
        'https://mica.cloudcele.com');
    expect(apiOrigin(Uri.parse('https://mica.cloudcele.com:443/api/x')),
        'https://mica.cloudcele.com');
  });

  test('http on the default port drops it', () {
    expect(apiOrigin(Uri.parse('http://127.0.0.1:80')), 'http://127.0.0.1');
    expect(apiOrigin(Uri.parse('http://example.com/a/b')), 'http://example.com');
  });

  test('a non-default port is kept — the dev server lives on one', () {
    expect(apiOrigin(Uri.parse('http://127.0.0.1:8080')), 'http://127.0.0.1:8080');
    expect(apiOrigin(Uri.parse('https://staging.example.com:8443')),
        'https://staging.example.com:8443');
  });

  test('a mismatched default (https on 80) is kept — it is not the default', () {
    expect(apiOrigin(Uri.parse('https://example.com:80')),
        'https://example.com:80');
  });
}
