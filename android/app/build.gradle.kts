plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

val workspaceVersion = rootProject.file("../Cargo.toml").readLines()
    .dropWhile { it.trim() != "[workspace.package]" }
    .drop(1)
    .first { it.trim().startsWith("version =") }
    .substringAfter('"').substringBefore('"')
val workspaceParts = workspaceVersion.split(".").map(String::toInt)
require(workspaceParts.size == 3 && workspaceParts[0] <= 2100 && workspaceParts[1] < 1000 && workspaceParts[2] < 1000) {
    "workspace version must be numeric major.minor.patch below Android versionCode limits"
}
val defaultVersionCode = workspaceParts[0] * 1_000_000 + workspaceParts[1] * 1_000 + workspaceParts[2]
val tetherlyVersion = providers.gradleProperty("tetherlyVersion").orElse(workspaceVersion).get()
val tetherlyVersionCode = providers.gradleProperty("tetherlyVersionCode").map(String::toInt).orElse(defaultVersionCode).get()
require(tetherlyVersion == workspaceVersion) { "Android version must match Cargo workspace version" }
val releaseKeystore = providers.gradleProperty("releaseKeystore").orNull
val releaseStorePassword = providers.gradleProperty("releaseStorePassword").orNull
val releaseKeyAlias = providers.gradleProperty("releaseKeyAlias").orNull
val releaseKeyPassword = providers.gradleProperty("releaseKeyPassword").orNull

android {
    namespace = "app.tetherly"
    compileSdk = 35
    defaultConfig {
        applicationId = "app.tetherly"
        minSdk = 26
        targetSdk = 35
        versionCode = tetherlyVersionCode
        versionName = tetherlyVersion
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }
    signingConfigs {
        create("release") {
            // Release CI supplies a dedicated keystore. Do not silently fall
            // back to the debug key: the release workflow inspects this
            // signature before publishing the APK.
            if (releaseKeystore != null && releaseStorePassword != null && releaseKeyAlias != null && releaseKeyPassword != null) {
                storeFile = file(releaseKeystore)
                storePassword = releaseStorePassword
                keyAlias = releaseKeyAlias
                keyPassword = releaseKeyPassword
            } else if (gradle.startParameter.taskNames.any { it.endsWith("assembleRelease") }) {
                error("releaseKeystore, releaseStorePassword, releaseKeyAlias, and releaseKeyPassword are required")
            }
        }
    }
    buildTypes {
        release {
            signingConfig = signingConfigs.getByName("release")
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { viewBinding = false; compose = true }
    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("com.google.android.material:material:1.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")
    // Jetpack Compose (§171 Android redesign): BOM pins every artifact to a
    // coherent set; material3 matches the manifest's Theme.Material3.
    val composeBom = platform("androidx.compose:compose-bom:2024.10.01")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.compose.ui:ui-tooling-preview")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
}
