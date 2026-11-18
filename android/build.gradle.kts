plugins {
    id("com.android.application") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
    // Compose compiler is shipped as a Kotlin plugin since Kotlin 2.0 — same
    // version as kotlin.android, no separate compiler artifact needed.
    id("org.jetbrains.kotlin.plugin.compose") version "2.0.21" apply false
}
