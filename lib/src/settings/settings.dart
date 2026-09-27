import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';

/// Настройки приложения в <data>/settings.json (атомарная запись tmp+rename).
/// На Android — во внутреннем хранилище приложения (через MethodChannel),
/// на desktop — в рабочей папке data/.
class SettingsStorage {
  SettingsStorage._();

  static String? _dir;
  static Map<String, dynamic> _cache = {};
  static bool _loaded = false;

  static const _channel = MethodChannel('piarapk/paths');

  static Future<String> _resolveDir() async {
    if (!Platform.isAndroid) return 'data';
    try {
      final p = await _channel.invokeMethod<String>('getFilesDir');
      if (p != null && p.isNotEmpty) return p;
    } catch (_) {}
    return '/data/data/com.piarkapk.piarapk/files';
  }

  static Future<String> _dirPath() async => _dir ??= await _resolveDir();

  static Future<void> _load() async {
    if (_loaded) return;
    _loaded = true;
    try {
      final base = await _dirPath();
      final f = File('$base/settings.json');
      if (f.existsSync()) {
        final parsed = jsonDecode(f.readAsStringSync());
        if (parsed is Map<String, dynamic>) {
          _cache = parsed;
        }
      }
    } catch (_) {
      _cache = {};
    }
  }

  static Future<void> _flush() async {
    final base = await _dirPath();
    final dir = Directory(base);
    if (!dir.existsSync()) {
      dir.createSync(recursive: true);
    }
    final tmp = File('$base/settings.json.tmp');
    tmp.writeAsStringSync(const JsonEncoder.withIndent('  ').convert(_cache));
    tmp.renameSync('$base/settings.json');
  }

  static Future<String?> getApiKey() async {
    await _load();
    return _cache['dark_shopping_api_key'] as String?;
  }

  static Future<void> setApiKey(String? key) async {
    await _load();
    if (key == null || key.isEmpty) {
      _cache.remove('dark_shopping_api_key');
    } else {
      _cache['dark_shopping_api_key'] = key;
    }
    await _flush();
  }
}
