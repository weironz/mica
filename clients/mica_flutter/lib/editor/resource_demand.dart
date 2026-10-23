/// Identity of a raster shared by block, inline and popup renderers.
typedef PreviewResourceKey = ({String id, String source});

/// Resources intersecting the current paint clip plus the editor's near band.
/// A complete snapshot, including empty sets when the document leaves view.
class EditorResourceDemand {
  EditorResourceDemand({
    Map<PreviewResourceKey, double> previews = const {},
    Set<String> images = const {},
  }) : previews = Map.unmodifiable(previews),
       images = Set.unmodifiable(images);

  final Map<PreviewResourceKey, double> previews;
  final Set<String> images;
}
