// Web: the token stays in the query string, because there is nowhere else.
//
// The browser `WebSocket` constructor takes a URL and subprotocols — no headers,
// by specification, not by omission. So `Authorization` is simply unavailable
// here and the URL is the only channel the handshake has.
//
// This is not "web is fine": a JWT in the URL is logged by whatever sits in
// front of the server, same as anywhere. What differs is the fix — it needs a
// protocol change (a first message carrying the token, or a short-lived
// single-use ticket minted over HTTPS and burned on connect), which is a server
// change too and belongs in its own commit. Leaving it visible and unchanged
// beats pretending a header call site made it safe.
import 'package:web_socket_channel/web_socket_channel.dart';

/// No-op on web: the token cannot leave the URL, so nothing is split out.
///
/// Present so both variants expose the same surface and a test can state the
/// platform difference as a fact rather than a comment.
({Uri uri, String? token}) splitSocketAuth(Uri uri) => (uri: uri, token: null);

WebSocketChannel connectAuthedSocket(Uri uri) => WebSocketChannel.connect(uri);
