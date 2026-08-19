import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

/// A custom AI provider is named by the operator, and that name becomes two
/// things at once: the primary key the config is stored under, and a path
/// segment of `DELETE /api/ai/providers/{id}`. The add form is the only gate in
/// front of both.
void main() {
  test('the built-in vendor ids are all valid names', () {
    // Not a formality: the add form writes the preset's own id for anything but
    // Local / Custom, so a preset id the validator rejects would make that
    // vendor unaddable while looking fine in the dropdown.
    for (final id in ['deepseek', 'zhipu', 'kimi', 'openai', 'anthropic']) {
      expect(isValidAiProviderId(id), isTrue, reason: id);
    }
  });

  test('a name may carry the punctuation real deployments use', () {
    // Two providers from the same vendor at different versions or endpoints is
    // the ordinary case, so the separators people reach for have to work.
    expect(isValidAiProviderId('nvidia-glm5.2'), isTrue);
    expect(isValidAiProviderId('ollama_local'), isTrue);
    expect(isValidAiProviderId('gw2'), isTrue);
  });

  test('an empty or blank name is rejected', () {
    // Empty would address the ACTIVE provider server-side (absent provider_id
    // means "the one in use"), so an unnamed add would silently overwrite
    // whatever is running.
    expect(isValidAiProviderId(''), isFalse);
    expect(isValidAiProviderId('   '), isFalse);
  });

  test('anything that would not survive a URL path is rejected', () {
    expect(isValidAiProviderId('my provider'), isFalse);
    expect(isValidAiProviderId('a/b'), isFalse);
    expect(isValidAiProviderId('a?b'), isFalse);
    expect(isValidAiProviderId('../etc'), isFalse);
  });

  test('the alphabet is lower case', () {
    // The server lower-cases every provider id it stores, so the id shown in
    // the list is the lower-cased one no matter what was typed. This validator
    // therefore describes the STORED form; the add form folds case before
    // calling it, so "DeepSeek" is accepted as `deepseek` rather than refused.
    // What must not happen is an id passing here that the server would then
    // store under a different name.
    expect(isValidAiProviderId('DeepSeek'), isFalse);
    expect(isValidAiProviderId('DeepSeek'.toLowerCase()), isTrue);
  });

  test('a name may not start with punctuation', () {
    // A leading dot or dash reads as a flag or a hidden file everywhere else it
    // will be seen, and buys nothing.
    expect(isValidAiProviderId('-deepseek'), isFalse);
    expect(isValidAiProviderId('.deepseek'), isFalse);
  });
}
