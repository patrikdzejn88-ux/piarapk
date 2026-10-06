package com.piarkapk.piarapk

import android.content.ContentValues
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.util.Log
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import io.flutter.embedding.android.FlutterFragmentActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import androidx.activity.result.contract.ActivityResultContracts
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/**
 * FlutterFragmentActivity (НЕ FlutterActivity!): только он наследует
 * androidx ComponentActivity, где доступен registerForActivityResult
 * для системного выбора картинки.
 */
class MainActivity : FlutterFragmentActivity() {
    companion object {
        /**
         * Предзагрузка Rust-ядра (libpiarcore.so) до старта Flutter:
         * гарантирует, что библиотека из APK распакована и доступна,
         * а dart:ffi находит её по имени (уже загружена в процесс).
         */
        private var coreLibError: String? = null

        const val REQ_WRITE_EXPORT = 4712

        const val PREFS_NAME = "piarapk"
        const val KEY_NOTIF_ASKED = "notif_permission_asked"

        init {
            try {
                System.loadLibrary("piarcore")
            } catch (e: UnsatisfiedLinkError) {
                coreLibError = e.message ?: e.toString()
                Log.e("piarapk", "libpiarcore.so НЕ загрузилась", e)
            }
        }
    }

    /** Ожидаемый результат системного выбора картинки (одиночный). */
    private var pendingPickResult: MethodChannel.Result? = null

    /** Отложенный экспорт после запроса разрешения на запись (API < 29). */
    private var pendingExportAction: ((Boolean) -> Unit)? = null

    private val pickImageLauncher =
        registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
            val result = pendingPickResult
            if (result == null) return@registerForActivityResult
            if (uri == null) {
                pendingPickResult = null
                result.success(null)
                return@registerForActivityResult
            }
            // копирование в фоне (не блокируем main thread / ANR); корутина
            // привязана к lifecycleScope и отменяется вместе с Activity
            lifecycleScope.launch {
                val path: String? = withContext(Dispatchers.IO) {
                    try {
                        // копируем выбранную картинку в файлы приложения (стабильный
                        // путь для Rust-загрузки)
                        val ext = when (contentResolver.getType(uri)) {
                            "image/png" -> "png"
                            "image/webp" -> "webp"
                            else -> "jpg"
                        }
                        // чистим старые копии, чтобы не копить мусор
                        filesDir.listFiles { f -> f.name.startsWith("picked_image.") }
                            ?.forEach { it.delete() }
                        val out = File(filesDir, "picked_image.$ext")
                        contentResolver.openInputStream(uri)?.use { input ->
                            out.outputStream().use { output -> input.copyTo(output) }
                        }
                        out.absolutePath
                    } catch (e: Exception) {
                        Log.e("piarapk", "pickImage copy failed", e)
                        null
                    }
                }
                // отвечаем только если результат ещё не был зафейлен в onDestroy
                if (pendingPickResult === result) {
                    pendingPickResult = null
                    result.success(path)
                }
            }
        }

    override fun onCreate(savedInstanceState: android.os.Bundle?) {
        super.onCreate(savedInstanceState)
        // магазин грузится сразу при запуске, ещё до открытия вкладки
        ShopWebViewHolder.ensure(this)
    }

    override fun onPause() {
        ShopWebViewHolder.persistCookies()
        super.onPause()
    }

    override fun onStop() {
        ShopWebViewHolder.persistCookies()
        super.onStop()
    }

    override fun onDestroy() {
        // Не оставляем Dart-Future pickImage() висеть, если Activity умирает
        // до завершения выбора/копирования картинки.
        pendingPickResult?.error(
            "PICK_CANCELLED",
            "Activity уничтожена до завершения выбора картинки",
            null
        )
        pendingPickResult = null
        super.onDestroy()
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        // Канал системных путей — вместо нативного плагина path_provider.
        val channel = MethodChannel(
            flutterEngine.dartExecutor.binaryMessenger,
            "piarapk/paths"
        )
        // WebView магазина как PlatformView (вкладка «Магазин» в окне приложения);
        // в 3.47.x реестр фабрик доступен через platformViewsController.registry
        flutterEngine.platformViewsController.registry.registerViewFactory(
            "shop-webview",
            ShopWebViewFactory { url, canBack, canFwd ->
                channel.invokeMethod(
                    "shopUrl",
                    mapOf(
                        "url" to url,
                        "canGoBack" to canBack,
                        "canGoForward" to canFwd
                    )
                )
            }
        )
        channel.setMethodCallHandler { call, result ->
            when (call.method) {
                "getFilesDir" -> result.success(filesDir.absolutePath)
                "getNativeLibraryDir" -> result.success(applicationInfo.nativeLibraryDir)
                "getCoreLibStatus" -> result.success(coreLibError ?: "ok")
                "shopWarmup" -> {
                    ShopWebViewHolder.ensure(this)
                    result.success(true)
                }
                "shopLoad" -> {
                    ShopWebViewHolder.webView?.loadUrl(
                        call.argument<String>("url") ?: ShopWebViewHolder.SHOP_URL
                    )
                    result.success(true)
                }
                "shopBack" -> {
                    ShopWebViewHolder.webView?.goBack()
                    result.success(true)
                }
                "shopForward" -> {
                    ShopWebViewHolder.webView?.goForward()
                    result.success(true)
                }
                "shopReload" -> {
                    ShopWebViewHolder.webView?.reload()
                    result.success(true)
                }
                "shopCanGoBack" -> result.success(ShopWebViewHolder.webView?.canGoBack() ?: false)
                "shopCanGoForward" -> result.success(ShopWebViewHolder.webView?.canGoForward() ?: false)
                "pickImage" -> {
                    if (pendingPickResult != null) {
                        result.error("PICK_BUSY", "выбор уже идёт", null)
                    } else {
                        pendingPickResult = result
                        pickImageLauncher.launch(arrayOf("image/*"))
                    }
                }
                "parser_service_start" -> {
                    try {
                        // API 33+: просим показ уведомления (сервис работает и без
                        // него) только если оно ещё не выдано и мы ещё не спрашивали.
                        if (Build.VERSION.SDK_INT >= 33 &&
                            ContextCompat.checkSelfPermission(
                                this,
                                android.Manifest.permission.POST_NOTIFICATIONS
                            ) != android.content.pm.PackageManager.PERMISSION_GRANTED &&
                            !wasNotificationPermissionRequested()
                        ) {
                            markNotificationPermissionRequested()
                            ActivityCompat.requestPermissions(
                                this,
                                arrayOf(android.Manifest.permission.POST_NOTIFICATIONS),
                                4711
                            )
                        }
                        val intent = Intent(this, ParserService::class.java)
                            .putExtra(
                                "text",
                                call.argument<String>("text") ?: "Сбор базы участников…"
                            )
                        ContextCompat.startForegroundService(this, intent)
                        result.success(true)
                    } catch (e: Exception) {
                        Log.e("piarapk", "parser_service_start failed", e)
                        result.error("FGS_FAIL", e.message, null)
                    }
                }
                "parser_service_stop" -> {
                    try {
                        stopService(Intent(this, ParserService::class.java))
                        result.success(true)
                    } catch (e: Exception) {
                        result.error("FGS_FAIL", e.message, null)
                    }
                }
                "openShop" -> {
                    try {
                        val url = call.argument<String>("url") ?: "https://dark.shopping/"
                        startActivity(
                            Intent(this, WebViewActivity::class.java)
                                .putExtra(WebViewActivity.EXTRA_URL, url)
                        )
                        result.success(true)
                    } catch (e: Exception) {
                        Log.e("piarapk", "openShop failed", e)
                        result.error("OPEN_FAIL", e.message, null)
                    }
                }
                "exportToDownloads" -> handleExport(call, result)
                else -> result.notImplemented()
            }
        }
    }

    /**
     * Экспорт текстового файла (база usernames) в «Загрузки» устройства.
     * На API 29+ — через MediaStore; на старых — в публичный Downloads напрямую.
     * Возвращает строку-описание, куда сохранилось (или ошибку).
     */
    private fun handleExport(call: io.flutter.plugin.common.MethodCall, result: MethodChannel.Result) {
        val name = call.argument<String>("name") ?: "db.txt"
        val content = call.argument<String>("content") ?: ""
        try {
            val safe = name.replace(Regex("[^A-Za-z0-9._-]"), "_")
            if (Build.VERSION.SDK_INT >= 29) {
                val values = ContentValues().apply {
                    put(MediaStore.Downloads.DISPLAY_NAME, safe)
                    put(MediaStore.Downloads.MIME_TYPE, "text/plain")
                }
                val uri = contentResolver.insert(
                    MediaStore.Downloads.EXTERNAL_CONTENT_URI, values
                ) ?: return result.error("EXPORT_FAIL", "MediaStore вернул null", null)
                contentResolver.openOutputStream(uri)?.use { it.write(content.toByteArray()) }
                result.success("Загрузки/$safe")
            } else {
                if (ContextCompat.checkSelfPermission(
                        this,
                        android.Manifest.permission.WRITE_EXTERNAL_STORAGE
                    ) != android.content.pm.PackageManager.PERMISSION_GRANTED
                ) {
                    pendingExportAction = { granted ->
                        if (granted) {
                            writeLegacyExport(safe, content, result)
                        } else {
                            result.error("EXPORT_FAIL", "нет разрешения на запись в «Загрузки»", null)
                        }
                    }
                    ActivityCompat.requestPermissions(
                        this,
                        arrayOf(android.Manifest.permission.WRITE_EXTERNAL_STORAGE),
                        REQ_WRITE_EXPORT
                    )
                } else {
                    writeLegacyExport(safe, content, result)
                }
            }
        } catch (e: Exception) {
            result.error("EXPORT_FAIL", e.message, null)
        }
    }

    @Suppress("DEPRECATION")
    private fun writeLegacyExport(safe: String, content: String, result: MethodChannel.Result) {
        try {
            val file = File(
                android.os.Environment.getExternalStoragePublicDirectory(
                    android.os.Environment.DIRECTORY_DOWNLOADS
                ), safe
            )
            file.writeText(content)
            result.success("Download/$safe")
        } catch (e: Exception) {
            result.error("EXPORT_FAIL", e.message, null)
        }
    }

    private fun wasNotificationPermissionRequested(): Boolean =
        getSharedPreferences(PREFS_NAME, MODE_PRIVATE)
            .getBoolean(KEY_NOTIF_ASKED, false)

    private fun markNotificationPermissionRequested() {
        getSharedPreferences(PREFS_NAME, MODE_PRIVATE)
            .edit().putBoolean(KEY_NOTIF_ASKED, true).apply()
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQ_WRITE_EXPORT) {
            val action = pendingExportAction
            pendingExportAction = null
            val granted = grantResults.isNotEmpty() &&
                grantResults[0] == android.content.pm.PackageManager.PERMISSION_GRANTED
            action?.invoke(granted)
        }
    }
}
