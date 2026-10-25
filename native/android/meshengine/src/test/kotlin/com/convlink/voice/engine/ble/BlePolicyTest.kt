package com.convlink.voice.engine.ble

import android.bluetooth.le.AdvertisingSetParameters
import android.bluetooth.le.ScanSettings
import com.convlink.voice.core.FfiAdvData
import com.convlink.voice.core.FfiDiscoveryMode
import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiPeerPlatform
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AdvBuilderTest {
    private val sd = byteArrayOf(0x01, 1, 2, 3, 4, 5, 6, 7, 8)

    @Test
    fun advertisingIsLegacyConnectableScannable() {
        val s = AdvBuilder.spec(BleConfig.DESIGN, FfiDiscoveryMode.FORMING, sd)
        assertTrue(s.legacy)
        assertTrue(s.connectable)
        assertTrue(s.scannable)
        assertFalse(s.includeDeviceName)
        assertEquals(AdvertisingSetParameters.TX_POWER_HIGH, s.txPower)
        assertEquals(BleConfig.DESIGN.service, s.serviceUuid)
    }

    @Test
    fun payloadsFitLegacyPdus() {
        val s = AdvBuilder.spec(BleConfig.DESIGN, FfiDiscoveryMode.STABLE, sd)
        assertEquals(21, s.primaryLength)
        assertEquals(27, s.scanResponseLength)
        assertArrayEquals(sd, s.serviceData)
    }

    @Test
    fun intervalFollowsDiscoveryModeAndLinks() {
        // ≈ 250 ms (400 × 0.625) only while forming with no link, ≈ 1 s (1600 × 0.625) otherwise.
        assertEquals(AdvertisingSetParameters.INTERVAL_MEDIUM, AdvBuilder.interval(FfiDiscoveryMode.FORMING, 0))
        assertEquals(400, AdvertisingSetParameters.INTERVAL_MEDIUM)
        assertEquals(AdvertisingSetParameters.INTERVAL_HIGH, AdvBuilder.interval(FfiDiscoveryMode.FORMING, 1))
        assertEquals(AdvertisingSetParameters.INTERVAL_HIGH, AdvBuilder.interval(FfiDiscoveryMode.STABLE, 0))
        assertEquals(1600, AdvertisingSetParameters.INTERVAL_HIGH)
        assertEquals(1600, AdvBuilder.spec(BleConfig.DESIGN, DiscoveryDuty.ADV_SLOW, sd).interval)
    }

    @Test(expected = IllegalArgumentException::class)
    fun wrongServiceDataLengthIsRefused() {
        AdvBuilder.spec(BleConfig.DESIGN, FfiDiscoveryMode.FORMING, ByteArray(12))
    }
}

class LinkDownMapperTest {
    @Test
    fun connectionLimitStatusCodesMapToLimitReached() {
        assertEquals(FfiLinkDownReason.LimitReached, LinkDownMapper.connectFailure(133))
        assertEquals(FfiLinkDownReason.LimitReached, LinkDownMapper.connectFailure(257))
    }

    @Test
    fun otherConnectFailures() {
        assertEquals(FfiLinkDownReason.Timeout, LinkDownMapper.connectFailure(8))
        assertEquals(FfiLinkDownReason.Timeout, LinkDownMapper.connectFailure(62))
        assertEquals(FfiLinkDownReason.PeerClosed, LinkDownMapper.connectFailure(19))
        assertEquals(FfiLinkDownReason.Error(147), LinkDownMapper.connectFailure(147))
    }

    @Test
    fun establishedLinkDisconnects() {
        assertEquals(FfiLinkDownReason.LocalClose, LinkDownMapper.disconnect(133, localClose = true))
        assertEquals(FfiLinkDownReason.PeerClosed, LinkDownMapper.disconnect(19, false))
        assertEquals(FfiLinkDownReason.Timeout, LinkDownMapper.disconnect(8, false))
        // After the link is up, 133 is an error, not a connection-limit signal.
        assertEquals(FfiLinkDownReason.Error(133), LinkDownMapper.disconnect(133, false))
    }
}

class ScanPolicyTest {
    @Test
    fun neverMoreThanFiveStartsPer30s() {
        val p = ScanPolicy()
        repeat(5) { assertTrue(p.recordStart(it * 1_000L)) }
        assertFalse(p.canStart(5_000))
        assertFalse(p.recordStart(5_000))
        assertEquals(25_000L, p.delayUntilAllowed(5_000))
        assertTrue(p.canStart(30_000))
        assertTrue(p.recordStart(30_000))
        assertFalse(p.recordStart(30_500))
    }

    @Test
    fun filteredScansRestartEveryNineMinutes() {
        val p = ScanPolicy()
        assertFalse(p.restartDue(0, 9 * 60_000L - 1))
        assertTrue(p.restartDue(0, 9 * 60_000L))
        assertEquals(540_000L, p.restartIntervalMs)
    }
}

class DiscoveryParserTest {
    @Test
    fun onlyAndroidServiceDataIsAccepted() {
        val android = byteArrayOf(0x03, 1, 2, 3, 4, 9, 8, 7, 6)
        assertArrayEquals(android, DiscoveryParser.androidServiceData(android))
        assertNull(DiscoveryParser.androidServiceData(byteArrayOf(0x02, 1, 2, 3, 4, 9, 8, 7, 6)))
        assertNull(DiscoveryParser.androidServiceData(byteArrayOf(0x01, 2)))
        assertNull(DiscoveryParser.androidServiceData(null))
    }

    @Test
    fun discoveryCarriesServiceDataAndHint() {
        val d = DiscoveryParser.discovery(7, byteArrayOf(1, 0, 0, 0, 0, 0, 0, 0, 0), -60)
        assertEquals(7u, d.discId)
        assertEquals(FfiPeerPlatform.ANDROID, d.platformHint)
        assertEquals((-60).toShort(), d.rssi)
        assertTrue(d.adv is FfiAdvData.ServiceData)
        assertNull(DiscoveryParser.rssi(127))
    }

    @Test
    fun discoveryIdsAreStablePerAddress() {
        val t = DiscoveryTable()
        val a = t.idFor("AA:BB")
        val b = t.idFor("CC:DD")
        assertEquals(a, t.idFor("AA:BB"))
        assertTrue(a != b && a != 0 && b != 0)
        assertEquals("CC:DD", t.address(b))
        t.reset()
        assertNull(t.address(b))
    }
}

class DiscoveryDutyTest {
    @Test
    fun lowLatencyOnlyWithoutLinks() {
        assertEquals(ScanIntensity.HIGH, DiscoveryDuty.intensity(FfiDiscoveryMode.FORMING, 0))
        assertEquals(ScanIntensity.MEDIUM, DiscoveryDuty.intensity(FfiDiscoveryMode.FORMING, 1))
        assertEquals(ScanIntensity.MEDIUM, DiscoveryDuty.intensity(FfiDiscoveryMode.FORMING, 3))
        assertEquals(ScanIntensity.LOW, DiscoveryDuty.intensity(FfiDiscoveryMode.STABLE, 0))
        assertEquals(ScanIntensity.LOW, DiscoveryDuty.intensity(FfiDiscoveryMode.STABLE, 2))
        assertNull(DiscoveryDuty.intensity(FfiDiscoveryMode.OFF, 0))
    }

    @Test
    fun scanModesPerIntensity() {
        val high = DiscoveryDuty.duty(ScanIntensity.HIGH, offloadedFiltering = true)
        assertEquals(ScanSettings.SCAN_MODE_LOW_LATENCY, high.scanMode)
        assertEquals(ScanSettings.CALLBACK_TYPE_ALL_MATCHES, high.callbackType)
        assertEquals(DiscoveryDuty.ADV_FAST, high.advInterval)
        val medium = DiscoveryDuty.duty(ScanIntensity.MEDIUM, offloadedFiltering = true)
        assertEquals(ScanSettings.SCAN_MODE_BALANCED, medium.scanMode)
        assertEquals(DiscoveryDuty.ADV_SLOW, medium.advInterval)
        val low = DiscoveryDuty.duty(ScanIntensity.LOW, offloadedFiltering = true)
        assertEquals(ScanSettings.SCAN_MODE_LOW_POWER, low.scanMode)
        assertEquals(ScanSettings.CALLBACK_TYPE_FIRST_MATCH, low.callbackType)
        assertEquals(DiscoveryDuty.ADV_SLOW, low.advInterval)
        // Without offloaded filtering FIRST_MATCH is not available.
        assertEquals(ScanSettings.CALLBACK_TYPE_ALL_MATCHES,
            DiscoveryDuty.duty(ScanIntensity.LOW, offloadedFiltering = false).callbackType)
        for (i in ScanIntensity.entries) {
            assertEquals(ScanSettings.MATCH_MODE_STICKY, DiscoveryDuty.duty(i, true).matchMode)
        }
    }

    @Test
    fun coreIntensityIsHonouredButNeverHighWithLinks() {
        assertEquals(ScanIntensity.LOW, DiscoveryDuty.intensity(FfiDiscoveryMode.FORMING, 0, ScanIntensity.LOW))
        assertEquals(ScanIntensity.HIGH, DiscoveryDuty.intensity(FfiDiscoveryMode.STABLE, 0, ScanIntensity.HIGH))
        assertEquals(ScanIntensity.MEDIUM, DiscoveryDuty.intensity(FfiDiscoveryMode.FORMING, 2, ScanIntensity.HIGH))
    }

    class WithIntensity(val intensity: String)

    class Without(val mode: Int)

    enum class FakeScan { LOW_LATENCY, BALANCED, LOW_POWER }

    class WithScan(val scan: FakeScan)

    @Test
    fun optionalBindingFieldIsReadByName() {
        assertEquals(ScanIntensity.HIGH, DiscoveryDuty.coreIntensity(WithScan(FakeScan.LOW_LATENCY)))
        assertEquals(ScanIntensity.MEDIUM, DiscoveryDuty.coreIntensity(WithScan(FakeScan.BALANCED)))
        assertEquals(ScanIntensity.LOW, DiscoveryDuty.coreIntensity(WithScan(FakeScan.LOW_POWER)))
        assertEquals(ScanIntensity.MEDIUM, DiscoveryDuty.coreIntensity(WithIntensity("Hunting")))
        assertEquals(ScanIntensity.LOW, DiscoveryDuty.coreIntensity(WithIntensity("LOW")))
        assertNull(DiscoveryDuty.coreIntensity(WithIntensity("weird")))
        assertNull(DiscoveryDuty.coreIntensity(Without(1)))
    }
}

class PhyPolicyTest {
    @Test
    fun prefersOnly1mForRange() {
        // Regression (field test 2026-10-24): 2M must never be in the preference mask.
        assertEquals(android.bluetooth.BluetoothDevice.PHY_LE_1M_MASK, PhyPolicy.PREFERRED_MASK)
        assertEquals(0, PhyPolicy.PREFERRED_MASK and android.bluetooth.BluetoothDevice.PHY_LE_2M_MASK)
        assertEquals("1M", PhyPolicy.label(android.bluetooth.BluetoothDevice.PHY_LE_1M))
        assertEquals("2M", PhyPolicy.label(android.bluetooth.BluetoothDevice.PHY_LE_2M))
        assertEquals("Coded", PhyPolicy.label(android.bluetooth.BluetoothDevice.PHY_LE_CODED))
    }
}
