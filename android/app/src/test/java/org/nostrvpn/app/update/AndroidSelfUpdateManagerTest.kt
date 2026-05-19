package org.nostrvpn.app.update

import java.net.URL
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidSelfUpdateManagerTest {
    @Test
    fun versionIsNewerHandlesCommonShapes() {
        assertTrue(versionIsNewer("v4.0.38", "4.0.37"))
        assertTrue(versionIsNewer("4.0.38", "v4.0.37"))
        assertTrue(versionIsNewer("v5.0.0", "4.99.99"))
        assertFalse(versionIsNewer("v4.0.37", "4.0.37"))
        assertFalse(versionIsNewer("v4.0.36", "4.0.37"))
    }

    @Test
    fun versionIsNewerTreatsZeroDotVersionsAsDevPlaceholders() {
        // Default/unconfigured builds report 0.0.0 — never claim an update
        // is available in that state.
        assertFalse(versionIsNewer("v4.0.38", "0.0.0"))
        assertFalse(versionIsNewer("v4.0.38", ""))
    }

    @Test
    fun resolveAssetUrlJoinsRelativePaths() {
        val manifest =
            "https://upload.iris.to/npub.../releases%2Fnostr-vpn/latest/release.json"
        val joined = resolveAssetUrl(manifest, "assets/nostr-vpn-v4.0.37-android-arm64.apk")
        assertTrue(
            "expected joined URL to keep manifest base, got: $joined",
            joined.endsWith("/latest/assets/nostr-vpn-v4.0.37-android-arm64.apk"),
        )
    }

    @Test
    fun liveReleaseManifestExposesAndroidApk() {
        // Hits the production release manifest. Fails loudly if upload.iris.to
        // ever changes the JSON shape or the APK suffix our updater filters on.
        val body = URL(MANIFEST_URL).readText()
        val apkPattern = Regex(""""name"\s*:\s*"[^"]*-android-arm64\.apk"""")
        assertTrue(
            "release.json must contain a *-android-arm64.apk asset; got: $body",
            apkPattern.containsMatchIn(body),
        )
        val tagPattern = Regex(""""tag"\s*:\s*"v\d+""")
        assertTrue(
            "release.json must contain a versioned tag; got: $body",
            tagPattern.containsMatchIn(body),
        )
    }

    private companion object {
        const val MANIFEST_URL =
            "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Fnostr-vpn/latest/release.json"
    }
}
