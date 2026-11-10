/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android.ui.theme

import androidx.compose.ui.graphics.Color

/**
 * Static fallback palettes for devices running below Android 12 (API 31),
 * where Material You dynamic color is not available.
 *
 * The values follow the Material 3 baseline tonal mappings, so contrast
 * ratios of the on-color pairs stay accessible in both themes. On Android
 * 12+ the theme prefers `dynamicLightColorScheme`/`dynamicDarkColorScheme`
 * instead (see [HyperHLETheme]).
 */
object HyperHLEPalette {
    // ---- Light scheme -------------------------------------------------
    val LightPrimary = Color(0xFF6750A4)
    val LightOnPrimary = Color(0xFFFFFFFF)
    val LightPrimaryContainer = Color(0xFFEADDFF)
    val LightOnPrimaryContainer = Color(0xFF21005D)
    val LightSecondary = Color(0xFF625B71)
    val LightOnSecondary = Color(0xFFFFFFFF)
    val LightSecondaryContainer = Color(0xFFE8DEF8)
    val LightOnSecondaryContainer = Color(0xFF1D192B)
    val LightTertiary = Color(0xFF7D5260)
    val LightOnTertiary = Color(0xFFFFFFFF)
    val LightTertiaryContainer = Color(0xFFFFD8E4)
    val LightOnTertiaryContainer = Color(0xFF31111D)
    val LightError = Color(0xFFB3261E)
    val LightOnError = Color(0xFFFFFFFF)
    val LightErrorContainer = Color(0xFFF9DEDC)
    val LightOnErrorContainer = Color(0xFF410E0B)
    val LightBackground = Color(0xFFFDF8FF)
    val LightOnBackground = Color(0xFF1D1B20)
    val LightSurface = Color(0xFFFDF8FF)
    val LightOnSurface = Color(0xFF1D1B20)
    val LightSurfaceVariant = Color(0xFFE7E0EC)
    val LightOnSurfaceVariant = Color(0xFF49454F)
    val LightOutline = Color(0xFF79747E)
    val LightOutlineVariant = Color(0xFFCAC4D0)
    val LightScrim = Color(0xFF000000)
    val LightInverseSurface = Color(0xFF322F35)
    val LightInverseOnSurface = Color(0xFFF5EFF7)
    val LightInversePrimary = Color(0xFFD0BCFF)
    val LightSurfaceDim = Color(0xFFDED8E1)
    val LightSurfaceBright = Color(0xFFFDF8FF)
    val LightSurfaceContainerLowest = Color(0xFFFFFFFF)
    val LightSurfaceContainerLow = Color(0xFFF7F2FA)
    val LightSurfaceContainer = Color(0xFFF2ECF4)
    val LightSurfaceContainerHigh = Color(0xFFECE6EE)
    val LightSurfaceContainerHighest = Color(0xFFE6E0E9)

    // ---- Dark scheme --------------------------------------------------
    val DarkPrimary = Color(0xFFD0BCFF)
    val DarkOnPrimary = Color(0xFF381E72)
    val DarkPrimaryContainer = Color(0xFF4F378B)
    val DarkOnPrimaryContainer = Color(0xFFEADDFF)
    val DarkSecondary = Color(0xFFCCC2DC)
    val DarkOnSecondary = Color(0xFF332D41)
    val DarkSecondaryContainer = Color(0xFF4A4458)
    val DarkOnSecondaryContainer = Color(0xFFE8DEF8)
    val DarkTertiary = Color(0xFFEFB8C8)
    val DarkOnTertiary = Color(0xFF492532)
    val DarkTertiaryContainer = Color(0xFF633B48)
    val DarkOnTertiaryContainer = Color(0xFFFFD8E4)
    val DarkError = Color(0xFFF2B8B5)
    val DarkOnError = Color(0xFF601410)
    val DarkErrorContainer = Color(0xFF8C1D18)
    val DarkOnErrorContainer = Color(0xFFF9DEDC)
    val DarkBackground = Color(0xFF141218)
    val DarkOnBackground = Color(0xFFE6E0E9)
    val DarkSurface = Color(0xFF141218)
    val DarkOnSurface = Color(0xFFE6E0E9)
    val DarkSurfaceVariant = Color(0xFF49454F)
    val DarkOnSurfaceVariant = Color(0xFFCAC4D0)
    val DarkOutline = Color(0xFF938F99)
    val DarkOutlineVariant = Color(0xFF49454F)
    val DarkScrim = Color(0xFF000000)
    val DarkInverseSurface = Color(0xFFE6E0E9)
    val DarkInverseOnSurface = Color(0xFF322F35)
    val DarkInversePrimary = Color(0xFF6750A4)
    val DarkSurfaceDim = Color(0xFF141218)
    val DarkSurfaceBright = Color(0xFF3B383E)
    val DarkSurfaceContainerLowest = Color(0xFF0F0D13)
    val DarkSurfaceContainerLow = Color(0xFF1D1B20)
    val DarkSurfaceContainer = Color(0xFF211F26)
    val DarkSurfaceContainerHigh = Color(0xFF2B2930)
    val DarkSurfaceContainerHighest = Color(0xFF36343B)
}
