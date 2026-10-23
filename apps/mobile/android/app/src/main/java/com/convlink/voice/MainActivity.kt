package com.convlink.voice

import android.os.Bundle
import com.convlink.voice.plugin.MeshIntercomPlugin
import com.getcapacitor.BridgeActivity

/**
 * Single activity (WP-18, DESIGN §2.4, §10.2.4): `singleTask` so an invite link
 * (`convlinkvoice://join`) reaches the running intercom instead of a second copy.
 * The app-local MeshIntercom bridge is registered BEFORE `super.onCreate`, which is
 * when Capacitor builds the bridge and its plugin list.
 */
class MainActivity : BridgeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        registerPlugin(MeshIntercomPlugin::class.java)
        super.onCreate(savedInstanceState)
    }
}
