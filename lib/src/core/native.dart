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

  /// Прямой вызов метода канала без аргументов (навигация магазина и т.п.).
  static Future<void> rawChannelCall(String method) async {
    try {
      await _ch.invokeMethod(method);
    } catch (_) {}
  }

  /// Открыть встроенный браузер (магазин dark.shopping).
  static Future<bool> openShop(String url) async {
    try {
      return await _ch.invokeMethod('openShop', {'url': url}) == true;
    } catch (_) {
      return false;
    }
  }

  /// Запустить foreground-сервис (парсер живёт при сворачивании приложения).
  static Future<bool> parserServiceStart(String text) async {
    try {
      return await _ch.invokeMethod('parser_service_start', {'text': text}) ==
          true;
    } catch (_) {
      return false;
    }
  }

  /// Остановить foreground-сервис.
  static Future<bool> parserServiceStop() async {
    try {
      return await _ch.invokeMethod('parser_service_stop') == true;
    } catch (_) {
      return false;
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
