// Opening an authenticated WebSocket, with the token wherever this platform can
// actually put it.
//
// The token used to ride in the query string on every platform, which means a
// plaintext JWT in reverse-proxy access logs and in browser history — the log
// line outlives the session it came from, and nobody redacts URLs.
//
// The server has always preferred `Authorization: Bearer` and only fallen back
// to `?token=` (`ws.rs` `token_from_request`, and its two tests). So on IO this
// is a client-side fix alone: move the token to the header the server already
// reads. On web it is NOT — the browser `WebSocket` API cannot set request
// headers at all, so the query string stays there, and that half needs a
// different mechanism (a first-frame handshake, or a short-lived single-use
// ticket). Deliberately out of scope here rather than half-done.
//
// The split is the usual `_stub`/`_web` conditional import: `dart:io` cannot be
// named on web, and `IOWebSocketChannel` would drag it in.
export 'ws_connect_stub.dart' if (dart.library.js_interop) 'ws_connect_web.dart';
