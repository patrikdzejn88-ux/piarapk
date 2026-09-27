import 'dart:convert';
import 'dart:io';

/// Настройки приложения в data/settings.json (атомарная запись tmp+rename).
class SettingsStorage {
  SettingsStorage._();

  static const _file = 'data/settings.json';

  static Map<String, dynamic> _cache = {};
  static bool _loaded = false;

  static Future<void> _load() async {
    if (_loaded) return;
    _loaded = true;
    try {
      final f = File(_file);
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
    final dir = Directory('data');
    if (!dir.existsSync()) {
      dir.createSync(recursive: true);
    }
    final tmp = File('$_file.tmp');
    tmp.writeAsStringSync(const JsonEncoder.withIndent('  ').convert(_cache));
    tmp.renameSync(_file);
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
