plugins {
    id("com.android.application")
}

val minSdkVersion = 31
// Rust targets are built for each ABI listed here.
val abis = listOf("arm64-v8a", "x86_64")

android {
    namespace = "cudzinec.rts"
    compileSdk {
        version = release(37) {
            minorApiLevel = 2
        }
    }
    buildToolsVersion = "37.0.0"
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "cudzinec.rts"
        minSdk = minSdkVersion
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            abiFilters += abis
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation("androidx.appcompat:appcompat:1.8.0")
    implementation("androidx.core:core-ktx:1.19.1")
    // Must stay on the 4.4.x line supported by the `android-activity` crate used by Bevy.
    implementation("androidx.games:games-activity:4.4.2")
}

abstract class CargoNdkTask : Exec() {
    @get:OutputDirectory
    abstract val jniLibsDir: DirectoryProperty
}

val ndkDirectory = androidComponents.sdkComponents.ndkDirectory

androidComponents {
    onVariants { variant ->
        val release = variant.buildType == "release"
        val cargoTask = tasks.register<CargoNdkTask>("cargoBuild${variant.name.replaceFirstChar(Char::uppercase)}") {
            group = "build"
            description = "Builds the Rust library for ${variant.name}."
            workingDir = rootDir
            jniLibsDir.set(layout.buildDirectory.dir("rustJniLibs/${variant.name}"))
            // Cargo tracks its own inputs, so always let it decide what to rebuild.
            outputs.upToDateWhen { false }
            doFirst {
                environment("ANDROID_NDK_HOME", ndkDirectory.get().asFile.absolutePath)
                commandLine(
                    listOf("cargo", "ndk") +
                        abis.flatMap { listOf("-t", it) } +
                        listOf("-P", "$minSdkVersion", "-o", jniLibsDir.get().asFile.absolutePath) +
                        listOf("build", "-p", "rts_android") +
                        (if (release) listOf("--release") else emptyList())
                )
            }
        }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargoTask, CargoNdkTask::jniLibsDir)
    }
}
