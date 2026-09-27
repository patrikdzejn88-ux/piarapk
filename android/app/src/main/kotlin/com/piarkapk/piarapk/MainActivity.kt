package com.piarkapk.piarapk

import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        // Канал для системных путей (файловое хранилище приложения) —
        // вместо нативного плагина path_provider.
        MethodChannel(
            flutterEngine.dartExecutor.binaryMessenger,
            "piarapk/paths"
        ).setMethodCallHandler { call, result ->
            when (call.method) {
                "getFilesDir" -> result.success(filesDir.absolutePath)
                else -> result.notImplemented()
            }
        }
    }
}
