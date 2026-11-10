/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.PhoneIphone
import androidx.compose.material.icons.outlined.PlayCircleOutline
import androidx.compose.material3.Card
import androidx.compose.material3.CardColors
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.text.DateFormat
import java.util.Date
import java.util.Locale
import org.touchhle.android.InstalledApp
import org.touchhle.android.R
import org.touchhle.android.ui.theme.HyperHLEAccentShapes

/**
 * Library card for one imported .ipa, in the Material 3 Expressive card
 * language: a roomy container with extra-large rounded corners, an icon
 * plate in one of the organic MaterialShapes, emphasized title typography
 * and a play affordance with a comfortable touch target.
 */
@Composable
fun AppCard(
    app: InstalledApp,
    accentIndex: Int,
    onLaunch: (InstalledApp) -> Unit,
    onDelete: (InstalledApp) -> Unit,
    modifier: Modifier = Modifier,
) {
    val cardColors: CardColors = CardDefaults.cardColors(
        containerColor = when (accentIndex % 3) {
            0 -> MaterialTheme.colorScheme.surfaceContainerLow
            1 -> MaterialTheme.colorScheme.surfaceContainer
            else -> MaterialTheme.colorScheme.surfaceContainerLow
        },
    )
    val plateContainer = when (accentIndex % 3) {
        0 -> MaterialTheme.colorScheme.primaryContainer
        1 -> MaterialTheme.colorScheme.secondaryContainer
        else -> MaterialTheme.colorScheme.tertiaryContainer
    }
    val plateContent = when (accentIndex % 3) {
        0 -> MaterialTheme.colorScheme.onPrimaryContainer
        1 -> MaterialTheme.colorScheme.onSecondaryContainer
        else -> MaterialTheme.colorScheme.onTertiaryContainer
    }

    Card(
        onClick = { onLaunch(app) },
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        colors = cardColors,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 20.dp, vertical = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // Organic icon plate; the shape cycles through the accent
            // MaterialShapes so the grid does not look uniform.
            val plateShape = when (accentIndex % 4) {
                0 -> HyperHLEAccentShapes.iconPlate()
                1 -> HyperHLEAccentShapes.cookie()
                2 -> HyperHLEAccentShapes.clover()
                else -> HyperHLEAccentShapes.cookie12()
            }
            androidx.compose.foundation.layout.Box(
                modifier = Modifier
                    .size(56.dp)
                    .clip(plateShape)
                    .background(plateContainer, plateShape),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = Icons.Outlined.PhoneIphone,
                    contentDescription = null,
                    tint = plateContent,
                )
            }

            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    text = app.displayName,
                    style = MaterialTheme.typography.titleMediumEmphasized,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = formatMetadata(app),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }

            FilledIconButton(
                onClick = { onDelete(app) },
                colors = IconButtonDefaults.filledIconButtonColors(
                    containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                    contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                ),
            ) {
                Icon(
                    imageVector = Icons.Outlined.DeleteOutline,
                    contentDescription = stringResource(R.string.library_delete_description),
                )
            }

            FilledIconButton(
                onClick = { onLaunch(app) },
                colors = IconButtonDefaults.filledIconButtonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary,
                ),
            ) {
                Icon(
                    imageVector = Icons.Outlined.PlayCircleOutline,
                    contentDescription = stringResource(
                        R.string.library_launch_description,
                        app.displayName,
                    ),
                )
            }
        }
    }
}

private fun formatMetadata(app: InstalledApp): String {
    val size = when {
        app.sizeBytes >= 1L shl 30 ->
            String.format(Locale.US, "%.2f GB", app.sizeBytes.toDouble() / (1L shl 30))
        app.sizeBytes >= 1L shl 20 ->
            String.format(Locale.US, "%.1f MB", app.sizeBytes.toDouble() / (1L shl 20))
        else ->
            String.format(Locale.US, "%.0f KB", app.sizeBytes.toDouble() / (1L shl 10))
    }
    val date = DateFormat.getDateInstance(DateFormat.MEDIUM, Locale.getDefault())
        .format(Date(app.lastModified))
    return "$size · $date"
}
