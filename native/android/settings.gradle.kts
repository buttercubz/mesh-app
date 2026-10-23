// Standalone Gradle build of the Android engine library (WP-16, DESIGN §2.4, §10.2).
// The app (apps/mobile/android, WP-18) includes this build and substitutes
// `com.convlink.voice:meshengine` with project(":meshengine").
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "meshengine-build"
include(":meshengine")
