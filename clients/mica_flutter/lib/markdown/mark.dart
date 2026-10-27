/// Inline rich-text marks stored additively over a block's plain text (in
/// `data.marks`). The text stays clean; a mark is a `[start, end)` range with a
/// type (and an href for links). See docs/architecture/editor-engine.md.
class Mark {
  Mark(this.start, this.end, this.type, {this.href, this.title});

  final int start;
  final int end;
  final String type; // bold | italic | code | strike | link
  final String? href;

  /// Optional link title (`[t](url "title")`) — carried for GFM fidelity.
  final String? title;

  Mark shifted(int delta) => withRange(start + delta, end + delta);

  /// The single re-range constructor: every mark transformation (shift, split,
  /// concat, normalize) rebuilds through this, so no structured field — `title`
  /// today, anything added tomorrow — is dropped by a hand-copied `Mark(…)`.
  Mark withRange(int start, int end) =>
      Mark(start, end, type, href: href, title: title);

  Map<String, dynamic> toJson() => {
    'start': start,
    'end': end,
    'type': type,
    if (href != null) 'href': href,
    if (title != null) 'title': title,
  };

  static const types = {'bold', 'italic', 'code', 'strike', 'link', 'footnote'};
}
