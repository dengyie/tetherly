# Tetherly Android (Phase 1)

NotificationListenerService + 前台服务。不读短信库。OTP 在桌面本地提取。

## 权限

- 通知使用权（系统设置 → 通知使用权 → Tetherly）
- Android 13+ `POST_NOTIFICATIONS`
- 各 ROM 自启动 / 电池优化：见应用内说明，不做黑保活

## 构建

```text
cd android
./gradlew :app:assembleDebug :app:testDebugUnitTest
```

Windows 无 wrapper jar 时用本机 Gradle 8.9+。CI 使用 `gradle/actions/setup-gradle`。

物理机 M1.1 p95 / 8h soak 标 Manual-required。
