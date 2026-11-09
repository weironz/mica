allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

val newBuildDir: Directory =
    rootProject.layout.buildDirectory
        .dir("../../build")
        .get()
rootProject.layout.buildDirectory.value(newBuildDir)

subprojects {
    val newSubprojectBuildDir: Directory = newBuildDir.dir(project.name)
    project.layout.buildDirectory.value(newSubprojectBuildDir)
}
// Plugins pinned in pubspec.lock (file_picker 8.x) still compile against
// API 34, while androidx dependencies they share now require 36. Compiling
// every plugin library against the app's SDK fixes that without changing any
// plugin's behaviour: minSdk / targetSdk stay what each declares.
// Registered BEFORE evaluationDependsOn below, which evaluates subprojects.
subprojects {
    pluginManager.withPlugin("com.android.library") {
        // finalizeDsl: after the plugin's own script set its compileSdk, before
        // AGP locks the DSL (afterEvaluate is already too late).
        extensions
            .getByType(com.android.build.api.variant.LibraryAndroidComponentsExtension::class.java)
            .finalizeDsl { it.compileSdk = 36 }
    }
}
subprojects {
    project.evaluationDependsOn(":app")
}

tasks.register<Delete>("clean") {
    delete(rootProject.layout.buildDirectory)
}
