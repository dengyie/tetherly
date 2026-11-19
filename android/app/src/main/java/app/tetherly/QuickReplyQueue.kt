package app.tetherly

import java.util.LinkedHashMap

/**
 * Process-local delivery queue for desktop quick replies. The native inbox is
 * destructive, so this queue owns retries after a reply crosses the JNI edge.
 */
internal class QuickReplyQueue(
    private val retryWindowMs: Long = DEFAULT_RETRY_WINDOW_MS,
    private val maxAttempts: Int = DEFAULT_MAX_ATTEMPTS,
) {
    private data class Key(val uid: String, val appId: String)

    private data class Entry(
        val reply: PhoneReply,
        val expiresAtMs: Long,
        val generation: Long,
        var attempts: Int,
    )

    private val entries = LinkedHashMap<Key, Entry>()
    private var nextGeneration = 0L

    init {
        require(retryWindowMs > 0) { "retry window must be positive" }
        require(maxAttempts > 0) { "max attempts must be positive" }
    }

    fun enqueue(replies: Iterable<PhoneReply>, nowMs: Long) {
        for (reply in replies) {
            entries[Key(reply.uid, reply.appId)] = Entry(
                reply = reply,
                expiresAtMs = nowMs + retryWindowMs,
                generation = ++nextGeneration,
                attempts = 0,
            )
        }
    }

    /** Return each deliverable reply once and increment its attempt counter. */
    fun takeAttempts(nowMs: Long): List<PendingQuickReply> {
        purge(nowMs)
        return entries.values.map { entry ->
            entry.attempts += 1
            PendingQuickReply(entry.reply, entry.expiresAtMs, entry.generation, entry.attempts)
        }
    }

    /** Remove a delivered entry, or retain a failed entry while it is viable. */
    fun finish(attempt: PendingQuickReply, delivered: Boolean, nowMs: Long) {
        val key = Key(attempt.reply.uid, attempt.reply.appId)
        val entry = entries[key] ?: return
        if (entry.generation != attempt.generation || entry.attempts != attempt.attempts) return
        if (delivered || entry.attempts >= maxAttempts || nowMs >= entry.expiresAtMs) {
            entries.remove(key)
        }
    }

    fun hasPending(nowMs: Long): Boolean {
        purge(nowMs)
        return entries.isNotEmpty()
    }

    fun clear() {
        entries.clear()
    }

    private fun purge(nowMs: Long) {
        val iterator = entries.entries.iterator()
        while (iterator.hasNext()) {
            val entry = iterator.next().value
            if (nowMs >= entry.expiresAtMs || entry.attempts >= maxAttempts) {
                iterator.remove()
            }
        }
    }

    companion object {
        const val DEFAULT_RETRY_WINDOW_MS = 5 * 60_000L
        const val DEFAULT_MAX_ATTEMPTS = 8
    }
}

internal data class PendingQuickReply(
    val reply: PhoneReply,
    val expiresAtMs: Long,
    internal val generation: Long,
    val attempts: Int,
)
