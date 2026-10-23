package com.convlink.voice.engine

import android.content.pm.ServiceInfo
import com.convlink.voice.engine.FgsTypes.Grants
import com.convlink.voice.engine.FgsTypes.Stage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** DESIGN §10.2.3: the FGS type set is computed from the current grants every time. */
class FgsTypesTest {
    private val cd = ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
    private val mic = ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
    private val loc = ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION
    private val all = Grants(microphone = true, fineLocation = true)

    @Test
    fun lobbyIsConnectedDeviceOnly() {
        assertEquals(cd, FgsTypes.compute(Stage.LOBBY, all, shareLocation = true))
    }

    @Test
    fun goLiveAddsMicrophone() {
        assertEquals(cd or mic, FgsTypes.compute(Stage.LIVE, all, shareLocation = false))
    }

    @Test
    fun locationOnlyWhenGrantedAndShared() {
        assertEquals(cd or mic or loc, FgsTypes.compute(Stage.LIVE, all, shareLocation = true))
        val noLoc = Grants(microphone = true, fineLocation = false)
        val t = FgsTypes.compute(Stage.LIVE, noLoc, shareLocation = true)
        assertFalse("LOCATION without the permission throws on Android 14+", FgsTypes.has(t, loc))
        assertEquals(cd or mic, t)
    }

    @Test
    fun noLocationTypeWithoutPermissionInAnyStage() {
        val noLoc = Grants(microphone = true, fineLocation = false)
        for (stage in Stage.entries) for (share in listOf(true, false)) {
            assertFalse("$stage/$share", FgsTypes.has(FgsTypes.compute(stage, noLoc, share), loc))
        }
    }

    @Test
    fun noMicrophoneTypeWithoutRecordAudio() {
        val t = FgsTypes.compute(Stage.LIVE, Grants(microphone = false, fineLocation = true), shareLocation = true)
        assertFalse(FgsTypes.has(t, mic))
        assertTrue(FgsTypes.has(t, cd))
        assertTrue(FgsTypes.has(t, loc))
    }

    @Test
    fun labWithoutMicKeepsLocationAnchor() {
        assertEquals(cd or loc, FgsTypes.compute(Stage.LAB_NO_MIC, all, shareLocation = true))
        assertEquals(cd, FgsTypes.compute(Stage.LAB_NO_MIC, all, shareLocation = false))
    }
}
