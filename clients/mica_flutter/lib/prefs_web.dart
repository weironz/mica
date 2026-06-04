// ignore: avoid_web_libraries_in_flutter
import 'dart:html' as html;

const _prefix = 'mica.';

String? loadPref(String key) => html.window.localStorage['$_prefix$key'];

void savePref(String key, String value) =>
    html.window.localStorage['$_prefix$key'] = value;

void removePref(String key) => html.window.localStorage.remove('$_prefix$key');
