part of 'editor.dart';

extension _EditorResources on _MicaEditorState {
  // Called during paint. Pin the complete set before any asynchronous producer
  // can finish; actual fetch/decode and cache trimming happen after the frame.
  void _onResourceDemand(EditorResourceDemand demand) {
    _previews.setDemand(demand.previews);
    _wantedImages = demand.images;
    if (_imageDemandScheduled) return;
    _imageDemandScheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _imageDemandScheduled = false;
      if (!mounted) return;
      if (_evictImages('')) _resourceSetState(() {});
      for (final key in _wantedImages) {
        _requestImage(key);
      }
    });
  }

  /// Lazily fetch/decode a key from the current paint demand.
  void _requestImage(String fileId) {
    if (_imageCache.containsKey(fileId) ||
        _imageLoading.contains(fileId) ||
        _imageErrors.contains(fileId)) {
      return;
    }
    final isUrl = fileId.startsWith('http://') || fileId.startsWith('https://');
    if (isUrl && _rehostPending.contains(fileId)) return;
    final load = widget.onLoadImageBytes;
    if (load == null) return;
    _imageLoading.add(fileId);
    load(fileId).then(
      (bytes) async {
        if (!mounted) return;
        if (!_wantedImages.contains(fileId) && fileId != _viewerKey) {
          _imageLoading.remove(fileId);
          return;
        }
        if (bytes == null) {
          _imageLoading.remove(fileId);
          _resourceSetState(() => _imageErrors.add(fileId));
          return;
        }
        try {
          await _decodeInto(fileId, bytes);
        } catch (error, stack) {
          if (!mounted) return;
          debugPrint('mica: image decode failed for "$fileId": $error\n$stack');
          _imageLoading.remove(fileId);
          _resourceSetState(() => _imageErrors.add(fileId));
        }
      },
      onError: (Object error, StackTrace stack) {
        if (!mounted) return;
        _imageLoading.remove(fileId);
        _resourceSetState(() => _imageErrors.add(fileId));
      },
    );
  }

  /// Also used by image insertion, when bytes are already present locally.
  Future<void> _decodeInto(String key, Uint8List bytes) async {
    // Claim synchronously, before insertion causes its first layout/paint.
    _imageLoading.add(key);
    // Keep decodeCapped's descriptor/codec ownership contract (lessons.md §5).
    final capPx = mounted
        ? View.of(context).physicalSize.width.round().clamp(2048, 4096)
        : 2048;
    final codec = await decodeCapped(bytes, capPx);
    if (!mounted) {
      codec.dispose();
      return;
    }
    if (codec.frameCount > 1) {
      _imageAnims.remove(key)?.dispose();
      _imagePainted.add(key);
      final anim = ImageAnimator(
        CodecFrameSource(codec),
        onFrame: (frame) => _onImageFrame(key, frame),
      );
      _imageAnims[key] = anim;
      anim.start();
      return;
    }
    final frame = await codec.getNextFrame();
    codec.dispose();
    if (!mounted) {
      frame.image.dispose();
      return;
    }
    final previous = _imageCache[key];
    _imageLoading.remove(key);
    _imageErrors.remove(key);
    _resourceSetState(() {
      _imageCache[key] = frame.image;
      _touchImage(key);
      _evictImages(key);
    });
    if (previous != null) _releaseImage(previous);
  }

  void _onImageFrame(String key, ui.Image frame) {
    if (!mounted) {
      frame.dispose();
      return;
    }
    if (!_imageCache.containsKey(key)) {
      _imageLoading.remove(key);
      _imageErrors.remove(key);
      _resourceSetState(() {
        _imageCache[key] = frame;
        _touchImage(key);
        _evictImages(key);
      });
      return;
    }
    final previous = _imageCache[key];
    _imageCache[key] = frame;
    _render?.replaceImage(key, frame);
    _imageFrameTick.value++;
    if (previous != null) _releaseImage(previous);
    if (!_imagePainted.remove(key)) _imageAnims[key]?.pause();
  }

  void _onImagePainted(String key) {
    _touchImage(key); // existing paint-touch LRU, shared with the demand pins
    _imagePainted.add(key);
    final anim = _imageAnims[key];
    if (anim != null && !anim.isPlaying) anim.start();
  }

  void _touchImage(String key) {
    final image = _imageCache.remove(key);
    if (image != null) _imageCache[key] = image;
  }

  // 64 is a soft resident budget: every near key and the fullscreen viewer
  // stays alive, even when a dense/large viewport itself exceeds that count.
  // A cold animator owns a codec too; dispose it when its cache slot is evicted.
  bool _evictImages(String protect) {
    var changed = false;
    for (final key in _imageCache.keys.toList(growable: false)) {
      if (_imageCache.length <= _MicaEditorState._maxCachedImages) break;
      if (key == protect || key == _viewerKey || _wantedImages.contains(key)) {
        continue;
      }
      _imageAnims.remove(key)?.dispose();
      final image = _imageCache.remove(key);
      _imagePainted.remove(key);
      if (image != null) _releaseImage(image);
      changed = true;
    }
    return changed;
  }

  // First remove the owner/cache reference and notify the renderer, then
  // release the old handle after the frame. LRU order alone proves no safety.
  void _releaseImage(ui.Image image) {
    WidgetsBinding.instance.addPostFrameCallback((_) => image.dispose());
  }
}
