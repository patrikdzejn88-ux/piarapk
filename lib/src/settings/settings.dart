import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';

/// Настройки приложения в <data>/settings.json (атомарная запись tmp+rename).
/// На Android — во внутреннем приватном хранилище приложения (getFilesDir,
/// через MethodChannel), на desktop — в папке data/ рядом с .exe.
class SettingsStorage {
  SettingsStorage._();

  static String? _dir;
  static Map<String, dynamic> _cache = {};

  static const _channel = MethodChannel('piarapk/paths');

  static Future<String> _resolveDir() async {
    if (!Platform.isAndroid) {
      // desktop: каталог данных рядом с .exe — не зависит от CWD запуска
      final exeDir = File(Platform.resolvedExecutable).parent.path;
      return '$exeDir${Platform.pathSeparator}data';
    }
    try {
      final p = await _channel.invokeMethod<String>('getFilesDir');
      if (p != null && p.isNotEmpty) return p;
    } catch (_) {}
    return '/data/data/com.piarkapk.piarapk/files';
  }

  static Future<String> _dirPath() async => _dir ??= await _resolveDir();

  static Future<void>? _loadFuture;

  /// Кэшируется сам Future: конкурентные вызовы ждут одну и ту же загрузку,
  /// а не читают пустой кэш до её завершения.
  static Future<void> _load() => _loadFuture ??= _doLoad();

  static Future<void> _doLoad() async {
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

  /// ---- Telegram API (api_id/api_hash с my.telegram.org) ----

  static Future<int?> getApiId() async {
    await _load();
    final v = _cache['telegram_api_id'];
    if (v is int) return v;
    return int.tryParse('${v ?? ''}');
  }

  static Future<String?> getApiHash() async {
    await _load();
    final v = _cache['telegram_api_hash'];
    return v?.toString();
  }

  static Future<void> setApiPair(int? id, String? hash) async {
    await _load();
    if (id == null) {
      _cache.remove('telegram_api_id');
    } else {
      _cache['telegram_api_id'] = id;
    }
    if (hash == null || hash.isEmpty) {
      _cache.remove('telegram_api_hash');
    } else {
      _cache['telegram_api_hash'] = hash;
    }
    await _flush();
  }
}
