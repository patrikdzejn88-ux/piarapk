import 'dart:io';

import 'package:flutter/widgets.dart';

import 'src/app.dart';
import 'src/core/bridge.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  // Каталог данных приложения создаётся мостом при init(); принудительно
  // готовим рабочую папку до запуска UI, чтобы файловые операции UI
  // (настройки магазина и пр.) не падали.
  try {
    Directory('data').createSync(recursive: true);
  } catch (_) {}
  PiarCore.instance.init();
  runApp(const PiarApp());
}
