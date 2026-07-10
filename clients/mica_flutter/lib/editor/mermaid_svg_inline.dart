// Flattens merman's scoped CSS `<style>` block into per-element inline `style`
// attributes so a CSS-unaware rasterizer (flutter_svg) renders the theme.
//
// Why this exists: merman emits Mermaid-parity SVG whose fills/strokes/widths
// live in a stylesheet using descendant selectors (`#merman .node rect{fill:..}`).
// flutter_svg honors geometry + presentation attributes but ignores `<style>`,
// so without this every shape falls back to a black fill. merman documents
// element/inline-style rewriting as a host-side boundary (the Zed integration
// does the same), so this is the intended integration seam, not a workaround.
//
// Scope: a deliberately small CSS subset — comma selector lists, descendant
// combinators, `tag` / `.class` / `#id` compounds, plain `prop:value` decls.
// That covers Mermaid's generated theme CSS; anything fancier is ignored. On any
// parse error we return the SVG untouched (flutter_svg still draws the geometry).
//
// `<marker>` defs can't be rendered by flutter_svg, so we synthesize arrowheads
// ourselves (see [_synthesizeArrowheads]): for every edge that references a
// marker, we clone the marker's shapes into a `<g>` positioned at the edge
// endpoint and rotated along the edge's direction, then drop the marker defs.
import 'dart:math' as math;

import 'package:xml/xml.dart';

/// Inline merman's `<style>` CSS into element `style` attributes. Returns the
/// rewritten SVG, or [svg] unchanged if anything goes wrong.
String inlineMermaidCss(String svg) {
  try {
    final doc = XmlDocument.parse(svg);

    // Extract + remove all <style> blocks.
    final styleEls = doc.findAllElements('style').toList();
    final css = StringBuffer();
    for (final s in styleEls) {
      css.write(s.innerText);
      s.remove();
    }

    // Inline the stylesheet (if any). Some diagram types — class, er — carry no
    // <style> and style their elements inline instead; those skip this loop but
    // STILL need the font-weight sanitization below, so this is not an early
    // return.
    final rules = _parseCss(css.toString());
    for (final el in rules.isEmpty
        ? const <XmlElement>[]
        : doc.descendants.whereType<XmlElement>()) {
      final ancestors = el.ancestors.whereType<XmlElement>().toList();
      // Winning declaration per property: chosen[prop] = [specificity, order].
      final chosen = <String, List<int>>{};
      final values = <String, String>{};
      for (final r in rules) {
        if (!_matches(el, ancestors, r.selector)) continue;
        r.decls.forEach((prop, value) {
          final cur = chosen[prop];
          if (cur == null ||
              r.specificity > cur[0] ||
              (r.specificity == cur[0] && r.order >= cur[1])) {
            chosen[prop] = [r.specificity, r.order];
            values[prop] = value;
          }
        });
      }
      if (values.isEmpty) continue;

      // Inline `style` attribute wins over the stylesheet; presentation
      // attributes lose to it (standard CSS cascade) — so we emit into `style`.
      final existing = _parseDecls(el.getAttribute('style') ?? '');
      final merged = <String, String>{};
      values.forEach((k, v) {
        if (!existing.containsKey(k)) merged[k] = v;
      });
      if (merged.isEmpty) continue;
      merged.addAll(existing); // existing inline props keep priority
      el.setAttribute(
        'style',
        merged.entries.map((e) => '${e.key}:${e.value}').join(';'),
      );
    }

    // Synthesize arrowheads from <marker> refs (markers are styled by the loop
    // above first), then drop the now-unrenderable marker/defs scaffolding.
    _synthesizeArrowheads(doc);
    for (final m in doc.findAllElements('marker').toList()) {
      m.remove();
    }
    for (final d in doc.findAllElements('defs').toList()) {
      if (d.childElements.isEmpty) d.remove();
    }

    // Materialize the theme's canvas colour. merman puts it in the root <svg>'s
    // CSS `background-color` (e.g. `background-color:white`), but flutter_svg
    // only paints SHAPES — it silently drops CSS backgrounds. Without this the
    // raster is transparent, so the diagram is not self-contained: whatever the
    // host paints behind the PNG becomes the area "around" the nodes (solid
    // black on any dark surface). Emit that colour as an opaque <rect> covering
    // the viewBox, inserted first so it paints behind everything.
    _materializeBackground(doc.rootElement);

    // flutter_svg parses font-weight strictly and THROWS on the relative
    // keywords `bolder`/`lighter` (Mermaid's class/er themes use them). Map them
    // to absolute weights in the serialized output — in style decls and as
    // attributes — or the whole picture fails to decode.
    return doc
        .toXmlString()
        .replaceAll('font-weight:bolder', 'font-weight:bold')
        .replaceAll('font-weight="bolder"', 'font-weight="bold"')
        .replaceAll('font-weight:lighter', 'font-weight:normal')
        .replaceAll('font-weight="lighter"', 'font-weight="normal"');
  } catch (_) {
    return svg;
  }
}

/// Paint the root `<svg>`'s CSS `background-color` as an opaque background rect
/// covering the viewBox (flutter_svg ignores CSS backgrounds). No-op when the
/// colour is absent/transparent, so themes that intend a see-through canvas are
/// respected.
void _materializeBackground(XmlElement root) {
  final bg = _parseDecls(root.getAttribute('style') ?? '')['background-color'];
  if (bg == null) return;
  final color = bg.trim();
  if (color.isEmpty ||
      color == 'transparent' ||
      color == 'none' ||
      color.startsWith('rgba(0, 0, 0, 0') ||
      color.startsWith('rgba(0,0,0,0')) {
    return;
  }
  // Cover the viewBox (handles a non-zero min-x/min-y); fall back to width/
  // height for the rare renderer that omits a viewBox.
  double x = 0, y = 0, w = 0, h = 0;
  final vb = root.getAttribute('viewBox');
  if (vb != null) {
    final v =
        _numRe.allMatches(vb).map((m) => double.parse(m.group(0)!)).toList();
    if (v.length == 4) {
      x = v[0];
      y = v[1];
      w = v[2];
      h = v[3];
    }
  }
  if (w <= 0 || h <= 0) {
    w = double.tryParse(root.getAttribute('width') ?? '') ?? 0;
    h = double.tryParse(root.getAttribute('height') ?? '') ?? 0;
  }
  if (w <= 0 || h <= 0) return;
  String n(double v) => v.toStringAsFixed(3);
  root.children.insert(
    0,
    XmlElement(XmlName('rect'), [
      XmlAttribute(XmlName('x'), n(x)),
      XmlAttribute(XmlName('y'), n(y)),
      XmlAttribute(XmlName('width'), n(w)),
      XmlAttribute(XmlName('height'), n(h)),
      XmlAttribute(XmlName('fill'), color),
    ]),
  );
}

class _Rule {
  _Rule(this.selector, this.decls, this.specificity, this.order);
  final List<_Compound> selector; // descendant chain, left → right
  final Map<String, String> decls;
  final int specificity;
  final int order;
}

class _Compound {
  _Compound(this.tag, this.id, this.classes);
  final String? tag;
  final String? id;
  final List<String> classes;

  int get specificity =>
      (id != null ? 100 : 0) + classes.length * 10 + (tag != null ? 1 : 0);
}

final _comment = RegExp(r'/\*.*?\*/', dotAll: true);
final _ruleRe = RegExp(r'([^{}]+)\{([^{}]*)\}');
final _ws = RegExp(r'\s+');

List<_Rule> _parseCss(String css) {
  final cleaned = css.replaceAll(_comment, '');
  final rules = <_Rule>[];
  var order = 0;
  for (final m in _ruleRe.allMatches(cleaned)) {
    final selectorList = m.group(1)!;
    final decls = _parseDecls(m.group(2)!);
    if (decls.isEmpty) continue;
    for (final selector in selectorList.split(',')) {
      final compounds = _parseSelector(selector);
      if (compounds.isEmpty) continue;
      final spec = compounds.fold<int>(0, (a, c) => a + c.specificity);
      rules.add(_Rule(compounds, decls, spec, order++));
    }
  }
  return rules;
}

List<_Compound> _parseSelector(String selector) {
  // Treat child/sibling combinators as descendants (good enough for Mermaid).
  final normalized =
      selector.replaceAll('>', ' ').replaceAll('+', ' ').replaceAll('~', ' ');
  final out = <_Compound>[];
  for (final token in normalized.trim().split(_ws)) {
    if (token.isEmpty || token == '*') continue;
    final c = _parseCompound(token);
    if (c == null) return const []; // unsupported token → skip whole selector
    out.add(c);
  }
  return out;
}

final _tagRe = RegExp(r'^([a-zA-Z][\w-]*)');
final _idRe = RegExp(r'#([\w-]+)');
final _classRe = RegExp(r'\.([\w-]+)');

_Compound? _parseCompound(String token) {
  String? tag;
  String? id;
  final classes = <String>[];
  final tagM = _tagRe.firstMatch(token);
  if (tagM != null) tag = tagM.group(1);
  final idM = _idRe.firstMatch(token);
  if (idM != null) id = idM.group(1);
  for (final cm in _classRe.allMatches(token)) {
    classes.add(cm.group(1)!);
  }
  // Anything that isn't a tag/id/class (pseudo-classes, attribute selectors)
  // makes the selector unsupported.
  final consumed = (tag?.length ?? 0) +
      (id != null ? id.length + 1 : 0) +
      classes.fold<int>(0, (a, c) => a + c.length + 1);
  if (consumed != token.length) return null;
  if (tag == null && id == null && classes.isEmpty) return null;
  return _Compound(tag, id, classes);
}

Map<String, String> _parseDecls(String body) {
  final out = <String, String>{};
  for (final part in body.split(';')) {
    final i = part.indexOf(':');
    if (i <= 0) continue;
    final prop = part.substring(0, i).trim();
    var value = part.substring(i + 1).trim();
    if (prop.isEmpty || value.isEmpty) continue;
    value = value.replaceAll('!important', '').trim();
    if (value.isEmpty) continue;
    out[prop] = value;
  }
  return out;
}

bool _matchesCompound(XmlElement el, _Compound c) {
  if (c.tag != null && el.localName != c.tag) return false;
  if (c.id != null && el.getAttribute('id') != c.id) return false;
  if (c.classes.isNotEmpty) {
    final cls = (el.getAttribute('class') ?? '').split(_ws);
    for (final want in c.classes) {
      if (!cls.contains(want)) return false;
    }
  }
  return true;
}

bool _matches(XmlElement el, List<XmlElement> ancestors, List<_Compound> sel) {
  if (!_matchesCompound(el, sel.last)) return false;
  // Greedy descendant match: each preceding compound must match an ancestor,
  // scanning from nearest to furthest.
  var ai = 0;
  for (var i = sel.length - 2; i >= 0; i--) {
    var found = false;
    while (ai < ancestors.length) {
      if (_matchesCompound(ancestors[ai++], sel[i])) {
        found = true;
        break;
      }
    }
    if (!found) return false;
  }
  return true;
}

// ── Arrowhead synthesis ──────────────────────────────────────────────────────

typedef _Pt = ({double x, double y});

/// Geometry of an edge: its two endpoints and the tangent direction (radians)
/// at each, used to place + rotate the marker.
class _EdgeGeom {
  _EdgeGeom(this.start, this.end, this.startAngle, this.endAngle);
  final _Pt start;
  final _Pt end;
  final double startAngle;
  final double endAngle;
}

/// For each edge that references a `<marker>` (via `marker-end`/`marker-start`),
/// clone the marker's shapes into a positioned, rotated, scaled `<g>` inserted
/// next to the edge — emulating SVG marker rendering, which flutter_svg lacks.
/// Best-effort: any edge we can't parse just keeps its plain line.
void _synthesizeArrowheads(XmlDocument doc) {
  final markers = <String, XmlElement>{};
  for (final m in doc.findAllElements('marker')) {
    final id = m.getAttribute('id');
    if (id != null) markers[id] = m;
  }
  if (markers.isEmpty) return;

  final edges = doc.descendants.whereType<XmlElement>().where((e) {
    return e.getAttribute('marker-end') != null ||
        e.getAttribute('marker-start') != null;
  }).toList();

  for (final edge in edges) {
    try {
      final geom = _edgeGeometry(edge);
      if (geom == null) continue;
      final strokeWidth = _edgeStrokeWidth(edge);
      final parent = edge.parent;
      if (parent is! XmlElement) continue;

      for (final end in const [true, false]) {
        final ref = edge.getAttribute(end ? 'marker-end' : 'marker-start');
        if (ref == null) continue;
        final marker = markers[_urlRef(ref)];
        if (marker == null) continue;
        final at = end ? geom.end : geom.start;
        final angle = end ? geom.endAngle : geom.startAngle;
        final g = _buildArrowhead(marker, at, angle, strokeWidth);
        if (g == null) continue;
        final idx = parent.children.indexOf(edge);
        parent.children.insert(idx + 1, g);
      }
    } catch (_) {
      // Skip this edge's arrowheads; the line itself still renders.
    }
  }
}

final _numRe = RegExp(r'-?\d*\.?\d+(?:[eE]-?\d+)?');

_EdgeGeom? _edgeGeometry(XmlElement edge) {
  if (edge.localName == 'line') {
    final x1 = double.tryParse(edge.getAttribute('x1') ?? '');
    final y1 = double.tryParse(edge.getAttribute('y1') ?? '');
    final x2 = double.tryParse(edge.getAttribute('x2') ?? '');
    final y2 = double.tryParse(edge.getAttribute('y2') ?? '');
    if (x1 == null || y1 == null || x2 == null || y2 == null) return null;
    final start = (x: x1, y: y1);
    final pEnd = (x: x2, y: y2);
    final fwd = math.atan2(y2 - y1, x2 - x1);
    return _EdgeGeom(start, pEnd, fwd, fwd);
  }
  final d = edge.getAttribute('d');
  if (d == null) return null;
  // Mermaid edge paths are absolute M/L/C — every coordinate comes in x,y pairs,
  // so naive pairing yields the on-path skeleton (curve control points included,
  // which still give a correct tangent at the ends).
  final nums =
      _numRe.allMatches(d).map((m) => double.parse(m.group(0)!)).toList();
  if (nums.length < 4 || nums.length.isOdd) return null;
  final pts = <_Pt>[];
  for (var i = 0; i + 1 < nums.length; i += 2) {
    pts.add((x: nums[i], y: nums[i + 1]));
  }
  if (pts.length < 2) return null;
  final start = pts.first;
  final afterStart = pts[1];
  final endPt = pts.last;
  final beforeEnd = pts[pts.length - 2];
  final startAngle =
      math.atan2(afterStart.y - start.y, afterStart.x - start.x);
  final endAngle = math.atan2(endPt.y - beforeEnd.y, endPt.x - beforeEnd.x);
  return _EdgeGeom(start, endPt, startAngle, endAngle);
}

double _edgeStrokeWidth(XmlElement edge) {
  final style = _parseDecls(edge.getAttribute('style') ?? '');
  final raw = style['stroke-width'] ?? edge.getAttribute('stroke-width');
  if (raw == null) return 1;
  final m = _numRe.firstMatch(raw);
  return m == null ? 1 : (double.tryParse(m.group(0)!) ?? 1);
}

String? _urlRef(String value) {
  final m = RegExp(r'url\(\s*#([^)\s]+)\s*\)').firstMatch(value);
  return m?.group(1);
}

/// Build a `<g>` containing the marker's cloned shapes, transformed so the
/// marker's reference point lands on [at] and its content points along [angle].
XmlElement? _buildArrowhead(
  XmlElement marker,
  _Pt at,
  double angle,
  double strokeWidth,
) {
  final shapes =
      marker.children.whereType<XmlElement>().map((e) => e.copy()).toList();
  if (shapes.isEmpty) return null;

  final refX = double.tryParse(marker.getAttribute('refX') ?? '') ?? 0;
  final refY = double.tryParse(marker.getAttribute('refY') ?? '') ?? 0;
  final mw = double.tryParse(marker.getAttribute('markerWidth') ?? '') ?? 3;
  final mh = double.tryParse(marker.getAttribute('markerHeight') ?? '') ?? 3;

  // Scale from the marker's content space to its viewport. With a viewBox the
  // content is scaled to markerWidth×markerHeight; without one, the content is
  // 1:1 and (for the default strokeWidth units) scaled by the line width.
  double sx = 1, sy = 1;
  final vb = marker.getAttribute('viewBox');
  if (vb != null) {
    final v = _numRe.allMatches(vb).map((m) => double.parse(m.group(0)!)).toList();
    if (v.length == 4 && v[2] > 0 && v[3] > 0) {
      sx = mw / v[2];
      sy = mh / v[3];
    }
  } else if ((marker.getAttribute('markerUnits') ?? 'strokeWidth') ==
      'strokeWidth') {
    sx = sy = strokeWidth;
  }

  final deg = angle * 180 / math.pi;
  String n(double v) => v.toStringAsFixed(3);
  final transform = 'translate(${n(at.x)},${n(at.y)}) rotate(${n(deg)}) '
      'scale(${n(sx)},${n(sy)}) translate(${n(-refX)},${n(-refY)})';

  // Marker shapes inherit fill/stroke from the marker element in SVG; once
  // lifted out we must carry that colour, or they'd default to black.
  final mStyle = _parseDecls(marker.getAttribute('style') ?? '');
  final fill = mStyle['fill'] ?? marker.getAttribute('fill') ?? '#333333';
  final stroke = mStyle['stroke'] ?? marker.getAttribute('stroke') ?? fill;

  return XmlElement(
    XmlName('g'),
    [
      XmlAttribute(XmlName('transform'), transform),
      XmlAttribute(XmlName('style'), 'fill:$fill;stroke:$stroke'),
    ],
    shapes,
  );
}
