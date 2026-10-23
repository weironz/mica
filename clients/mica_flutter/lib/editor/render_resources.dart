part of 'render.dart';

extension _RenderResources on RenderDocument {
  Rect _resourceClip(Rect canvasClip) {
    var clip = canvasClip;
    RenderObject child = this;
    RenderObject? ancestor = parent;
    while (ancestor != null) {
      final ancestorClip = ancestor.describeApproximatePaintClip(child) ??
          (ancestor is RenderView ? ancestor.paintBounds : null);
      if (ancestorClip != null) {
        final transform = getTransformTo(ancestor);
        if (transform.invert() != 0) {
          clip = clip.intersect(MatrixUtils.transformRect(transform, ancestorClip));
        }
      }
      child = ancestor;
      ancestor = ancestor.parent;
    }
    return clip;
  }
  void _invalidatePreviewGeometry(Set<PreviewResourceKey> keys) {
    if (keys.isEmpty) return;
    for (final key in keys) {
      for (final nodeId in _previewDependents[key] ?? const <String>{}) {
        _layoutCache.remove(nodeId);
      }
    }
    markNeedsLayout();
  }

  // Metrics outlive textures, but only while the current document references
  // their source. Editing a formula repeatedly must not grow this map forever.
  void _pruneResourceGeometry() {
    if (!_resourceReferencesDirty) return;
    _resourceReferencesDirty = false;
    _previewDependents.clear();
    final imageKeys = <String>{};
    for (final node in _nodes) {
      void depend(PreviewResourceKey key) =>
          (_previewDependents[key] ??= {}).add(node.id);
      for (final renderer
          in RenderDocument._renderersByKind[node.kind] ?? const []) {
        final key = renderer.previewKey(node);
        if (key != null) depend(key);
        final imageKey = renderer.imageKey(node);
        if (imageKey != null) imageKeys.add(imageKey);
      }
      if (node.isCode) continue;
      for (final mark in marksFromData(node.data)) {
        final renderer = _inlineAtomRenderers[mark.type];
        if (renderer == null) continue;
        final start = mark.start.clamp(0, node.text.length);
        final end = mark.end.clamp(start, node.text.length);
        final key = renderer.previewKey(node.text.substring(start, end));
        if (key != null) depend(key);
      }
    }
    for (final key in _previewDependents.keys) {
      final image = _previewImages[key.id]?[key.source];
      if (image != null) {
        _previewSizes.putIfAbsent(key, () => Size(image.width.toDouble(), image.height.toDouble()));
      }
      final baseline = _previewBaselines[key.id]?[key.source];
      if (baseline != null) _retainedBaselines.putIfAbsent(key, () => baseline);
    }
    for (final key in imageKeys) {
      final image = _images[key];
      if (image != null) _imageSizes.putIfAbsent(key, () => Size(image.width.toDouble(), image.height.toDouble()));
    }
    _previewSizes.removeWhere((key, _) => !_previewDependents.containsKey(key));
    _retainedBaselines.removeWhere(
      (key, _) => !_previewDependents.containsKey(key),
    );
    _imageSizes.removeWhere((key, _) => !imageKeys.contains(key));
  }

  // Called on every paint, including scroll-only paints and cached layouts.
  // Waiting for measure() to miss would strand source/placeholder layouts that
  // were cached before their resources first entered the viewport.
  void _publishResourceDemand(Rect clip) {
    final previews = <PreviewResourceKey, double>{};
    final images = <String>{};
    // An unbounded recording/capture is not an interactive viewport. It must
    // not accidentally enqueue and pin every resource in a huge document.
    if (!clip.isEmpty && clip.isFinite && clip.height < 1e7 && clip.width < 1e7) {
      final near = Rect.fromLTRB(
        clip.left,
        clip.top - RenderDocument._cullSlack,
        clip.right,
        clip.bottom + RenderDocument._cullSlack,
      );
      for (var index = 0; index < _layouts.length; index++) {
        final layout = _layouts[index];
        if (!_nodeVisible(layout)) continue;
        final node = _nodes[index];
        for (final renderer
            in RenderDocument._renderersByKind[node.kind] ?? const []) {
          final key = renderer.previewKey(node);
          if (key != null) previews[key] = renderer.previewWidth(this);
          final imageKey = renderer.imageKey(node);
          if (imageKey != null) images.add(imageKey);
        }
        if (node.isCode || layout.renderedBy != null) continue;
        final origin = Offset(layout.contentLeft, layout.textTop);
        for (final mark in marksFromData(node.data)) {
          final renderer = _inlineAtomRenderers[mark.type];
          if (renderer == null) continue;
          final start = mark.start.clamp(0, node.text.length);
          final end = mark.end.clamp(start, node.text.length);
          if (end <= start) continue;
          final source = node.text.substring(start, end);
          final key = renderer.previewKey(source);
          if (key == null) continue;
          final from = layout.fold?.docToPainter(start) ?? start;
          final to =
              layout.fold?.docToPainter(end, ceilInsideAtom: true) ?? end;
          final boxes = layout.painter.getBoxesForSelection(
            TextSelection(baseOffset: from, extentOffset: to),
          );
          if (boxes.any((box) => box.toRect().shift(origin).overlaps(near))) {
            previews[key] = 0;
          }
        }
      }
    }
    final demand = EditorResourceDemand(previews: previews, images: images);
    final consumer = onResourceDemand;
    if (consumer != null) {
      consumer(demand); // bookkeeping only; consumers defer actual work
    } else if (previews.isNotEmpty || images.isNotEmpty) {
      // Preserve standalone DocumentSurface callback clients. Production uses
      // the snapshot hook above so it can pin the entire set before producing.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!attached) return;
        for (final entry in previews.entries) {
          onRequestPreview?.call(entry.key.id, entry.key.source, entry.value);
        }
        for (final key in images) {
          onRequestImage?.call(key);
        }
      });
    }
  }
}
