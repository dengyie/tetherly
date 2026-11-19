package app.tetherly

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class QuickReplyQueueTest {
    @Test
    fun failedDeliveryIsRetriedUntilTheAttemptBound() {
        val queue = QuickReplyQueue(retryWindowMs = 1_000, maxAttempts = 2)
        queue.enqueue(listOf(reply("first")), nowMs = 0)

        val first = queue.takeAttempts(nowMs = 0).single()
        assertEquals(1, first.attempts)
        queue.finish(first, delivered = false, nowMs = 0)
        assertTrue(queue.hasPending(nowMs = 1))

        val second = queue.takeAttempts(nowMs = 1).single()
        assertEquals(2, second.attempts)
        queue.finish(second, delivered = false, nowMs = 1)
        assertFalse(queue.hasPending(nowMs = 1))
    }

    @Test
    fun ttlExpiresAnUnmatchedNotification() {
        val queue = QuickReplyQueue(retryWindowMs = 100, maxAttempts = 8)
        queue.enqueue(listOf(reply("reply")), nowMs = 0)

        val attempt = queue.takeAttempts(nowMs = 0).single()
        queue.finish(attempt, delivered = false, nowMs = 99)
        assertTrue(queue.hasPending(nowMs = 99))
        assertFalse(queue.hasPending(nowMs = 100))
    }

    @Test
    fun deliveryRemovesTheReplyImmediately() {
        val queue = QuickReplyQueue(retryWindowMs = 100, maxAttempts = 8)
        queue.enqueue(listOf(reply("reply")), nowMs = 0)

        val attempt = queue.takeAttempts(nowMs = 0).single()
        queue.finish(attempt, delivered = true, nowMs = 0)
        assertFalse(queue.hasPending(nowMs = 0))
    }

    @Test
    fun newerReplyForTheSameNotificationReplacesTheOldOne() {
        val queue = QuickReplyQueue(retryWindowMs = 100, maxAttempts = 8)
        queue.enqueue(listOf(reply("old")), nowMs = 0)
        queue.enqueue(listOf(reply("new")), nowMs = 1)

        val attempt = queue.takeAttempts(nowMs = 1).single()
        assertEquals("new", attempt.reply.text)
    }

    @Test
    fun staleAttemptCannotDeleteAReplacedReply() {
        val queue = QuickReplyQueue(retryWindowMs = 100, maxAttempts = 8)
        queue.enqueue(listOf(reply("old")), nowMs = 0)
        val stale = queue.takeAttempts(nowMs = 0).single()
        queue.enqueue(listOf(reply("new")), nowMs = 1)

        queue.finish(stale, delivered = true, nowMs = 1)
        val current = queue.takeAttempts(nowMs = 1).single()
        assertEquals("new", current.reply.text)
        assertEquals(1, current.attempts)
    }

    private fun reply(text: String) = PhoneReply(
        uid = "notification-key",
        appId = "com.example.chat",
        text = text,
    )
}
