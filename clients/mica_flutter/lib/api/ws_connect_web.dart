// Web: the token is dropped from the URL entirely — the session cookie
// authenticates the handshake.
//
// The browser `WebSocket` constructor takes a URL and subprotocols, no headers,
// by specification. That much is true, and it is why this file used to leave a
// JWT in the query string with a comment explaining that there was nowhere else
// to put it.
//
// There was. **The handshake is an ordinary HTTP request**, so the browser
// attaches same-origin cookies to it without being asked, and the server reads
// `mica_session` there (`ws.rs` `token_from_request`). "A browser cannot
// authenticate a WebSocket" is only true of CUSTOM headers; taking it as true of
// cookies as well is what kept the token in the URL. Same shape AFFiNE uses,
// under the same constraint.
//
// So on web the URL carries no credential at all — strictly better than the
// desktop header, which at least exists in a request the proxy may log.
import 'package:web_socket_channel/web_socket_channel.dart';

/// Strip the token: on web it must not travel in the URL, and there is no header
/// to move it to. The cookie is already going along.
///
/// Returns a null token — not because there is none, but because this side has
/// nothing to do with it. Same shape as the IO variant so both platforms answer
/// the same question, and a test can state the difference as a fact.
({Uri uri, String? token}) splitSocketAuth(Uri uri) {
  if (!uri.queryParameters.containsKey('token')) return (uri: uri, token: null);
  final rest = Map<String, String>.from(uri.queryParameters)..remove('token');
  return (
    uri: rest.isEmpty
        ? Uri(scheme: uri.scheme, host: uri.host, port: uri.hasPort ? uri.port : null, path: uri.path)
        : uri.replace(queryParameters: rest),
    token: null,
  );
}

WebSocketChannel connectAuthedSocket(Uri uri) =>
    WebSocketChannel.connect(splitSocketAuth(uri).uri);
