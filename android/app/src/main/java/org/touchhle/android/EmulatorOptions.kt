/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android

import android.content.Context
import android.content.SharedPreferences
import java.util.Locale

/**
 * Startup orientation of the virtual device, mapped onto the touchHLE
 * command-line flags (see OPTIONS_HELP.txt in the repository root).
 */
enum class VirtualOrientation(val flag: String?, val labelRes: Int) {
    PORTRAIT(null, R.string.settings_orientation_portrait),
    LANDSCAPE_LEFT("--landscape-left", R.string.settings_orientation_landscape_left),
    LANDSCAPE_RIGHT("--landscape-right", R.string.settings_orientation_landscape_right),
    UPSIDE_DOWN("--upside-down", R.string.settings_orientation_upside_down),
}

/**
 * Global emulator options editable from the Material 3 Expressive settings
 * screen.
 *
 * The options are persisted in SharedPreferences and translated into
 * touchHLE command-line arguments at launch time (see [toArguments]). This
 * deliberately mirrors how the desktop build receives options, so the
 * per-app overrides in `touchHLE_options.txt` keep working untouched: the
 * core applies command-line options on top of the per-app file options.
 *
 * Defaults documented in OPTIONS_HELP.txt:
 *  - portrait startup orientation
 *  - scale hack off (1x)
 *  - UI scale 2x
 *  - stretched output (aspect ratio not preserved)
 *  - on-screen performance overlay enabled
 *  - framerate limit 60
 *  - analog-stick tilt mapping enabled
 *  - stick deadzone 0.1
 *  - network access disabled
 *  - local Game Center sign-in disabled
 */
data class EmulatorOptions(
    val orientation: VirtualOrientation = VirtualOrientation.PORTRAIT,
    val scaleHack: Int = 1,
    val uiScale: Int = 2,
    val preserveAspectRatio: Boolean = false,
    val performanceOverlay: Boolean = true,
    val fpsLimit: FpsLimit = FpsLimit.FPS_60,
    val tiltControls: Boolean = true,
    val deadzone: Float = 0.1f,
    val allowNetworkAccess: Boolean = false,
    val gameCenterSignIn: Boolean = false,
)

/** Framerate limit presets exposed in the settings screen. */
enum class FpsLimit(val flag: String?, val labelRes: Int) {
    FPS_30("--fps-limit=30", R.string.settings_fps_limit_30),
    FPS_60(null, R.string.settings_fps_limit_60),
    OFF("--fps-limit=off", R.string.settings_fps_limit_off),
}

/**
 * Translate the options into the touchHLE command line. Only non-default
 * values emit a flag where the flag itself has a default; boolean options
 * always emit the matching pair so the intent can never drift from the state
 * shown in the UI.
 */
fun EmulatorOptions.toArguments(): List<String> = buildList {
    orientation.flag?.let { add(it) }
    if (scaleHack > 1) {
        add("--scale-hack=$scaleHack")
    }
    if (uiScale > 1) {
        add("--ui-scale=$uiScale")
    }
    add(if (preserveAspectRatio) "--preserve-aspect-ratio" else "--no-preserve-aspect-ratio")
    add(if (performanceOverlay) "--overlay=on" else "--overlay=off")
    fpsLimit.flag?.let { add(it) }
    add(if (tiltControls) "--enable-analog-stick-tilt-controls" else "--disable-analog-stick-tilt-controls")
    add("--deadzone=${String.format(Locale.US, "%.2f", deadzone)}")
    add(if (allowNetworkAccess) "--allow-network-access" else "--no-network-access")
    if (gameCenterSignIn) {
        add("--game-center-authenticated")
    }
}

/**
 * Persistence for [EmulatorOptions]. Plain SharedPreferences: the values are
 * a handful of scalars, so a full preferences/datastore library would be
 * overkill, and the file lives in the app-private storage just like the rest
 * of touchHLE's own state.
 */
class OptionsStore(context: Context) {
    private val prefs: SharedPreferences =
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    fun load(): EmulatorOptions {
        val orientation =
            VirtualOrientation.entries.firstOrNull {
                it.name == prefs.getString(KEY_ORIENTATION, null)
            } ?: VirtualOrientation.PORTRAIT
        return EmulatorOptions(
            orientation = orientation,
            scaleHack = prefs.getInt(KEY_SCALE_HACK, 1).coerceIn(MIN_SCALE_HACK, MAX_SCALE_HACK),
            uiScale = prefs.getInt(KEY_UI_SCALE, 2).coerceIn(MIN_UI_SCALE, MAX_UI_SCALE),
            preserveAspectRatio = prefs.getBoolean(KEY_PRESERVE_ASPECT, false),
            performanceOverlay = prefs.getBoolean(KEY_PERFORMANCE_OVERLAY, true),
            fpsLimit = FpsLimit.entries.firstOrNull {
                it.name == prefs.getString(KEY_FPS_LIMIT, null)
            } ?: FpsLimit.FPS_60,
            tiltControls = prefs.getBoolean(KEY_TILT_CONTROLS, true),
            deadzone = prefs.getFloat(KEY_DEADZONE, 0.1f).coerceIn(0f, 1f),
            allowNetworkAccess = prefs.getBoolean(KEY_ALLOW_NETWORK, false),
            gameCenterSignIn = prefs.getBoolean(KEY_GAME_CENTER, false),
        )
    }

    fun save(options: EmulatorOptions) {
        prefs.edit()
            .putString(KEY_ORIENTATION, options.orientation.name)
            .putInt(KEY_SCALE_HACK, options.scaleHack)
            .putInt(KEY_UI_SCALE, options.uiScale)
            .putBoolean(KEY_PRESERVE_ASPECT, options.preserveAspectRatio)
            .putBoolean(KEY_PERFORMANCE_OVERLAY, options.performanceOverlay)
            .putString(KEY_FPS_LIMIT, options.fpsLimit.name)
            .putBoolean(KEY_TILT_CONTROLS, options.tiltControls)
            .putFloat(KEY_DEADZONE, options.deadzone)
            .putBoolean(KEY_ALLOW_NETWORK, options.allowNetworkAccess)
            .putBoolean(KEY_GAME_CENTER, options.gameCenterSignIn)
            .apply()
    }

    companion object {
        const val MIN_SCALE_HACK = 1
        const val MAX_SCALE_HACK = 4
        const val MIN_UI_SCALE = 1
        const val MAX_UI_SCALE = 4

        private const val PREFS_NAME = "hyperhle_launcher_settings"
        private const val KEY_ORIENTATION = "orientation"
        private const val KEY_SCALE_HACK = "scale_hack"
        private const val KEY_UI_SCALE = "ui_scale"
        private const val KEY_PRESERVE_ASPECT = "preserve_aspect_ratio"
        private const val KEY_PERFORMANCE_OVERLAY = "performance_overlay"
        private const val KEY_FPS_LIMIT = "fps_limit"
        private const val KEY_TILT_CONTROLS = "tilt_controls"
        private const val KEY_DEADZONE = "deadzone"
        private const val KEY_ALLOW_NETWORK = "allow_network_access"
        private const val KEY_GAME_CENTER = "game_center_sign_in"
    }
}
