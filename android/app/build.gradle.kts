plugins {
    id("com.android.application")
}

val minSdkVersion = 31
val abis = listOf("arm64-v8a", "x86_64")
val abiTargets = mapOf("arm64-v8a" to "aarch64-linux-android", "x86_64" to "x86_64-linux-android")

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
        // The release workflow passes these from the version input and run number.
        versionCode = project.findProperty("versionCode")?.toString()?.toInt() ?: 1
        versionName = project.findProperty("versionName")?.toString() ?: "0.1.0"
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

    androidResources {
        noCompress += "pack"
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

    @get:OutputDirectory
    abstract val assetsDir: DirectoryProperty
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
            assetsDir.set(layout.buildDirectory.dir("rustAssets/${variant.name}"))
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
            doLast {
                val profile = if (release) "release" else "debug"
                rootDir.resolve("../target/${abiTargets.getValue(abis.first())}/$profile/assets.pack")
                    .copyTo(assetsDir.get().file("assets.pack").asFile, overwrite = true)
            }
        }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargoTask, CargoNdkTask::jniLibsDir)
        variant.sources.assets?.addGeneratedSourceDirectory(cargoTask, CargoNdkTask::assetsDir)
    }
}
