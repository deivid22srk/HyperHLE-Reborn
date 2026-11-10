/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/**
 * Material 3 type scale used by the HyperHLE launcher.
 *
 * The platform Roboto font family is kept so the launcher needs no bundled
 * fonts (keeping the APK small and text rendering identical to the rest of
 * the system). The Material 3 Expressive *emphasized* variants are used
 * through `MaterialTheme.typography.<role>Emphasized` for headlines and key
 * data (see [org.touchhle.android.ui.library.LibraryScreen]).
 */
fun hyperHLETypography(): Typography {
    val base = Typography()
    return base.copy(
        // Slightly tightened, bolder display style for the flexible top bar.
        displaySmallEmphasized = base.displaySmall.copy(
            fontWeight = FontWeight.Bold,
            letterSpacing = (-0.25).sp,
        ),
        headlineSmallEmphasized = base.headlineSmall.copy(fontWeight = FontWeight.Bold),
        titleLargeEmphasized = base.titleLarge.copy(fontWeight = FontWeight.Bold),
        titleMediumEmphasized = base.titleMedium.copy(fontWeight = FontWeight.SemiBold),
        bodyLargeEmphasized = base.bodyLarge.copy(fontWeight = FontWeight.SemiBold),
        labelLargeEmphasized = base.labelLarge.copy(fontWeight = FontWeight.Bold),
    )
}

/** Small helper used for monospaced bits (file sizes, option values). */
val sizeTextStyle: TextStyle =
    TextStyle(fontFamily = FontFamily.Monospace, fontSize = 12.sp, fontWeight = FontWeight.Medium)
