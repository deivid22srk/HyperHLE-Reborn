/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedContentTransitionScope
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.windowsizeclass.ExperimentalMaterial3WindowSizeClassApi
import androidx.compose.material3.windowsizeclass.WindowWidthSizeClass
import androidx.compose.material3.windowsizeclass.calculateWindowSizeClass
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import org.touchhle.android.ui.HyperHLEScaffold
import org.touchhle.android.ui.LauncherDestination
import org.touchhle.android.ui.components.DeleteAppDialog
import org.touchhle.android.ui.components.ErrorDialog
import org.touchhle.android.ui.components.ImportingDialog
import org.touchhle.android.ui.library.LibraryScreen
import org.touchhle.android.ui.library.LibraryTopBar
import org.touchhle.android.ui.library.rememberLibraryScrollBehavior
import org.touchhle.android.ui.settings.SettingsScreen
import org.touchhle.android.ui.settings.SettingsTopBar
import org.touchhle.android.ui.theme.HyperHLETheme

/**
 * The Material 3 Expressive entry point of HyperHLE-Reborn.
 *
 * Replaces the old SDL-drawn first screen: the user now manages their .ipa
 * library and the emulator options in a Compose UI, and launches games into
 * the SDL emulation host ([MainActivity]) from here. The emulation side is
 * unchanged — [MainActivity] still talks to the Rust core over SDL/JNI and
 * receives the selected .ipa path plus the configured options through
 * SDLActivity#getArguments().
 */
class LauncherActivity : ComponentActivity() {

    private val viewModel: AppLibraryViewModel by viewModels()

    /**
     * .ipa import: the same ACTION_GET_CONTENT chooser the old app used,
     * including the extra MIME types so .ipa files open with any document
     * provider.
     */
    private val importIpaLauncher =
        registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
            if (result.resultCode != Activity.RESULT_OK) return@registerForActivityResult
            val data = result.data ?: return@registerForActivityResult
            val uri = data.data ?: return@registerForActivityResult
            viewModel.importFromUri(uri)
        }

    /**
     * "Open apps folder": the Storage Access Framework tree picker, starting
     * in the touchHLE apps directory exposed by the DocumentsProvider — the
     * same behaviour as the old File manager button.
     */
    private val openAppsFolderLauncher =
        registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { _ ->
            // Opening the folder in a file manager is the whole interaction;
            // nothing to do with the result.
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent {
            HyperHLETheme {
                LauncherApp(
                    viewModel = viewModel,
                    onPickIpa = { launchIpaPicker() },
                    onOpenAppsFolder = { launchAppsFolderPicker() },
                )
            }
        }
    }

    private fun launchIpaPicker() {
        val intent = Intent(Intent.ACTION_GET_CONTENT)
        intent.addCategory(Intent.CATEGORY_OPENABLE)
        intent.type = "*/*"
        intent.putExtra(
            Intent.EXTRA_MIME_TYPES,
            arrayOf(
                "application/octet-stream",
                "application/zip",
                "application/x-zip-compressed",
            ),
        )
        try {
            importIpaLauncher.launch(Intent.createChooser(intent, getString(R.string.library_import_chooser)))
        } catch (e: Exception) {
            // No document provider available; surface it as an import error.
            viewModel.reportImportFailure(e.message ?: e.javaClass.simpleName)
        }
    }

    private fun launchAppsFolderPicker() {
        try {
            // Mirrors the content URI the Rust core builds for the apps
            // directory (see android_document_uri() in src/paths.rs): the
            // DocumentsProvider authority of this application plus the
            // hierarchical document id "root/touchHLE_apps".
            val appsUri = Uri.parse(
                "content://${packageName}.provider/document/root%2FtouchHLE_apps",
            )
            openAppsFolderLauncher.launch(appsUri)
        } catch (e: Exception) {
            viewModel.reportImportFailure(e.message ?: e.javaClass.simpleName)
        }
    }
}

@OptIn(ExperimentalMaterial3WindowSizeClassApi::class)
@Composable
private fun LauncherApp(
    viewModel: AppLibraryViewModel,
    onPickIpa: () -> Unit,
    onOpenAppsFolder: () -> Unit,
) {
    val activity = LocalContext.current as Activity
    val windowSizeClass = calculateWindowSizeClass(activity)
    val widthSizeClass = windowSizeClass.widthSizeClass

    var destination by rememberSaveable { mutableStateOf(LauncherDestination.LIBRARY) }
    val apps by viewModel.apps.collectAsState()
    val options by viewModel.options.collectAsState()
    val importState by viewModel.importState.collectAsState()

    val snackbarHostState = remember { SnackbarHostState() }
    val libraryScrollBehavior = rememberLibraryScrollBehavior()

    var deleteCandidate by remember { mutableStateOf<InstalledApp?>(null) }
    var showErrorDialog by remember { mutableStateOf(false) }

    // Import feedback: success goes to a snackbar, failures open a dialog.
    val importedMessage = stringResource(R.string.snackbar_import_done)
    LaunchedEffect(importState) {
        when (val state = importState) {
            is ImportState.Done -> {
                snackbarHostState.showSnackbar(importedMessage.format(state.displayName))
                viewModel.acknowledgeImportState()
            }
            is ImportState.Failed -> showErrorDialog = true
            else -> Unit
        }
    }

    val gridColumns = when (widthSizeClass) {
        WindowWidthSizeClass.Compact -> 1
        WindowWidthSizeClass.Medium -> 2
        else -> 3
    }

    HyperHLEScaffold(
        windowWidthSizeClass = widthSizeClass,
        selectedDestination = destination,
        onDestinationSelected = { destination = it },
        snackbarHostState = snackbarHostState,
        topBar = {
            AnimatedContent(
                targetState = destination,
                transitionSpec = { expressiveScreenTransition() },
                label = "topBarSwitch",
            ) { current ->
                when (current) {
                    LauncherDestination.LIBRARY -> LibraryTopBar(
                        appCount = apps.size,
                        onOpenAppsFolder = onOpenAppsFolder,
                        scrollBehavior = libraryScrollBehavior,
                    )
                    LauncherDestination.SETTINGS -> SettingsTopBar()
                }
            }
        },
    ) {
        AnimatedContent(
            targetState = destination,
            transitionSpec = { expressiveScreenTransition() },
            label = "screenSwitch",
        ) { current ->
            when (current) {
                LauncherDestination.LIBRARY -> LibraryScreen(
                    apps = apps,
                    gridColumns = gridColumns,
                    scrollBehavior = libraryScrollBehavior,
                    onImportClick = onPickIpa,
                    onOpenAppsFolder = onOpenAppsFolder,
                    onLaunch = viewModel::launchApp,
                    onDeleteRequest = { deleteCandidate = it },
                )
                LauncherDestination.SETTINGS -> SettingsScreen(
                    options = options,
                    onOptionsChange = viewModel::updateOptions,
                )
            }
        }
    }

    deleteCandidate?.let { candidate ->
        DeleteAppDialog(
            app = candidate,
            onConfirm = {
                viewModel.deleteApp(candidate)
                deleteCandidate = null
            },
            onDismiss = { deleteCandidate = null },
        )
    }

    if (importState is ImportState.Importing) {
        val importing = importState as ImportState.Importing
        ImportingDialog(
            displayName = importing.displayName,
            bytesCopied = importing.bytesCopied,
            totalBytes = importing.totalBytes,
        )
    }

    if (showErrorDialog && importState is ImportState.Failed) {
        val failure = importState as ImportState.Failed
        ErrorDialog(
            title = stringResource(R.string.dialog_import_failed_title),
            message = failure.message,
            onDismiss = {
                showErrorDialog = false
                viewModel.acknowledgeImportState()
            },
        )
    }
}

/**
 * Spring-driven expressive transition used when switching between the two
 * destinations: quick fade out, springy scale+fade in.
 */
private fun AnimatedContentTransitionScope<LauncherDestination>.expressiveScreenTransition() =
    (fadeIn(animationSpec = tween(durationMillis = 180, delayMillis = 60)) +
        scaleIn(
            initialScale = 0.94f,
            animationSpec = spring(
                dampingRatio = Spring.DampingRatioLowBouncy,
                stiffness = Spring.StiffnessMediumLow,
            ),
        )) togetherWith (
        fadeOut(animationSpec = tween(durationMillis = 110)) +
            scaleOut(targetScale = 0.97f, animationSpec = tween(durationMillis = 110))
        )
