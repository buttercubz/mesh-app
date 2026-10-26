package com.convlink.voice

import android.content.res.Configuration
import android.os.Bundle
import com.convlink.voice.plugin.MeshIntercomPlugin
import com.getcapacitor.BridgeActivity
import kotlin.math.roundToInt

/**
 * Single activity (WP-18, DESIGN §2.4, §10.2.4): `singleTask` so an invite link
 * (`convlinkvoice://join`) reaches the running intercom instead of a second copy.
 * The app-local MeshIntercom bridge is registered BEFORE `super.onCreate`, which is
 * when Capacitor builds the bridge and its plugin list.
 *
 * Text size: the WebView takes the system font scale as its text zoom (85-200 %, every
 * font included). The ride screen is read at a glance and laid out for 100-130 %, so the
 * zoom follows the system setting within [MIN_TEXT_ZOOM, MAX_TEXT_ZOOM]; riders who need
 * more keep the system Magnification. `fontScale` is in the manifest's configChanges, so a
 * change made while the app is in the background re-applies the zoom here instead of
 * recreating the activity (and reloading the WebView mid-ride).
 */
class MainActivity : BridgeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        registerPlugin(MeshIntercomPlugin::class.java)
        super.onCreate(savedInstanceState)
        applyTextZoom(resources.configuration)
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        applyTextZoom(newConfig)
    }

    private fun applyTextZoom(config: Configuration) {
        val zoom = (config.fontScale * 100).roundToInt().coerceIn(MIN_TEXT_ZOOM, MAX_TEXT_ZOOM)
        bridge?.webView?.settings?.textZoom = zoom
    }

    companion object {
        /** Never smaller than the design size (13 px banner text, glove targets). */
        const val MIN_TEXT_ZOOM = 100

        /** The largest size the ride's glance layout is verified at. */
        const val MAX_TEXT_ZOOM = 130
    }
}
