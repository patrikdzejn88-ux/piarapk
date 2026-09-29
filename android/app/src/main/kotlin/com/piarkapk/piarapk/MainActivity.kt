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

    private val pickImageLauncher =
        registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
            val result = pendingPickResult
            pendingPickResult = null
            if (result == null) return@registerForActivityResult
            if (uri == null) {
                result.success(null)
                return@registerForActivityResult
            }
            // копирование в фоне (не блокируем main thread / ANR)
            Thread {
                val path: String? = try {
                    // копируем выбранную картинку в файлы приложения (стабильный
                    // путь для Rust-загрузки)
                    val ext = when (contentResolver.getType(uri)) {
                        "image/png" -> "png"
                        "image/webp" -> "webp"
                        else -> "jpg"
                    }
                    val out = File(filesDir, "picked_image.$ext")
                    contentResolver.openInputStream(uri)?.use { input ->
                        out.outputStream().use { output -> input.copyTo(output) }
                    }
                    out.absolutePath
                } catch (e: Exception) {
                    Log.e("piarapk", "pickImage copy failed", e)
                    null
                }
                runOnUiThread { result.success(path) }
            }.start()
        }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        // Канал системных путей — вместо нативного плагина path_provider.
        val channel = MethodChannel(
            flutterEngine.dartExecutor.binaryMessenger,
            "piarapk/paths"
        )
        // WebView магазина как PlatformView (вкладка «Магазин» в окне приложения)
        flutterEngine.platformViewsController.registerViewFactory(
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
                "shopLoad" -> {
                    ShopWebViewHolder.webView?.loadUrl(
                        call.argument<String>("url") ?: "https://dark.shopping/"
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
                        // API 33+: просим показ уведомления (сервис работает и без него)
                        if (Build.VERSION.SDK_INT >= 33) {
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
                @Suppress("DEPRECATION")
                val dir = File(
                    android.os.Environment.getExternalStoragePublicDirectory(
                        android.os.Environment.DIRECTORY_DOWNLOADS
                    ), safe
                )
                dir.writeText(content)
                result.success("Download/$safe")
            }
        } catch (e: Exception) {
            result.error("EXPORT_FAIL", e.message, null)
        }
    }
}
