import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Нативные функции Android через MethodChannel (без плагинов).
class Native {
  static const _ch = MethodChannel('piarapk/paths');

  /// Канал `piarapk/paths` реализован только на Android: в windows/macos
  /// обработчика нет, поэтому на desktop действия, требующие канала, честно
  /// отключаются, а не падают с вечным «Не удалось открыть браузер».
  static bool get supported => !kIsWeb && Platform.isAndroid;

  /// Системный выбор картинки: путь к скопированному в файлы приложения
  /// файлу (для отправки через Rust), либо null (отмена). При ошибке
  /// канала/копирования бросает [PlatformException] с текстом ошибки —
  /// вызывающий может показать его пользователю.
  static Future<String?> pickImage() async {
    return await _ch.invokeMethod<String>('pickImage');
  }

  /// Прямой вызов метода канала без аргументов (навигация магазина и т.п.).
  static Future<void> rawChannelCall(String method) async {
    try {
      await _ch.invokeMethod(method);
    } catch (_) {}
  }

  /// Создать/прогреть WebView магазина при старте приложения.
  static Future<void> shopWarmup() async {
    try {
      await _ch.invokeMethod('shopWarmup');
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
