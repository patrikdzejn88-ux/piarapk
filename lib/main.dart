import 'dart:async';

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

  // Своя пара api_id/api_hash (my.telegram.org) обязательна: без неё ядро
  // не инициализируется (код -3), дефолтной пары нет.
  final apiId = await SettingsStorage.getApiId();
  final apiHash = await SettingsStorage.getApiHash();
  unawaited(PiarCore.instance.init(apiId: apiId, apiHash: apiHash));
  runApp(const PiarApp());
}
