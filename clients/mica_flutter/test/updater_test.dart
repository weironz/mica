import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/updater_common.dart';

void main() {
  group('compareVersions', () {
    test('orders by numeric component, not lexically', () {
      expect(compareVersions('0.1.10', '0.1.9'), greaterThan(0));
      expect(compareVersions('0.1.9', '0.1.10'), lessThan(0));
      expect(compareVersions('0.2.0', '0.1.99'), greaterThan(0));
    });

    test('equal versions compare equal', () {
      expect(compareVersions('0.1.5', '0.1.5'), 0);
      expect(compareVersions('v0.1.5', '0.1.5'), 0); // leading v tolerated
    });

    test('differing component counts pad with zero', () {
      expect(compareVersions('0.1', '0.1.0'), 0);
      expect(compareVersions('0.1.1', '0.1'), greaterThan(0));
      expect(compareVersions('1', '0.9.9'), greaterThan(0));
    });

    test('trailing non-numeric junk on a component is ignored', () {
      expect(compareVersions('1.2.0-rc1', '1.2.0'), 0);
      expect(compareVersions('1.2.3-beta', '1.2.2'), greaterThan(0));
    });

    test('the update-available decision (latest vs current)', () {
      // What checkForUpdate uses: newer => offer, else skip.
      expect(compareVersions('0.1.6', '0.1.5'), greaterThan(0)); // offer
      expect(compareVersions('0.1.5', '0.1.5'), 0); // up to date
      expect(compareVersions('0.1.4', '0.1.5'), lessThan(0)); // older, skip
    });
  });
}
