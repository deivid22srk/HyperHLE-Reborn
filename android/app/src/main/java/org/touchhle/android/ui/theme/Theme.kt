/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MotionScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalContext

/**
 * The static light color scheme used below Android 12, where Material You
 * dynamic color extraction is not available.
 */
private val LightColors: ColorScheme = lightColorScheme(
    primary = HyperHLEPalette.LightPrimary,
    onPrimary = HyperHLEPalette.LightOnPrimary,
    primaryContainer = HyperHLEPalette.LightPrimaryContainer,
    onPrimaryContainer = HyperHLEPalette.LightOnPrimaryContainer,
    secondary = HyperHLEPalette.LightSecondary,
    onSecondary = HyperHLEPalette.LightOnSecondary,
    secondaryContainer = HyperHLEPalette.LightSecondaryContainer,
    onSecondaryContainer = HyperHLEPalette.LightOnSecondaryContainer,
    tertiary = HyperHLEPalette.LightTertiary,
    onTertiary = HyperHLEPalette.LightOnTertiary,
    tertiaryContainer = HyperHLEPalette.LightTertiaryContainer,
    onTertiaryContainer = HyperHLEPalette.LightOnTertiaryContainer,
    error = HyperHLEPalette.LightError,
    onError = HyperHLEPalette.LightOnError,
    errorContainer = HyperHLEPalette.LightErrorContainer,
    onErrorContainer = HyperHLEPalette.LightOnErrorContainer,
    background = HyperHLEPalette.LightBackground,
    onBackground = HyperHLEPalette.LightOnBackground,
    surface = HyperHLEPalette.LightSurface,
    onSurface = HyperHLEPalette.LightOnSurface,
    surfaceVariant = HyperHLEPalette.LightSurfaceVariant,
    onSurfaceVariant = HyperHLEPalette.LightOnSurfaceVariant,
    outline = HyperHLEPalette.LightOutline,
    outlineVariant = HyperHLEPalette.LightOutlineVariant,
    scrim = HyperHLEPalette.LightScrim,
    inverseSurface = HyperHLEPalette.LightInverseSurface,
    inverseOnSurface = HyperHLEPalette.LightInverseOnSurface,
    inversePrimary = HyperHLEPalette.LightInversePrimary,
    surfaceDim = HyperHLEPalette.LightSurfaceDim,
    surfaceBright = HyperHLEPalette.LightSurfaceBright,
    surfaceContainerLowest = HyperHLEPalette.LightSurfaceContainerLowest,
    surfaceContainerLow = HyperHLEPalette.LightSurfaceContainerLow,
    surfaceContainer = HyperHLEPalette.LightSurfaceContainer,
    surfaceContainerHigh = HyperHLEPalette.LightSurfaceContainerHigh,
    surfaceContainerHighest = HyperHLEPalette.LightSurfaceContainerHighest,
)

/**
 * The static dark color scheme used below Android 12.
 */
private val DarkColors: ColorScheme = darkColorScheme(
    primary = HyperHLEPalette.DarkPrimary,
    onPrimary = HyperHLEPalette.DarkOnPrimary,
    primaryContainer = HyperHLEPalette.DarkPrimaryContainer,
    onPrimaryContainer = HyperHLEPalette.DarkOnPrimaryContainer,
    secondary = HyperHLEPalette.DarkSecondary,
    onSecondary = HyperHLEPalette.DarkOnSecondary,
    secondaryContainer = HyperHLEPalette.DarkSecondaryContainer,
    onSecondaryContainer = HyperHLEPalette.DarkOnSecondaryContainer,
    tertiary = HyperHLEPalette.DarkTertiary,
    onTertiary = HyperHLEPalette.DarkOnTertiary,
    tertiaryContainer = HyperHLEPalette.DarkTertiaryContainer,
    onTertiaryContainer = HyperHLEPalette.DarkOnTertiaryContainer,
    error = HyperHLEPalette.DarkError,
    onError = HyperHLEPalette.DarkOnError,
    errorContainer = HyperHLEPalette.DarkErrorContainer,
    onErrorContainer = HyperHLEPalette.DarkOnErrorContainer,
    background = HyperHLEPalette.DarkBackground,
    onBackground = HyperHLEPalette.DarkOnBackground,
    surface = HyperHLEPalette.DarkSurface,
    onSurface = HyperHLEPalette.DarkOnSurface,
    surfaceVariant = HyperHLEPalette.DarkSurfaceVariant,
    onSurfaceVariant = HyperHLEPalette.DarkOnSurfaceVariant,
    outline = HyperHLEPalette.DarkOutline,
    outlineVariant = HyperHLEPalette.DarkOutlineVariant,
    scrim = HyperHLEPalette.DarkScrim,
    inverseSurface = HyperHLEPalette.DarkInverseSurface,
    inverseOnSurface = HyperHLEPalette.DarkInverseOnSurface,
    inversePrimary = HyperHLEPalette.DarkInversePrimary,
    surfaceDim = HyperHLEPalette.DarkSurfaceDim,
    surfaceBright = HyperHLEPalette.DarkSurfaceBright,
    surfaceContainerLowest = HyperHLEPalette.DarkSurfaceContainerLowest,
    surfaceContainerLow = HyperHLEPalette.DarkSurfaceContainerLow,
    surfaceContainer = HyperHLEPalette.DarkSurfaceContainer,
    surfaceContainerHigh = HyperHLEPalette.DarkSurfaceContainerHigh,
    surfaceContainerHighest = HyperHLEPalette.DarkSurfaceContainerHighest,
)

/**
 * Root theme of the HyperHLE-Reborn launcher.
 *
 * - Uses [MaterialExpressiveTheme] with [MotionScheme.expressive] so every
 *   Material component inside picks up the expressive motion (spring-driven)
 *   defaults.
 * - On Android 12+ (API 31) the wallpaper-based Material You dynamic color
 *   schemes (`dynamicLightColorScheme`/`dynamicDarkColorScheme`) are applied;
 *   on older devices the static light/dark baseline scheme above is used.
 */
@Composable
fun HyperHLETheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    dynamicColor: Boolean = true,
    content: @Composable () -> Unit,
) {
    val colorScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }
        darkTheme -> DarkColors
        else -> LightColors
    }

    MaterialExpressiveTheme(
        colorScheme = colorScheme,
        motionScheme = MotionScheme.expressive(),
        shapes = hyperHLEShapes(),
        typography = hyperHLETypography(),
        content = content,
    )
}
