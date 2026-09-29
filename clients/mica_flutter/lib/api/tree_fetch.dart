import 'dart:async';

import 'package:http/http.dart' as http;

import '../swallowed.dart';
import 'models.dart';

/// A tree-change bell runs this request without a caller awaiting it. Ordinary
/// HTTP failures are retried by the next bell or a manual refresh; programming
/// errors still escape so they remain visible during development.
Future<T?> fetchBackgroundTree<T>(Future<T> Function() fetch) async {
  try {
    return await fetch();
  } on ApiException {
    swallowed('views_tree_api');
    return null;
  } on http.ClientException {
    // package:http wraps IO socket and protocol errors in ClientException.
    swallowed('views_tree_network');
    return null;
  } on TimeoutException {
    swallowed('views_tree_timeout');
    return null;
  }
}
