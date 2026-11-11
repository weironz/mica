plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

// Secrets come from the caller/runner environment, never a tracked properties file.
val signingNames = listOf(
    "MICA_ANDROID_KEYSTORE_PATH",
    "MICA_ANDROID_KEY_ALIAS",
    "MICA_ANDROID_STORE_PASSWORD",
    "MICA_ANDROID_KEY_PASSWORD",
)
val signingValues = signingNames.associateWith { System.getenv(it).orEmpty() }
val stableSigning = signingValues.values.all { it.isNotBlank() }
val signingFile = signingValues.getValue("MICA_ANDROID_KEYSTORE_PATH")
    .takeIf { it.isNotBlank() }?.let { file(it) }
val signingType = System.getenv("MICA_ANDROID_STORE_TYPE").orEmpty().ifEmpty { "JKS" }.uppercase()
require(signingType in setOf("JKS", "PKCS12")) { "MICA_ANDROID_STORE_TYPE must be JKS or PKCS12" }
if (signingValues.values.any { it.isNotBlank() } && !stableSigning) {
    throw GradleException("Stable signing configuration is incomplete: set all four MICA_ANDROID signing variables")
}
if (stableSigning && (signingFile?.isFile != true || signingFile?.canRead() != true)) {
    throw GradleException("MICA_ANDROID_KEYSTORE_PATH must name a readable keystore file")
}
// Never silently sign a release with a machine-specific debug key. Task-graph
// validation also catches aggregate tasks such as `assemble`, not only assembleRelease.
gradle.taskGraph.whenReady {
    if (!stableSigning && allTasks.any { it.name.contains("Release", ignoreCase = true) }) {
        throw GradleException("Release APK needs a stable signing key; configure MICA_ANDROID_KEYSTORE_PATH, KEY_ALIAS, STORE_PASSWORD and KEY_PASSWORD. Debug builds keep the standard debug key.")
    }
}

android {
    namespace = "io.github.weironz.mica_flutter"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        applicationId = "io.github.weironz.mica"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    signingConfigs {
        if (stableSigning) {
            create("stableRelease") {
                storeFile = signingFile
                storePassword = signingValues.getValue("MICA_ANDROID_STORE_PASSWORD")
                keyAlias = signingValues.getValue("MICA_ANDROID_KEY_ALIAS")
                keyPassword = signingValues.getValue("MICA_ANDROID_KEY_PASSWORD")
                storeType = signingType
            }
        }
    }
    buildTypes {
        release {
            signingConfig = if (stableSigning) signingConfigs.getByName("stableRelease") else null
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}
