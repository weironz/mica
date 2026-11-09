package io.github.weironz.mica_flutter

import android.os.Bundle
import android.system.Os
import io.flutter.embedding.android.FlutterActivity

class MainActivity : FlutterActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        // The app's private files directory, for the Dart side's local store
        // and preferences (configDir / LocalOffline._localDir). An Android app
        // process has no usable HOME, so the desktop rules would point at an
        // unwritable path. Set before super.onCreate starts the Dart VM, which
        // snapshots the environment.
        Os.setenv("MICA_DATA_DIR", filesDir.absolutePath, true)
        super.onCreate(savedInstanceState)
    }
}
