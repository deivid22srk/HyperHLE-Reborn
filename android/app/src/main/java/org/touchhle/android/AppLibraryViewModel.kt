/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android

import android.app.Application
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Log
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** One imported .ipa shown in the library grid. */
data class InstalledApp(
    val displayName: String,
    val file: File,
    val sizeBytes: Long,
    val lastModified: Long,
)

/** Progress of an .ipa import from the system document picker. */
sealed interface ImportState {
    data object Idle : ImportState
    data class Importing(val displayName: String, val bytesCopied: Long, val totalBytes: Long) : ImportState
    data class Done(val displayName: String) : ImportState
    data class Failed(val message: String) : ImportState
}

/**
 * Holds the state of the launcher UI: the list of imported .ipa files, the
 * import progress and the emulator options.
 *
 * The apps directory is the same one the Rust core reads
 * (`SDL_AndroidGetExternalStoragePath()/touchHLE_apps`, see APPS_DIR in
 * src/paths.rs), so anything the user imports here is picked up by the
 * emulator exactly as if it had been added through the old in-emulator
 * picker.
 */
class AppLibraryViewModel(application: Application) : AndroidViewModel(application) {

    private val optionsStore = OptionsStore(application)

    private val _apps = MutableStateFlow<List<InstalledApp>>(emptyList())
    val apps: StateFlow<List<InstalledApp>> = _apps.asStateFlow()

    private val _importState = MutableStateFlow<ImportState>(ImportState.Idle)
    val importState: StateFlow<ImportState> = _importState.asStateFlow()

    private val _options = MutableStateFlow(optionsStore.load())
    val options: StateFlow<EmulatorOptions> = _options.asStateFlow()

    /** Directory where the emulator core looks for .ipa files. */
    val appsDir: File
        get() = File(getApplication<Application>().getExternalFilesDir(null), APPS_DIR_NAME)

    init {
        refreshApps()
    }

    /** Rescan the apps directory. */
    fun refreshApps() {
        viewModelScope.launch(Dispatchers.IO) {
            val dir = appsDir
            if (!dir.exists() && !dir.mkdirs()) {
                Log.e(TAG, "Couldn't create ${dir.absolutePath}")
            }
            val list = dir.listFiles { f -> f.isFile && f.name.endsWith(".ipa") }
                ?.map { f ->
                    InstalledApp(
                        displayName = f.name,
                        file = f,
                        sizeBytes = f.length(),
                        lastModified = f.lastModified(),
                    )
                }
                ?.sortedBy { it.displayName.lowercase() }
                ?: emptyList()
            _apps.value = list
        }
    }

    /**
     * Copy the picked document into the apps directory as an .ipa, streaming
     * the bytes so multi-gigabyte games never sit fully in memory.
     */
    fun importFromUri(uri: Uri) {
        if (_importState.value is ImportState.Importing) return
        viewModelScope.launch(Dispatchers.IO) {
            val context = getApplication<Application>()
            val name = queryDisplayName(context, uri) ?: DEFAULT_IPA_NAME
            val target = uniqueTargetFile(appsDir, ensureIpaExtension(name))
            var copied = 0L
            val total = querySize(context, uri)
            _importState.value = ImportState.Importing(target.name, 0L, total)
            try {
                context.contentResolver.openInputStream(uri)?.use { input ->
                    FileOutputStream(target).use { output ->
                        val buffer = ByteArray(COPY_BUFFER_BYTES)
                        var read = input.read(buffer)
                        while (read >= 0) {
                            output.write(buffer, 0, read)
                            copied += read
                            _importState.value =
                                ImportState.Importing(target.name, copied, total)
                            read = input.read(buffer)
                        }
                        output.flush()
                    }
                } ?: run {
                    _importState.value =
                        ImportState.Failed("Couldn't open $name")
                    return@launch
                }
                Log.i(TAG, "Added game to ${appsDir.absolutePath}: ${target.name}")
                _importState.value = ImportState.Done(target.name)
                refreshApps()
            } catch (e: IOException) {
                Log.e(TAG, "Couldn't copy .ipa file: ${target.name}", e)
                target.delete()
                _importState.value =
                    ImportState.Failed(e.message ?: e.javaClass.simpleName)
            } catch (e: SecurityException) {
                Log.e(TAG, "Couldn't copy .ipa file: ${target.name}", e)
                _importState.value =
                    ImportState.Failed(e.message ?: e.javaClass.simpleName)
            }
        }
    }

    /** Reset a terminal import state back to idle (dialog dismissal). */
    fun acknowledgeImportState() {
        when (_importState.value) {
            is ImportState.Done, is ImportState.Failed -> _importState.value = ImportState.Idle
            else -> Unit
        }
    }

    /** Surface a failure that happened outside the copy loop (e.g. no file picker). */
    fun reportImportFailure(message: String) {
        _importState.value = ImportState.Failed(message)
    }

    /** Delete an imported .ipa from the apps directory. */
    fun deleteApp(app: InstalledApp) {
        viewModelScope.launch(Dispatchers.IO) {
            if (!app.file.delete()) {
                Log.w(TAG, "Couldn't delete ${app.file.absolutePath}")
            }
            refreshApps()
        }
    }

    /** Update the emulator options and persist them. */
    fun updateOptions(transform: (EmulatorOptions) -> EmulatorOptions) {
        val next = transform(_options.value)
        _options.value = next
        optionsStore.save(next)
    }

    /**
     * Start the SDL emulation host for [app]. The emulator options ride
     * along as command-line style extras that MainActivity feeds to the
     * Rust core via SDLActivity#getArguments().
     */
    fun launchApp(app: InstalledApp) {
        val context = getApplication<Application>()
        val arguments = _options.value.toArguments().joinToString("\n")
        val intent = Intent(context, MainActivity::class.java).apply {
            action = Intent.ACTION_MAIN
            addCategory(Intent.CATEGORY_LAUNCHER)
            putExtra(LauncherContract.EXTRA_APP_PATH, app.file.absolutePath)
            if (arguments.isNotEmpty()) {
                putExtra(LauncherContract.EXTRA_OPTIONS, arguments)
            }
            // The SDL activity is declared singleInstance, which lives in its
            // own task; NEW_TASK keeps the launch well-formed from a non-
            // activity context.
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        context.startActivity(intent)
    }

    private fun queryDisplayName(context: Application, uri: Uri): String? {
        var name: String? = null
        context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
            val idx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (idx >= 0 && cursor.moveToFirst() && !cursor.isNull(idx)) {
                name = cursor.getString(idx)
            }
        }
        if (name == null) {
            uri.lastPathSegment?.let { last ->
                name = last.substring(last.lastIndexOf('/') + 1)
            }
        }
        return name
    }

    private fun querySize(context: Application, uri: Uri): Long {
        context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
            val idx = cursor.getColumnIndex(OpenableColumns.SIZE)
            if (idx >= 0 && cursor.moveToFirst() && !cursor.isNull(idx)) {
                return cursor.getLong(idx)
            }
        }
        return 0L
    }

    private fun ensureIpaExtension(name: String): String =
        if (name.lowercase().endsWith(".ipa")) name else "$name.ipa"

    /** Never overwrite an existing game; mirror the core's " (1)" style. */
    private fun uniqueTargetFile(dir: File, name: String): File {
        var candidate = File(dir, name)
        if (!candidate.exists()) return candidate
        val dot = name.lastIndexOf('.')
        val base = if (dot > 0) name.substring(0, dot) else name
        val ext = if (dot > 0) name.substring(dot) else ""
        var i = 1
        while (candidate.exists()) {
            candidate = File(dir, "$base ($i)$ext")
            i++
        }
        return candidate
    }

    companion object {
        private const val TAG = "touchHLE"
        // Must match APPS_DIR in src/paths.rs.
        const val APPS_DIR_NAME = "touchHLE_apps"
        private const val DEFAULT_IPA_NAME = "game.ipa"
        private const val COPY_BUFFER_BYTES = 65536
    }
}
