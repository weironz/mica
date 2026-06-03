// Rich clipboard paste: on web, intercept the native paste event to read the
// `text/html` clipboard flavor and convert it to Markdown so web-page content
// keeps its structure (like Typora). No-op off the web.
export 'rich_paste_stub.dart'
    if (dart.library.html) 'rich_paste_web.dart';
