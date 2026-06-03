// Browser image actions for the right-click menu: download to disk and copy the
// bitmap to the system clipboard. Web-only; no-ops elsewhere.
//
// `downloadImage(bytes, filename, mime)` triggers a browser download.
// `copyImageToClipboard(bytes, mime)` returns true on success (needs a secure
// context — https or localhost — so it may fail over a plain-http LAN address).
export 'image_actions_stub.dart' if (dart.library.html) 'image_actions_web.dart';
