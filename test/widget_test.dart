import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:piarapk/src/app.dart';

void main() {
  testWidgets('Приложение рендерит навигацию и раздел аккаунтов',
      (tester) async {
    await tester.pumpWidget(const PiarApp());

    // Навигация (рельса на широком экране тест-стенда)
    expect(find.text('Аккаунты'), findsWidgets);
    expect(find.text('Пиар'), findsWidgets);
    expect(find.text('Парсер'), findsWidgets);
    expect(find.text('Магазин'), findsWidgets);

    // Раздел аккаунтов открыт по умолчанию
    expect(find.text('Добавить аккаунт'), findsOneWidget);
  });

  testWidgets('Навигация: магазин открывает экран запуска браузера',
      (tester) async {
    await tester.pumpWidget(const PiarApp());

    // «Магазин» уникален в навигации (во вкладках пулов его нет)
    await tester.tap(find.text('Магазин'));
    await tester.pumpAndSettle();

    // магазин теперь — встроенный браузер dark.shopping
    expect(find.text('Магазин расходников'), findsOneWidget);
    expect(find.text('Открыть магазин'), findsOneWidget);
  });

  testWidgets('Кнопка «Добавить аккаунт» открывает диалог добавления',
      (tester) async {
    await tester.pumpWidget(const PiarApp());

    await tester.tap(find.text('Добавить аккаунт'));
    await tester.pumpAndSettle();

    // диалог открыт: выбор способа добавления
    expect(find.text('По номеру телефона (код + 2FA)'), findsOneWidget);
    expect(find.text('Из StringSession (telethon/gramjs)'), findsOneWidget);
    expect(find.text('Из tdata (zip-архив)'), findsOneWidget);
    // никакого «Диалог не открылся»
    expect(find.byType(SnackBar), findsNothing);
  });
}
