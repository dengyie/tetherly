package app.tetherly

/**
 * Decision table for "which event may end the native session". The Kotlin
 * callbacks call this (or follow it by convention) so the ownership rule has a
 * single auditable home instead of being implied by which call sites happen to
 * invoke session.stop().
 */
internal object SessionOwnershipPolicy {
    enum class Event {
        /** NMS rebinds the notification listener; can be transient on some ROMs. */
        LISTENER_DISCONNECTED,

        /** The listener service object is destroyed. */
        LISTENER_DESTROYED,

        /** The foreground service is destroyed without a pending restart. */
        SERVICE_DESTROYED,

        /** User/app-initiated stop via ForegroundKeep.stop. */
        EXPLICIT_STOP,

        /** Android 15 dataSync timeout; RestartCoordinator immediately re-dials. */
        FGS_TIMEOUT,
    }

    fun teardownOnEvent(event: Event): Boolean = when (event) {
        Event.EXPLICIT_STOP, Event.FGS_TIMEOUT -> true
        Event.LISTENER_DISCONNECTED, Event.LISTENER_DESTROYED, Event.SERVICE_DESTROYED -> false
    }
}
