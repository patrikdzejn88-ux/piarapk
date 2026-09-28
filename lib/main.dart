import 'dart:io';

import 'package:flutter/widgets.dart';

import 'src/app.dart';
import 'src/core/bridge.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();

  // Ошибки релиз-сборки больше не молчат: всё пишется в журнал приложения
  // (виден через «Лог ядра» в разделе Аккаунты).
  FlutterError.onError = (details) {
    PiarCore.instance.noteError('виджет: ${details.exception}');
    FlutterError.presentError(details);
  };

  // Каталог данных приложения создаётся мостом при init(); принудительно
  // готовим рабочую папку до запуска UI, чтобы файловые операции UI
  // (настройки магазина и пр.) не падали.
  try {
    Directory('data').createSync(recursive: true);
  } catch (_) {}
  PiarCore.instance.init();
  runApp(const PiarApp());
}
