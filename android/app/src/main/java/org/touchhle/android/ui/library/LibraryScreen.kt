/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android.ui.library

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.Ios
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material3.ButtonGroup
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FloatingActionButtonMenu
import androidx.compose.material3.FloatingActionButtonMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LargeFlexibleTopAppBar
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.ToggleFloatingActionButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.touchhle.android.InstalledApp
import org.touchhle.android.R
import org.touchhle.android.ui.components.AppCard
import org.touchhle.android.ui.components.MorphingShapeBlob
import org.touchhle.android.ui.theme.HyperHLEAccentShapes

/**
 * The launcher home: the library of imported .ipa apps.
 *
 * Material 3 Expressive building blocks used here:
 *  - [LargeFlexibleTopAppBar] with a subtitle and an expressive
 *    [ButtonGroup] in the actions slot;
 *  - a [FloatingActionButtonMenu] with a morphing [ToggleFloatingActionButton]
 *    for the import / file-manager actions;
 *  - an animated [MorphingShapeBlob] empty state;
 *  - adaptive grid columns driven by the window width class.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class, ExperimentalMaterial3Api::class)
@Composable
fun LibraryScreen(
    apps: List<InstalledApp>,
    gridColumns: Int,
    scrollBehavior: TopAppBarScrollBehaviorHolder,
    onImportClick: () -> Unit,
    onOpenAppsFolder: () -> Unit,
    onLaunch: (InstalledApp) -> Unit,
    onDeleteRequest: (InstalledApp) -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(modifier = modifier.fillMaxSize()) {
        LazyVerticalGrid(
            columns = GridCells.Fixed(gridColumns),
            modifier = Modifier
                .fillMaxSize()
                .nestedScroll(scrollBehavior.behavior.nestedScrollConnection),
            contentPadding = PaddingValues(
                start = 20.dp,
                end = 20.dp,
                top = 8.dp,
                bottom = 140.dp,
            ),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (apps.isEmpty()) {
                item(span = { GridItemSpan(maxLineSpan) }) {
                    EmptyLibrary(
                        onImportClick = onImportClick,
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 48.dp),
                    )
                }
            } else {
                itemsIndexed(apps, key = { _, app -> app.file.absolutePath }) { index, app ->
                    AppCard(
                        app = app,
                        accentIndex = index,
                        onLaunch = onLaunch,
                        onDelete = onDeleteRequest,
                    )
                }
            }
        }

        // Expressive floating action menu (import / open apps folder).
        var menuExpanded by rememberSaveable { mutableStateOf(false) }
        FloatingActionButtonMenu(
            expanded = menuExpanded,
            button = {
                ToggleFloatingActionButton(
                    checked = menuExpanded,
                    onCheckedChange = { menuExpanded = it },
                ) {
                    val imageVector = if (checkedProgress > 0.5f) {
                        Icons.Outlined.Close
                    } else {
                        Icons.Outlined.Add
                    }
                    Icon(
                        imageVector = imageVector,
                        contentDescription = stringResource(
                            if (menuExpanded) {
                                R.string.library_fab_collapse_description
                            } else {
                                R.string.library_fab_expand_description
                            },
                        ),
                    )
                }
            },
            modifier = Modifier
                .align(Alignment.BottomEnd)
                .padding(24.dp),
        ) {
            FloatingActionButtonMenuItem(
                onClick = {
                    menuExpanded = false
                    onImportClick()
                },
                icon = {
                    Icon(
                        Icons.Outlined.Add,
                        contentDescription = null,
                    )
                },
                text = { Text(stringResource(R.string.library_import_action)) },
            )
            FloatingActionButtonMenuItem(
                onClick = {
                    menuExpanded = false
                    onOpenAppsFolder()
                },
                icon = {
                    Icon(
                        Icons.Outlined.FolderOpen,
                        contentDescription = null,
                    )
                },
                text = { Text(stringResource(R.string.library_open_folder_action)) },
            )
        }
    }
}

/** The expressive top app bar of the library, hoisted for the shell. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class, ExperimentalMaterial3Api::class)
@Composable
fun LibraryTopBar(
    appCount: Int,
    onOpenAppsFolder: () -> Unit,
    scrollBehavior: TopAppBarScrollBehaviorHolder,
) {
    LargeFlexibleTopAppBar(
        title = { Text(stringResource(R.string.app_name_display)) },
        subtitle = {
            Text(
                text = pluralStringResource(
                    R.plurals.library_subtitle_apps,
                    appCount,
                    appCount,
                ),
            )
        },
        actions = {
            // Expressive button group; items that do not fit collapse into
            // the overflow indicator automatically.
            ButtonGroup(
                overflowIndicator = { menuState ->
                    IconButton(
                        onClick = {
                            if (menuState.isShowing) menuState.dismiss() else menuState.show()
                        },
                    ) {
                        Icon(
                            Icons.Outlined.MoreVert,
                            contentDescription = stringResource(R.string.menu_more),
                        )
                    }
                },
            ) {
                clickableItem(
                    onClick = onOpenAppsFolder,
                    label = stringResource(R.string.library_open_folder_action),
                    icon = {
                        Icon(
                            Icons.Outlined.FolderOpen,
                            contentDescription = null,
                        )
                    },
                )
            }
        },
        scrollBehavior = scrollBehavior.behavior,
    )
}

/** Wrapper so one scroll behavior instance can be shared by bar and grid. */
class TopAppBarScrollBehaviorHolder internal constructor(
    val behavior: androidx.compose.material3.TopAppBarScrollBehavior,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun rememberLibraryScrollBehavior(): TopAppBarScrollBehaviorHolder =
    TopAppBarScrollBehaviorHolder(
        TopAppBarDefaults.exitUntilCollapsedScrollBehavior(),
    )

/** Empty state of the library, with the morphing expressive blob. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun EmptyLibrary(
    onImportClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier,
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        MorphingShapeBlob(
            diameter = 168.dp,
            startShape = HyperHLEAccentShapes.cookie(),
            endShape = HyperHLEAccentShapes.sunny(),
        ) {
            Icon(
                imageVector = Icons.Outlined.Ios,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onPrimaryContainer,
            )
        }
        Text(
            text = stringResource(R.string.library_empty_title),
            style = MaterialTheme.typography.headlineSmallEmphasized,
            textAlign = TextAlign.Center,
        )
        Text(
            text = stringResource(R.string.library_empty_message),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(horizontal = 24.dp),
        )
        Spacer(Modifier.height(8.dp))
        FilledTonalButton(onClick = onImportClick) {
            Icon(
                Icons.Outlined.Add,
                contentDescription = null,
            )
            Text(stringResource(R.string.library_import_action))
        }
    }
}
