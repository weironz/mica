// Desktop/IO: the token goes in the `Authorization` header, never the URL.
import 'package:web_socket_channel/io.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

/// Split a socket URI into the URI to request and the bearer token to send.
///
/// Callers build the URI with `?token=` because that is the one place that
/// already knows the token; taking it back out here keeps the change to a single
/// seam instead of threading a second argument through three constructors and a
/// `Future<Uri> Function()`.
///
/// Exposed (and tested) separately from [connectAuthedSocket] because the
/// interesting part is this rewrite — the connect itself is one library call.
({Uri uri, String? token}) splitSocketAuth(Uri uri) {
  final token = uri.queryParameters['token'];
  if (token == null || token.isEmpty) {
    return (uri: uri, token: null);
  }
  final rest = Map<String, String>.from(uri.queryParameters)..remove('token');
  return (uri: rest.isEmpty ? _withoutQuery(uri) : uri.replace(queryParameters: rest), token: token);
}

/// Rebuild [uri] with no query at all.
///
/// `replace(query: '')` and `replace(queryParameters: {})` both keep a bare
/// trailing `?` — an emptied query is not the same as no query, and the only way
/// to say the latter is to construct the URI without one. Caught by a test
/// rather than reasoned about: the `?` is invisible until something downstream
/// compares URLs.
Uri _withoutQuery(Uri uri) => Uri(
  scheme: uri.scheme,
  userInfo: uri.userInfo.isEmpty ? null : uri.userInfo,
  host: uri.host,
  port: uri.hasPort ? uri.port : null,
  path: uri.path,
  fragment: uri.hasFragment ? uri.fragment : null,
);

/// Connect with the token in a header the URL never sees.
WebSocketChannel connectAuthedSocket(Uri uri) {
  final split = splitSocketAuth(uri);
  if (split.token == null) {
    return WebSocketChannel.connect(uri);
  }
  return IOWebSocketChannel.connect(
    split.uri,
    headers: {'Authorization': 'Bearer ${split.token}'},
  );
}
