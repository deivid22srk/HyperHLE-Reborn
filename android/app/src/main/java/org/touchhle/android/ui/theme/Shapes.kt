/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android.ui.theme

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.Shapes
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Shape

/**
 * Corner-shape system for the whole launcher. Material 3 Expressive leans on
 * rounder, more organic corners than the default Material 3 scale, so every
 * role is bumped up a notch: small chips get fully-rounded ends, cards and
 * sheets sit around 24-28dp, and large surfaces use 40dp+ corners.
 */
fun hyperHLEShapes(): Shapes = Shapes(
    extraSmall = RoundedCornerShape(10),
    small = RoundedCornerShape(16),
    medium = RoundedCornerShape(24),
    large = RoundedCornerShape(32),
    extraLarge = RoundedCornerShape(40),
)

/**
 * Expressive accent shapes used as decorative containers (empty-state blob,
 * app icon plates, floating action affordances). They come from the
 * MaterialShapes "cookie"/organic family so containers no longer have to be
 * plain rounded rectangles.
 *
 * `MaterialShapes` is still experimental in Material 3 1.5.x, hence the
 * opt-in. `RoundedPolygon.toShape()` is composable (it caches geometry in
 * the composition), so these helpers are composable too.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
object HyperHLEAccentShapes {
    /** Soft-square shape for app icon plates. */
    @Composable
    fun iconPlate(): Shape = MaterialShapes.Square.toShape()

    /** Nine-sided cookie shape for the empty-state blob. */
    @Composable
    fun cookie(): Shape = MaterialShapes.Cookie9Sided.toShape()

    /** Sunny (flower/gear) shape used as the morph partner of the blob. */
    @Composable
    fun sunny(): Shape = MaterialShapes.Sunny.toShape()

    /** Pill-shaped shape used for chips. */
    @Composable
    fun pill(): Shape = MaterialShapes.Pill.toShape()

    /** Clover shape used as a secondary decorative accent. */
    @Composable
    fun clover(): Shape = MaterialShapes.Clover.toShape()

    /** 12-sided cookie shape used as another decorative accent. */
    @Composable
    fun cookie12(): Shape = MaterialShapes.Cookie12Sided.toShape()
}
