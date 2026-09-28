import 'package:flutter/services.dart';

/// Нативные функции Android через MethodChannel (без плагинов).
class Native {
  static const _ch = MethodChannel('piarapk/paths');

  /// Экспорт текстового файла в «Загрузки» устройства.
  /// Возвращает описание места, куда сохранилось, или null при ошибке.
  static Future<String?> exportToDownloads(String name, String content) async {
    try {
      return await _ch.invokeMethod<String>(
        'exportToDownloads',
        {'name': name, 'content': content},
      );
    } catch (_) {
      return null;
    }
  }
}
