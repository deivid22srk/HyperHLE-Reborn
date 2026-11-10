/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Apps
import androidx.compose.material.icons.outlined.ChevronLeft
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.WideNavigationRail
import androidx.compose.material3.WideNavigationRailItem
import androidx.compose.material3.WideNavigationRailValue
import androidx.compose.material3.rememberWideNavigationRailState
import androidx.compose.material3.windowsizeclass.ExperimentalMaterial3WindowSizeClassApi
import androidx.compose.material3.windowsizeclass.WindowWidthSizeClass
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import kotlinx.coroutines.launch
import org.touchhle.android.R

/** The two top-level destinations of the launcher. */
enum class LauncherDestination(val labelRes: Int) {
    LIBRARY(R.string.tab_library),
    SETTINGS(R.string.tab_settings),
}

/**
 * Adaptive shell for the launcher, in the Material 3 Expressive navigation
 * language:
 *
 * - compact widths (phones): bottom [NavigationBar];
 * - medium/expanded widths (tablets, foldables, landscape desktop mode):
 *   [WideNavigationRail] with an expand/collapse header.
 *
 * The selected destination is hoisted by the caller; screens are switched
 * with expressive (spring) transitions in [org.touchhle.android.LauncherActivity].
 */
@OptIn(
    ExperimentalMaterial3ExpressiveApi::class,
    ExperimentalMaterial3Api::class,
    ExperimentalMaterial3WindowSizeClassApi::class,
)
@Composable
fun HyperHLEScaffold(
    windowWidthSizeClass: WindowWidthSizeClass,
    selectedDestination: LauncherDestination,
    onDestinationSelected: (LauncherDestination) -> Unit,
    snackbarHostState: SnackbarHostState,
    topBar: @Composable () -> Unit,
    content: @Composable () -> Unit,
) {
    val useRail = windowWidthSizeClass != WindowWidthSizeClass.Compact

    if (useRail) {
        val railState = rememberWideNavigationRailState(WideNavigationRailValue.Collapsed)
        val scope = rememberCoroutineScope()
        val railExpanded = railState.currentValue == WideNavigationRailValue.Expanded
        Scaffold(
            topBar = topBar,
            snackbarHost = { SnackbarHost(snackbarHostState) },
        ) { innerPadding ->
            Row(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(innerPadding),
            ) {
                WideNavigationRail(
                    state = railState,
                    header = {
                        IconButton(
                            onClick = { scope.launch { railState.toggle() } },
                        ) {
                            Icon(
                                imageVector = if (railExpanded) {
                                    Icons.Outlined.ChevronLeft
                                } else {
                                    Icons.Outlined.ChevronRight
                                },
                                contentDescription = stringResource(
                                    if (railExpanded) {
                                        R.string.rail_collapse_description
                                    } else {
                                        R.string.rail_expand_description
                                    },
                                ),
                            )
                        }
                    },
                ) {
                    LauncherDestination.entries.forEach { destination ->
                        WideNavigationRailItem(
                            selected = destination == selectedDestination,
                            onClick = { onDestinationSelected(destination) },
                            icon = {
                                Icon(
                                    imageVector = destination.icon(),
                                    contentDescription = stringResource(destination.labelRes),
                                )
                            },
                            label = { Text(stringResource(destination.labelRes)) },
                            railExpanded = railExpanded,
                        )
                    }
                }
                Box(
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxSize(),
                ) {
                    content()
                }
            }
        }
    } else {
        Scaffold(
            topBar = topBar,
            bottomBar = {
                NavigationBar {
                    LauncherDestination.entries.forEach { destination ->
                        NavigationBarItem(
                            selected = destination == selectedDestination,
                            onClick = { onDestinationSelected(destination) },
                            icon = {
                                Icon(
                                    imageVector = destination.icon(),
                                    contentDescription = stringResource(destination.labelRes),
                                )
                            },
                            label = { Text(stringResource(destination.labelRes)) },
                        )
                    }
                }
            },
            snackbarHost = { SnackbarHost(snackbarHostState) },
        ) { innerPadding ->
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(innerPadding),
            ) {
                content()
            }
        }
    }
}

private fun LauncherDestination.icon() = when (this) {
    LauncherDestination.LIBRARY -> Icons.Outlined.Apps
    LauncherDestination.SETTINGS -> Icons.Outlined.Settings
}
