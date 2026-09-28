import 'dart:io';

import 'package:flutter/widgets.dart';

import 'package:piarapk/src/settings/settings.dart';
import 'src/app.dart';
import 'src/core/bridge.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();

  // Ошибки релиз-сборки больше не молчат: всё пишется в журнал приложения
  // (виден через «Лог ядра» в разделе Аккаунты).
  FlutterError.onError = (details) {
    PiarCore.instance.noteError('виджет: ${details.exception}');
    FlutterError.presentError(details);
  };

  // Каталог данных приложения создаётся мостом при init(); принудительно
  // готовим рабочую папку до запуска UI.
  try {
    Directory('data').createSync(recursive: true);
  } catch (_) {}

  // Своя пара api_id/api_hash (my.telegram.org) — если задана в настройках,
  // передаём в ядро; иначе ядро использует публичную (может отклоняться).
  final apiId = await SettingsStorage.getApiId();
  final apiHash = await SettingsStorage.getApiHash();
  PiarCore.instance.init(apiId: apiId, apiHash: apiHash);
  runApp(const PiarApp());
}
