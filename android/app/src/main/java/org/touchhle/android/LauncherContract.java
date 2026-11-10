/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android;

/**
 * Shared contract between the Material 3 Expressive launcher UI
 * ({@link LauncherActivity}) and the SDL emulation host ({@link MainActivity}).
 *
 * The launcher lists the .ipa files stored in the touchHLE apps directory and
 * starts {@link MainActivity} with the app the user picked, plus any global
 * emulator options configured in the settings screen. MainActivity feeds both
 * into the Rust core through SDLActivity#getArguments(), which maps them onto
 * the command line of the native touchHLE binary — no Rust-side changes are
 * involved.
 */
public final class LauncherContract {
    /** Intent extra: absolute path of the .ipa file to launch. */
    public static final String EXTRA_APP_PATH =
            "org.touchhle.android.extra.APP_PATH";

    /**
     * Intent extra: newline-separated touchHLE command-line options (e.g.
     * "--landscape-left", "--scale-hack=2"). Parsed before the app path by
     * the core's argument parser.
     */
    public static final String EXTRA_OPTIONS =
            "org.touchhle.android.extra.OPTIONS";

    private LauncherContract() {
    }
}
