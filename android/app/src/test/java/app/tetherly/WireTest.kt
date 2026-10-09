package app.tetherly

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class WireTest {
    @Test
    fun notifyPushHasNoUrlAction() {
        val j = Wire.notifyPush("42", "com.example.sms", "信息", "网易", "【网易】验证码：868740", 1)
        assertEquals("42", j.getString("uid"))
        val actions = j.getJSONArray("actions")
        for (i in 0 until actions.length()) {
            assertFalse(actions.getString(i).contains("://"))
            assertFalse(actions.getString(i) == "url")
        }
    }

    @Test
    fun deviceIdPrefix() {
        val id = Wire.deviceIdFromIdPk(ByteArray(32) { 7 })
        assertTrue(id.startsWith("tdev_"))
        assertEquals(5 + 16, id.length)
    }

    @Test
    fun titleTruncated() {
        val big = "x".repeat(5000)
        val j = Wire.notifyPush("1", "a", "a", big, big, 0)
        assertEquals(4096, j.getString("title").length)
        assertEquals(4096, j.getString("body").length)
    }
}
