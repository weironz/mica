import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations_en.dart';
import 'package:mica_flutter/l10n/app_localizations_zh.dart';

// The first account on an empty instance is verified by the server on the spot,
// so the "check your email" line is two lies in one sentence for it: nothing was
// sent, and nothing needs clicking. A fresh self-hoster hit exactly that and sat
// waiting for a message that would never arrive.
//
// The branch itself lives in `_authenticate`, which needs the whole app shell to
// exercise. What IS reachable — and what actually broke — is the copy: there has
// to be a distinct string for the verified case, and it must not send anyone to
// their inbox.
void main() {
  test('the first-account line does not send anyone to their inbox', () {
    for (final l10n in [AppLocalizationsZh(), AppLocalizationsEn()]) {
      expect(
        l10n.authFirstAccountReady,
        isNot(l10n.authVerifySent),
        reason: 'a separate string is the whole point',
      );
      // Not "must not mention email" — the English line legitimately says
      // "needs NO email confirmation". What it must not do is send you off to
      // do something: no 查收, no "check your".
      expect(l10n.authFirstAccountReady, isNot(contains('查收')));
      expect(
        l10n.authFirstAccountReady.toLowerCase(),
        isNot(contains('check your')),
      );
      // It must also SAY WHY, or the next reader assumes verification is broken.
      expect(
        l10n.authFirstAccountReady.contains('第一个') ||
            l10n.authFirstAccountReady.toLowerCase().contains('first'),
        isTrue,
        reason: 'name the reason: this is the first account on the server',
      );
    }
  });

  test('the ordinary case still points at the mailbox', () {
    expect(AppLocalizationsZh().authVerifySent, contains('邮件'));
    expect(AppLocalizationsEn().authVerifySent.toLowerCase(), contains('email'));
  });
}
