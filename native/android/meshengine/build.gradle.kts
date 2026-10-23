// :meshengine, the Android engine library (WP-16, DESIGN §10.2). Kotlin 2.3, JDK 21,
// compile/target 36, minSdk 31, NDK 27.1.12297006, CMake 3.22.1 (Oboe shim in src/main/cpp).
// The Rust core ships as jniLibs/<abi>/libmesh_ffi.so (`cargo xtask build-android`); the
// UniFFI Kotlin bindings (com.convlink.voice.core, generated and committed) load it with JNA.
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

/** `-Pmeshengine.telecom=true` compiles the Core-Telecom strategy and its manifest overlay. */
val telecom = providers.gradleProperty("meshengine.telecom").map { it.toBoolean() }.getOrElse(false)

/** Repository root (native/android/meshengine → ../../..): host dylib for the JVM tests. */
val repoRoot: File = rootProject.projectDir.resolve("../..").canonicalFile

android {
    namespace = "com.convlink.voice.engine"
    compileSdk = 36
    ndkVersion = "27.1.12297006"

    defaultConfig {
        minSdk = 31
        consumerProguardFiles("consumer-rules.pro")
        buildConfigField("boolean", "TELECOM", telecom.toString())
        ndk {
            // Phones only: arm64 devices and the x86_64 emulator, like jniLibs (DESIGN §2.4).
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
        externalNativeBuild {
            cmake {
                arguments += listOf("-DANDROID_STL=c++_shared", "-DANDROID_SUPPORT_FLEXIBLE_PAGE_SIZES=ON")
                cppFlags += listOf("-std=c++17", "-fno-exceptions", "-fno-rtti")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    buildFeatures {
        buildConfig = true
        prefab = true
    }

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
            version = "3.22.1"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    sourceSets {
        getByName("main") {
            if (telecom) {
                kotlin.srcDir("src/telecom/kotlin")
            }
        }
        // The telecom manifest overlay merges on top of the main manifest through the
        // build-type source sets (a library has one main manifest).
        if (telecom) {
            getByName("debug").manifest.srcFile("src/telecom/AndroidManifest.xml")
            getByName("release").manifest.srcFile("src/telecom/AndroidManifest.xml")
        }
    }

    testOptions {
        unitTests {
            // android.jar stubs return defaults instead of throwing (pure logic is tested).
            isReturnDefaultValues = true
            all { test ->
                // Generated bindings run against the host libmesh_ffi.dylib (DESIGN §13).
                test.systemProperty("jna.library.path", repoRoot.resolve("target/release").path)
                test.systemProperty("meshengine.repoRoot", repoRoot.path)
                test.systemProperty("meshengine.telecom", telecom.toString())
            }
        }
    }

    lint {
        abortOnError = true
        checkReleaseBuilds = false
        // The generated UniFFI bindings are not ours to lint.
        lintConfig = rootProject.file("gradle/lint.xml")
    }

    packaging {
        jniLibs {
            useLegacyPackaging = false
        }
    }
}

kotlin {
    jvmToolchain(21)
}

dependencies {
    implementation("androidx.core:core-ktx:1.17.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("com.google.oboe:oboe:1.10.0")
    implementation("com.google.android.gms:play-services-code-scanner:16.1.0")
    if (telecom) {
        implementation("androidx.core:core-telecom:1.0.0")
    }

    testImplementation("junit:junit:4.13.2")
    testImplementation("net.java.dev.jna:jna:5.17.0")
    testImplementation("org.json:json:20240303")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
}

// Acceptance (WORKPLAN WP-16): the merged library manifest of the default variant declares
// neither ACCESS_LOCAL_NETWORK nor the telecom permissions (DESIGN §10.2.4).
val forbiddenDefaultPermissions = listOf(
    "android.permission.ACCESS_LOCAL_NETWORK",
    "android.permission.MANAGE_OWN_CALLS",
    "android.permission.FOREGROUND_SERVICE_PHONE_CALL",
)

listOf("Debug", "Release").forEach { variant ->
    val check = tasks.register("verify${variant}MergedManifest") {
        group = "verification"
        description = "Checks the merged $variant manifest against DESIGN §10.2.4."
        val merged = layout.buildDirectory.file(
            "intermediates/merged_manifest/${variant.lowercase()}/process${variant}Manifest/AndroidManifest.xml",
        )
        inputs.file(merged)
        inputs.property("telecom", telecom)
        doLast {
            val xml = merged.get().asFile.readText()
            if (!telecom) {
                forbiddenDefaultPermissions.filter { xml.contains("\"$it\"") }.takeIf { it.isNotEmpty() }?.let {
                    throw GradleException("Merged $variant manifest must not declare $it")
                }
            }
            require(!xml.contains("\"android.permission.ACCESS_LOCAL_NETWORK\"")) {
                "ACCESS_LOCAL_NETWORK is added only when targeting 37"
            }
            require(xml.contains("com.convlink.voice.engine.MeshEngineService")) {
                "MeshEngineService missing from the merged $variant manifest"
            }
        }
    }
    tasks.matching { it.name == "process${variant}Manifest" }.configureEach {
        // The overlay is wired through the source sets; make the flag a cache input too.
        inputs.property("meshengine.telecom", telecom)
        finalizedBy(check)
    }
}
