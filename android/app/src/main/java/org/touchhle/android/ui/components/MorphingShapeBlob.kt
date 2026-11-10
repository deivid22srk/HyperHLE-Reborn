/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android.ui.components

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import org.touchhle.android.ui.theme.HyperHLEAccentShapes

/**
 * A large decorative blob that continuously "breathes" between two
 * MaterialShapes with the expressive spring feel of the theme: the geometry
 * cross-fades between the organic cookie shape and the sunny shape while a
 * slow rotation and a springy scale pulse run on top.
 *
 * Only alpha, rotation and scale are animated, which stay on the render
 * thread, so the blob never costs a recomposition per frame.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun MorphingShapeBlob(
    modifier: Modifier = Modifier,
    diameter: Dp = 168.dp,
    startShape: Shape = HyperHLEAccentShapes.cookie(),
    endShape: Shape = HyperHLEAccentShapes.sunny(),
    content: @Composable () -> Unit = {},
) {
    val transition = rememberInfiniteTransition(label = "shapeBlob")
    // 0 -> 1 -> 0: drives the cross-fade between the two geometries.
    val morphProgress by transition.animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 2600, easing = FastOutSlowInEasing),
            repeatMode = RepeatMode.Reverse,
        ),
        label = "morphProgress",
    )
    // Very slow continuous rotation keeps the blob alive.
    val rotation by transition.animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 24_000, easing = LinearEasing),
        ),
        label = "rotation",
    )
    // Springy breathing scale (duration-based spec: spring cannot repeat).
    val scale by transition.animateFloat(
        initialValue = 0.96f,
        targetValue = 1.04f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 1300, easing = FastOutSlowInEasing),
            repeatMode = RepeatMode.Reverse,
        ),
        label = "scale",
    )

    Box(
        modifier = modifier
            .size(diameter)
            .scale(scale),
        contentAlignment = Alignment.Center,
    ) {
        // Cross-fade the two expressive geometries; combined with the slow
        // rotation this reads as a shape morph.
        BlobLayer(
            shape = startShape,
            alpha = 1f - morphProgress,
            rotation = rotation,
            diameter = diameter,
        )
        BlobLayer(
            shape = endShape,
            alpha = morphProgress,
            rotation = rotation,
            diameter = diameter,
        )
        content()
    }
}

@Composable
private fun BlobLayer(
    shape: Shape,
    alpha: Float,
    rotation: Float,
    diameter: Dp,
) {
    val brush = Brush.linearGradient(
        colors = listOf(
            MaterialTheme.colorScheme.primaryContainer,
            MaterialTheme.colorScheme.secondaryContainer,
            MaterialTheme.colorScheme.tertiaryContainer,
        ),
    )
    Box(
        modifier = Modifier
            .size(diameter)
            .rotate(rotation)
            .clip(shape)
            .background(brush, shape)
            .alpha(alpha),
    )
}
