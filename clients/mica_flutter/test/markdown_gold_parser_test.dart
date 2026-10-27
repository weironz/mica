import 'package:flutter_test/flutter_test.dart';
import 'markdown_codec_contract_body.dart';

void main() {
  test('gold envelope preserves the raw wire output', () {
    final gold = parseWireGold(
      r'{"function":"parse_body","cases":[{"id":"sample","args":{"markdown":"x","fragment":false},"output":"[{\"type\":\"paragraph\"}]"}]}',
    );
    expect(gold.function, 'parse_body');
    expect(gold.cases.single.id, 'sample');
    expect(gold.cases.single.args['fragment'], false);
    expect(gold.cases.single.output, '[{"type":"paragraph"}]');
  });
}
