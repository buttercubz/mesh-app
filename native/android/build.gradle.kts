// Top-level build of the engine library (WP-16). Versions match the app build
// (apps/mobile/android/build.gradle): AGP 8.13.2, Kotlin 2.3.21, JDK 21.
plugins {
    id("com.android.library") version "8.13.2" apply false
    id("org.jetbrains.kotlin.android") version "2.3.21" apply false
}

allprojects {
    group = "com.convlink.voice"
    version = "0.1.0"
}
