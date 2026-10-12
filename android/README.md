# Tetherly Android (Phase 1)

NotificationListenerService + 前台服务。不读短信库。OTP 在桌面本地提取。

## 权限

- 通知使用权（系统设置 → 通知使用权 → Tetherly）
- Android 13+ `POST_NOTIFICATIONS`
- 各 ROM 自启动 / 电池优化：见应用内说明，不做黑保活

## 构建

先编原生库（真实 Ed25519 身份 + Noise 会话，`libtetherly_android.so`），再编 APK：

```text
cargo install cargo-ndk --locked        # 一次
export ANDROID_NDK_HOME=<ndk>
cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build -p tetherly-android --release

cd android
./gradlew :app:assembleDebug :app:testDebugUnitTest
```

`jniLibs/` 是 cargo-ndk 的产物，不入库（见 `.gitignore`）；CI 的 `android` job 会先跑上面的 `cargo ndk` 再跑 Gradle。

Windows 无 wrapper jar 时用本机 Gradle 8.9+。CI 使用 `gradle/actions/setup-gradle`。

真机自检（无需 APK，直接验证配对 + notify 通路）：

```text
cargo ndk -t arm64-v8a build -p tetherly-android --release --bin android_probe
adb push target/aarch64-linux-android/release/android_probe /data/local/tmp/tetherly/
adb shell /data/local/tmp/tetherly/android_probe <host:port> <pin> [data_dir] [title] [body]
```

物理机 M1.1 p95 / 8h soak 标 Manual-required；上述 probe 已在两台 Galaxy Fold（arm64-v8a, Android 13）上完成真实配对 + notify 投递 + 断线重连（Noise IK resume）验证。
