import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/server_list.dart';

/// The bug: a fresh install of a build with no baked-in cloud URL seeded `''`
/// into the server list, so the account menu showed a cloud row with no label
/// and a delete button — and picking it pointed the app at a server that does not
/// exist. Found by running the desktop app, not by any test, which is why the
/// rule now lives in a function that can have one.
void main() {
  test('no cloud configured means no server row at all', () {
    // The whole bug in one line: an empty seed must produce nothing, not a row
    // that names nothing.
    expect(knownServers(rawPref: null, seed: ''), isEmpty);
    expect(knownServers(rawPref: '', seed: ''), isEmpty);
    expect(knownServers(rawPref: '[]', seed: ''), isEmpty);
  });

  test('an empty entry already saved by an older build is dropped', () {
    // The ghost was durable: adding a real server persisted ['', 'https://…'].
    // Loading has to clean it, or the row survives the fix.
    expect(knownServers(rawPref: '["","https://mica.example.com"]', seed: ''), [
      'https://mica.example.com',
    ]);
  });

  test('a configured cloud leads the list', () {
    expect(knownServers(rawPref: null, seed: 'https://mica.example.com'), [
      'https://mica.example.com',
    ]);
    // Already stored → not duplicated, and it keeps its stored position.
    expect(
      knownServers(
        rawPref: '["https://a.example","https://mica.example.com"]',
        seed: 'https://mica.example.com',
      ),
      ['https://a.example', 'https://mica.example.com'],
    );
  });

  test("the stored order is the user's and is preserved", () {
    // The switcher lets them reorder; loading must not sort.
    expect(
      knownServers(
        rawPref: '["https://z.example","https://a.example"]',
        seed: '',
      ),
      ['https://z.example', 'https://a.example'],
    );
  });

  test('duplicates collapse to the first occurrence', () {
    expect(
      knownServers(
        rawPref: '["https://a.example","https://a.example"]',
        seed: '',
      ),
      ['https://a.example'],
    );
  });

  test('a corrupt pref reads as no servers, never throws', () {
    // Losing the list is recoverable (add the server again); failing to start is
    // not.
    for (final raw in ['not json', '{"a":1}', '[1,2,3]', '[null]']) {
      expect(
        knownServers(rawPref: raw, seed: 'https://mica.example.com'),
        ['https://mica.example.com'],
        reason: 'raw=$raw',
      );
    }
  });
}
