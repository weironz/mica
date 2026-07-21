// Custom bootstrap template: identical to the flutter-tools default (flutter.js
// + build config + loader.load with the service-worker version) EXCEPT that it
// points the engine's runtime font downloads (default Roboto + Noto fallback
// shards) at the same-origin mirror in web/gfonts/ instead of
// https://fonts.gstatic.com/s/ — the last gstatic dependency left after
// --no-web-resources-cdn moved CanvasKit same-origin. tool/gfonts/mirror.sh
// maintains the mirror; CI --checks it against the engine's shard list.
// Relative URL on purpose: resolves against <base href>, like every other asset.
{{flutter_js}}
{{flutter_build_config}}
_flutter.loader.load({
  serviceWorkerSettings: {
    serviceWorkerVersion: {{flutter_service_worker_version}}
  },
  config: {
    fontFallbackBaseUrl: "gfonts/"
  }
});
