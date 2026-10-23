package com.convlink.voice.engine

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.w3c.dom.Element
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory

/**
 * Library manifest contract (DESIGN §10.2.4). The merged-manifest check of the default
 * variant runs in Gradle (verify<Variant>MergedManifest, after process<Variant>Manifest);
 * this test pins the source manifests.
 */
class ManifestTest {
    private val android = "http://schemas.android.com/apk/res/android"
    private val module = Repo.file("native/android/meshengine")

    private fun permissions(f: File): Map<String, Element> {
        val doc = DocumentBuilderFactory.newInstance().apply { isNamespaceAware = true }.newDocumentBuilder().parse(f)
        val nodes = doc.getElementsByTagName("uses-permission")
        return (0 until nodes.length).map { nodes.item(it) as Element }.associateBy { it.getAttributeNS(android, "name") }
    }

    @Test
    fun defaultManifestDeclaresExactlyTheDesignPermissions() {
        val p = permissions(File(module, "src/main/AndroidManifest.xml"))
        val forbidden = listOf("ACCESS_LOCAL_NETWORK", "MANAGE_OWN_CALLS", "FOREGROUND_SERVICE_PHONE_CALL")
        forbidden.forEach { assertFalse(it, p.containsKey("android.permission.$it")) }
        val required = listOf(
            "RECORD_AUDIO", "MODIFY_AUDIO_SETTINGS", "FOREGROUND_SERVICE", "FOREGROUND_SERVICE_MICROPHONE",
            "FOREGROUND_SERVICE_CONNECTED_DEVICE", "FOREGROUND_SERVICE_LOCATION", "POST_NOTIFICATIONS",
            "BLUETOOTH_SCAN", "BLUETOOTH_ADVERTISE", "BLUETOOTH_CONNECT", "NEARBY_WIFI_DEVICES",
            "ACCESS_WIFI_STATE", "CHANGE_WIFI_STATE", "ACCESS_NETWORK_STATE", "CHANGE_NETWORK_STATE",
            "ACCESS_FINE_LOCATION", "ACCESS_COARSE_LOCATION", "WAKE_LOCK", "REQUEST_IGNORE_BATTERY_OPTIMIZATIONS",
        )
        assertEquals(required.map { "android.permission.$it" }.toSet(), p.keys)
        assertEquals("neverForLocation", p.getValue("android.permission.BLUETOOTH_SCAN").getAttributeNS(android, "usesPermissionFlags"))
        assertEquals("neverForLocation", p.getValue("android.permission.NEARBY_WIFI_DEVICES").getAttributeNS(android, "usesPermissionFlags"))
        val text = File(module, "src/main/AndroidManifest.xml").readText()
        assertTrue(text.contains("android:foregroundServiceType=\"microphone|connectedDevice|location\""))
        assertTrue(text.contains("android:exported=\"false\""))
    }

    @Test
    fun telecomOverlayAddsOnlyTheTelecomPermissions() {
        val p = permissions(File(module, "src/telecom/AndroidManifest.xml"))
        assertEquals(setOf("android.permission.MANAGE_OWN_CALLS", "android.permission.FOREGROUND_SERVICE_PHONE_CALL"), p.keys)
        assertTrue(File(module, "src/telecom/AndroidManifest.xml").readText().contains("phoneCall"))
    }

    @Test
    fun telecomBuildFlagMatchesTheGradleProperty() {
        assertEquals(System.getProperty("meshengine.telecom") == "true", BuildConfig.TELECOM)
    }
}
