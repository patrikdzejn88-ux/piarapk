import 'package:flutter/services.dart';

/// Нативные функции Android через MethodChannel (без плагинов).
class Native {
  static const _ch = MethodChannel('piarapk/paths');

  /// Системный выбор картинки: возвращает путь к скопированному в файлы
  /// приложения файлу (для отправки через Rust), либо null (отмена/ошибка).
  static Future<String?> pickImage() async {
    try {
      return await _ch.invokeMethod<String>('pickImage');
    } catch (_) {
      return null;
    }
  }

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
