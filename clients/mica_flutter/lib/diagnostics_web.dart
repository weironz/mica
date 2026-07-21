/// Web has no filesystem to drop capture files into, so diagnostics is simply
/// absent there: `diagnosticsSupported` is false and Settings hides the whole
/// section rather than offering a switch that does nothing.
///
/// Mirrors `diagnostics_stub.dart` member for member — the conditional import
/// means a name missing here is a web-build failure, which is the intended
/// enforcement.
library;

const bool diagnosticsSupported = false;

bool get diagnosticsOn => false;

void setDiagnostics(bool on) {}

String get diagnosticsDir => '';

void captureIo(String kind, String ext, String input, String output) {}

void trace(String line) {}

Future<void> openDiagnosticsFolder() async {}
