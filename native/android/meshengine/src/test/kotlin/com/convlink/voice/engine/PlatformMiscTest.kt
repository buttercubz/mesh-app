package com.convlink.voice.engine

import android.bluetooth.BluetoothAdapter
import android.os.BatteryManager
import android.os.PowerManager
import com.convlink.voice.core.FfiRadioAvail
import com.convlink.voice.core.FfiThermal
import com.convlink.voice.engine.notify.IntercomAction
import com.convlink.voice.engine.notify.IntercomState
import com.convlink.voice.engine.notify.NotificationCatalog
import com.convlink.voice.engine.notify.NotificationModels
import com.convlink.voice.engine.qr.InviteScanner
import com.convlink.voice.engine.radio.RadioStates
import com.convlink.voice.engine.sensors.LocationSampler
import com.convlink.voice.engine.sensors.Samples
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Locale

class NotificationModelTest {
    private val en = NotificationCatalog.fallback(Locale.ENGLISH)
    private val es = NotificationCatalog.fallback(Locale.forLanguageTag("es"))

    @Test
    fun liveShowsMuteAndTalkLatch() {
        val m = NotificationModels.ongoing(IntercomState(IntercomState.Stage.LIVE), en)
        assertEquals("Intercom live", m.text)
        assertEquals(listOf(IntercomAction.MUTE, IntercomAction.TALK_LATCH, IntercomAction.LEAVE), m.actions.map { it.first })
        val muted = NotificationModels.ongoing(IntercomState(IntercomState.Stage.LIVE, muted = true), es)
        assertEquals(IntercomAction.UNMUTE, muted.actions.first().first)
        assertEquals("Activar mic", muted.actions.first().second)
    }

    @Test
    fun captureLostOffersResume() {
        val m = NotificationModels.ongoing(IntercomState(IntercomState.Stage.LIVE, captureLost = true), es)
        assertEquals("Micrófono en pausa: toca para reanudar", m.text)
        assertEquals(listOf(IntercomAction.RESUME, IntercomAction.LEAVE), m.actions.map { it.first })
    }

    @Test
    fun lobbyOnlyLeaves() {
        val m = NotificationModels.ongoing(IntercomState(), en)
        assertEquals(listOf(IntercomAction.LEAVE), m.actions.map { it.first })
    }

    @Test
    fun catalogTemplatesFromTheUi() {
        val c = NotificationCatalog("convLink Voice", mapOf("notices.memberLost" to "{{count}} lost: {{names}}"))
        assertEquals("2 lost: Beto, Carla", c.text("notices.memberLost", JSONObject("""{"count":2,"names":"Beto, Carla"}""")))
        assertNull(c.text("notices.unknown"))
        assertEquals("Callar", NotificationCatalog("x", mapOf("notify.mute" to "Callar")).label(IntercomAction.MUTE))
    }

    @Test
    fun everyActionRoundTripsThroughItsIntentAction() {
        IntercomAction.entries.forEach { assertEquals(it, IntercomAction.fromIntent(it.intentAction)) }
        assertNull(IntercomAction.fromIntent("other"))
        // ES and EN fallback catalogs carry the same keys (i18n parity).
        assertEquals(en.templates.keys, es.templates.keys)
    }
}

class RadioAndSamplesTest {
    @Test
    fun bluetoothAvailability() {
        assertEquals(FfiRadioAvail.UNSUPPORTED, RadioStates.ble(false, BluetoothAdapter.STATE_ON, true))
        assertEquals(FfiRadioAvail.UNAUTHORIZED, RadioStates.ble(true, BluetoothAdapter.STATE_ON, false))
        assertEquals(FfiRadioAvail.ON, RadioStates.ble(true, BluetoothAdapter.STATE_ON, true))
        assertEquals(FfiRadioAvail.OFF, RadioStates.ble(true, BluetoothAdapter.STATE_TURNING_OFF, true))
        assertEquals(FfiRadioAvail.OFF, RadioStates.ble(true, BluetoothAdapter.STATE_OFF, true))
        assertEquals(FfiRadioAvail.OFF, RadioStates.aware(true, true, airplane = true))
        assertEquals(FfiRadioAvail.UNSUPPORTED, RadioStates.aware(false, true, false))
        assertEquals(FfiRadioAvail.ON, RadioStates.aware(true, true, false))
    }

    @Test
    fun deviceSamples() {
        assertEquals(0.5, Samples.batteryFraction(50, 100)!!, 1e-9)
        assertNull(Samples.batteryFraction(-1, 100))
        assertTrue(Samples.charging(BatteryManager.BATTERY_STATUS_CHARGING))
        assertEquals(FfiThermal.NOMINAL, Samples.thermal(PowerManager.THERMAL_STATUS_LIGHT))
        assertEquals(FfiThermal.FAIR, Samples.thermal(PowerManager.THERMAL_STATUS_MODERATE))
        assertEquals(FfiThermal.SERIOUS, Samples.thermal(PowerManager.THERMAL_STATUS_SEVERE))
        assertEquals(FfiThermal.CRITICAL, Samples.thermal(PowerManager.THERMAL_STATUS_SHUTDOWN))
        val fix = LocationSampler.toFix(40.4, -3.7, 12f, null, 90f)
        assertEquals(12.0, fix.speedMps!!, 1e-6)
        assertNull(fix.accM)
    }

    @Test
    fun inviteScannerAcceptsOnlyInvites() {
        assertEquals(InviteScanner.Result.Invite("convlinkvoice://join?v=1#AAEC"),
            InviteScanner.classify(" convlinkvoice://join?v=1#AAEC "))
        assertEquals(InviteScanner.Result.NotInvite, InviteScanner.classify("https://example.com"))
        assertEquals(InviteScanner.Result.NotInvite, InviteScanner.classify(null))
    }
}
