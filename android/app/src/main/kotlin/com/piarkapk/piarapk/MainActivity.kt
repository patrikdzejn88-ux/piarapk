package com.piarkapk.piarapk

import android.util.Log
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
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

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        // Канал системных путей — вместо нативного плагина path_provider.
        MethodChannel(
            flutterEngine.dartExecutor.binaryMessenger,
            "piarapk/paths"
        ).setMethodCallHandler { call, result ->
            when (call.method) {
                "getFilesDir" -> result.success(filesDir.absolutePath)
                "getNativeLibraryDir" -> result.success(applicationInfo.nativeLibraryDir)
                "getCoreLibStatus" -> result.success(coreLibError ?: "ok")
                else -> result.notImplemented()
            }
        }
    }
}
