// One Windows build runs the shared contract and actual application startup.
import 'markdown_codec_test.dart' as contract;
import 'markdown_codec_startup_test.dart' as startup;

void main() {
  contract.main();
  startup.main();
}
