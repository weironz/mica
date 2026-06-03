/// Non-web stub: rich clipboard paste is a web-only capability.
library;

/// Returns true if the paste was consumed by the handler.
/// [markdown] is the structured conversion (HTML→Markdown, or the plain text);
/// [plain] is the raw `text/plain` flavor (used verbatim inside code blocks).
typedef RichPasteHandler = bool Function(String markdown, String plain, bool rich);

void setRichPasteHandler(RichPasteHandler? handler) {}
