/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MediumFlexibleTopAppBar
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import org.touchhle.android.EmulatorOptions
import org.touchhle.android.FpsLimit
import org.touchhle.android.OptionsStore
import org.touchhle.android.R
import org.touchhle.android.VirtualOrientation

/**
 * Emulator settings, in the Material 3 Expressive language:
 *  - a [MediumFlexibleTopAppBar];
 *  - segmented rows ([SingleChoiceSegmentedButtonRow]) for the discrete
 *    choices (startup orientation, framerate limit);
 *  - expressive sliders for the numeric scales (scale hack, UI scale,
 *    deadzone);
 *  - roomy [ListItem] rows with switches for the boolean options.
 *
 * Every control edits the hoisted [EmulatorOptions] through
 * [onOptionsChange]; the ViewModel persists them and they are applied as
 * command-line options when a game is launched.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class, ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    options: EmulatorOptions,
    onOptionsChange: ((EmulatorOptions) -> EmulatorOptions) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(bottom = 48.dp),
    ) {
        SectionHeader(stringResource(R.string.settings_section_display))
        Text(
            text = stringResource(R.string.settings_orientation_label),
            style = MaterialTheme.typography.titleMediumEmphasized,
            modifier = Modifier.padding(horizontal = 20.dp),
        )
        SingleChoiceSegmentedButtonRow(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 20.dp, vertical = 8.dp),
        ) {
            VirtualOrientation.entries.forEachIndexed { index, orientation ->
                SegmentedButton(
                    selected = options.orientation == orientation,
                    onClick = {
                        onOptionsChange { it.copy(orientation = orientation) }
                    },
                    shape = SegmentedButtonDefaults.itemShape(
                        index = index,
                        count = VirtualOrientation.entries.size,
                    ),
                    label = {
                        Text(
                            stringResource(orientation.labelRes),
                            maxLines = 1,
                        )
                    },
                )
            }
        }

        SliderSetting(
            title = stringResource(R.string.settings_scale_hack_label),
            supporting = stringResource(
                R.string.settings_scale_hack_supporting,
                options.scaleHack,
            ),
            value = options.scaleHack.toFloat(),
            onValueChange = { value ->
                onOptionsChange { it.copy(scaleHack = value.toInt()) }
            },
            valueRange = OptionsStore.MIN_SCALE_HACK.toFloat()..
                OptionsStore.MAX_SCALE_HACK.toFloat(),
            steps = OptionsStore.MAX_SCALE_HACK - OptionsStore.MIN_SCALE_HACK - 1,
        )

        SliderSetting(
            title = stringResource(R.string.settings_ui_scale_label),
            supporting = stringResource(
                R.string.settings_ui_scale_supporting,
                options.uiScale,
            ),
            value = options.uiScale.toFloat(),
            onValueChange = { value ->
                onOptionsChange { it.copy(uiScale = value.toInt()) }
            },
            valueRange = OptionsStore.MIN_UI_SCALE.toFloat()..
                OptionsStore.MAX_UI_SCALE.toFloat(),
            steps = OptionsStore.MAX_UI_SCALE - OptionsStore.MIN_UI_SCALE - 1,
        )

        SwitchSetting(
            title = stringResource(R.string.settings_preserve_aspect_label),
            supporting = stringResource(R.string.settings_preserve_aspect_supporting),
            checked = options.preserveAspectRatio,
            onCheckedChange = { checked ->
                onOptionsChange { it.copy(preserveAspectRatio = checked) }
            },
        )

        SwitchSetting(
            title = stringResource(R.string.settings_overlay_label),
            supporting = stringResource(R.string.settings_overlay_supporting),
            checked = options.performanceOverlay,
            onCheckedChange = { checked ->
                onOptionsChange { it.copy(performanceOverlay = checked) }
            },
        )

        Text(
            text = stringResource(R.string.settings_fps_limit_label),
            style = MaterialTheme.typography.titleMediumEmphasized,
            modifier = Modifier.padding(horizontal = 20.dp),
        )
        SingleChoiceSegmentedButtonRow(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 20.dp, vertical = 8.dp),
        ) {
            FpsLimit.entries.forEachIndexed { index, limit ->
                SegmentedButton(
                    selected = options.fpsLimit == limit,
                    onClick = {
                        onOptionsChange { it.copy(fpsLimit = limit) }
                    },
                    shape = SegmentedButtonDefaults.itemShape(
                        index = index,
                        count = FpsLimit.entries.size,
                    ),
                    label = { Text(stringResource(limit.labelRes), maxLines = 1) },
                )
            }
        }

        SectionHeader(stringResource(R.string.settings_section_input))
        SwitchSetting(
            title = stringResource(R.string.settings_tilt_label),
            supporting = stringResource(R.string.settings_tilt_supporting),
            checked = options.tiltControls,
            onCheckedChange = { checked ->
                onOptionsChange { it.copy(tiltControls = checked) }
            },
        )

        SliderSetting(
            title = stringResource(R.string.settings_deadzone_label),
            supporting = stringResource(
                R.string.settings_deadzone_supporting,
                (options.deadzone * 100).toInt(),
            ),
            value = options.deadzone,
            onValueChange = { value ->
                onOptionsChange { it.copy(deadzone = value) }
            },
            valueRange = 0f..0.5f,
        )

        SectionHeader(stringResource(R.string.settings_section_system))
        SwitchSetting(
            title = stringResource(R.string.settings_network_label),
            supporting = stringResource(R.string.settings_network_supporting),
            checked = options.allowNetworkAccess,
            onCheckedChange = { checked ->
                onOptionsChange { it.copy(allowNetworkAccess = checked) }
            },
        )

        SwitchSetting(
            title = stringResource(R.string.settings_game_center_label),
            supporting = stringResource(R.string.settings_game_center_supporting),
            checked = options.gameCenterSignIn,
            onCheckedChange = { checked ->
                onOptionsChange { it.copy(gameCenterSignIn = checked) }
            },
        )

        SectionHeader(stringResource(R.string.settings_section_about))
        Text(
            text = stringResource(R.string.settings_about_body),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 20.dp),
        )
    }
}

/** The expressive top app bar of the settings tab. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SettingsTopBar(modifier: Modifier = Modifier) {
    MediumFlexibleTopAppBar(
        title = { Text(stringResource(R.string.tab_settings)) },
        modifier = modifier,
    )
}

@Composable
private fun SectionHeader(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.titleLargeEmphasized,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(
            start = 20.dp,
            end = 20.dp,
            top = 32.dp,
            bottom = 8.dp,
        ),
    )
}

@Composable
private fun SliderSetting(
    title: String,
    supporting: String,
    value: Float,
    onValueChange: (Float) -> Unit,
    valueRange: ClosedFloatingPointRange<Float>,
    steps: Int = 0,
) {
    ListItem(
        headlineContent = { Text(title, style = MaterialTheme.typography.titleMediumEmphasized) },
        supportingContent = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(supporting, style = MaterialTheme.typography.bodySmall)
                Slider(
                    value = value,
                    onValueChange = onValueChange,
                    valueRange = valueRange,
                    steps = steps,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        modifier = Modifier.padding(horizontal = 8.dp),
    )
}

@Composable
private fun SwitchSetting(
    title: String,
    supporting: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    ListItem(
        headlineContent = { Text(title, style = MaterialTheme.typography.titleMediumEmphasized) },
        supportingContent = { Text(supporting, style = MaterialTheme.typography.bodySmall) },
        trailingContent = {
            Switch(checked = checked, onCheckedChange = onCheckedChange)
        },
        modifier = Modifier.padding(horizontal = 8.dp),
    )
}
