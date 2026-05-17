package com.menketechnologies.zshrs.dap

import com.google.gson.JsonObject
import com.google.gson.JsonParser
import com.intellij.openapi.diagnostic.Logger
import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.io.OutputStream
import java.nio.charset.StandardCharsets
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

/**
 * DAP client speaking Content-Length-framed JSON-RPC. Byte-based framing
 * (Content-Length is BYTES, not chars) so multi-byte UTF-8 in variable reprs
 * doesn't desync.
 */
class ZshrsDapClient(
    output: OutputStream,
    input: InputStream,
    private val onEvent: (event: String, body: JsonObject) -> Unit,
    private val onLog: (line: String) -> Unit = {},
) {
    private val output = output
    private val input = BufferedInputStream(input)
    private val seq = AtomicInteger(1)
    private val pending = ConcurrentHashMap<Int, AtomicReference<JsonObject?>>()
    private val pendingLatch = ConcurrentHashMap<Int, CountDownLatch>()
    private val readerThread: Thread

    @Volatile private var alive = true

    init {
        readerThread = Thread({
            try { runReader() } catch (e: Exception) { LOG.warn("DAP reader died", e) }
            alive = false
            pendingLatch.values.forEach { it.countDown() }
        }, "zshrs-DAP-Reader").apply {
            isDaemon = true
            start()
        }
    }

    fun isAlive(): Boolean = alive

    fun request(command: String, arguments: JsonObject = JsonObject(), timeoutMs: Long = 10_000): JsonObject? {
        val s = seq.getAndIncrement()
        val msg = JsonObject().apply {
            addProperty("seq", s)
            addProperty("type", "request")
            addProperty("command", command)
            add("arguments", arguments)
        }
        val latch = CountDownLatch(1)
        val slot = AtomicReference<JsonObject?>()
        pendingLatch[s] = latch
        pending[s] = slot
        send(msg)
        latch.await(timeoutMs, java.util.concurrent.TimeUnit.MILLISECONDS)
        pending.remove(s)
        pendingLatch.remove(s)
        return slot.get()
    }

    fun requestAsync(command: String, arguments: JsonObject = JsonObject()) {
        val s = seq.getAndIncrement()
        val msg = JsonObject().apply {
            addProperty("seq", s)
            addProperty("type", "request")
            addProperty("command", command)
            add("arguments", arguments)
        }
        send(msg)
    }

    @Synchronized
    private fun send(msg: JsonObject) {
        if (!alive) return
        val body = msg.toString().toByteArray(StandardCharsets.UTF_8)
        val header = "Content-Length: ${body.size}\r\n\r\n".toByteArray(StandardCharsets.US_ASCII)
        try {
            output.write(header)
            output.write(body)
            output.flush()
            val cmd = msg.get("command")?.asString
            val seqStr = msg.get("seq")?.asString ?: msg.get("seq")?.asInt?.toString()
            com.menketechnologies.zshrs.ZshrsDebugLog.log(
                "dap",
                "→ seq=$seqStr type=${msg.get("type")?.asString} command=$cmd bytes=${body.size}",
            )
        } catch (e: Exception) {
            LOG.warn("DAP send failed", e)
            com.menketechnologies.zshrs.ZshrsDebugLog.log("dap", "send failed: ${e.message}")
            alive = false
        }
    }

    private fun runReader() {
        while (alive) {
            var contentLength = -1
            val headerBytes = ByteArrayOutputStream()
            var sawCRLFCRLF = false
            while (!sawCRLFCRLF) {
                val b = input.read()
                if (b < 0) { alive = false; return }
                headerBytes.write(b)
                val arr = headerBytes.toByteArray()
                val sz = arr.size
                if (sz >= 4 && arr[sz - 4] == 0x0d.toByte() && arr[sz - 3] == 0x0a.toByte()
                    && arr[sz - 2] == 0x0d.toByte() && arr[sz - 1] == 0x0a.toByte()) {
                    sawCRLFCRLF = true
                }
            }
            val headerText = String(headerBytes.toByteArray(), StandardCharsets.US_ASCII)
            for (line in headerText.split("\r\n")) {
                val idx = line.indexOf(':')
                if (idx > 0) {
                    val k = line.substring(0, idx).trim()
                    val v = line.substring(idx + 1).trim()
                    if (k.equals("Content-Length", ignoreCase = true)) {
                        contentLength = v.toIntOrNull() ?: -1
                    }
                }
            }
            if (contentLength <= 0) continue

            val bodyBytes = ByteArray(contentLength)
            var off = 0
            while (off < contentLength) {
                val n = input.read(bodyBytes, off, contentLength - off)
                if (n < 0) { alive = false; return }
                off += n
            }
            val body = String(bodyBytes, StandardCharsets.UTF_8)
            onLog("← $body")
            val obj = try { JsonParser.parseString(body).asJsonObject } catch (_: Exception) { continue }
            when (obj.get("type")?.asString) {
                "response" -> {
                    val reqSeq = obj.get("request_seq")?.asInt ?: continue
                    val cmd = obj.get("command")?.asString
                    val success = obj.get("success")?.asBoolean
                    com.menketechnologies.zshrs.ZshrsDebugLog.log(
                        "dap",
                        "← response req_seq=$reqSeq command=$cmd success=$success bytes=$contentLength",
                    )
                    pending[reqSeq]?.set(obj.getAsJsonObject("body") ?: JsonObject())
                    pendingLatch[reqSeq]?.countDown()
                }
                "event" -> {
                    val event = obj.get("event")?.asString ?: continue
                    val eventBody = obj.getAsJsonObject("body") ?: JsonObject()
                    com.menketechnologies.zshrs.ZshrsDebugLog.log(
                        "dap",
                        "← event=$event bytes=$contentLength",
                    )
                    try { onEvent(event, eventBody) } catch (e: Exception) {
                        LOG.warn("event handler", e)
                        com.menketechnologies.zshrs.ZshrsDebugLog.log(
                            "dap",
                            "event handler threw: ${e.message}",
                        )
                    }
                }
            }
        }
    }

    fun close() {
        alive = false
        try { output.close() } catch (_: Exception) {}
    }

    companion object {
        private val LOG = Logger.getInstance(ZshrsDapClient::class.java)
    }
}
