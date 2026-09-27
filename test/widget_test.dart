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

  testWidgets('Навигация: магазин открывается и запрашивает API-ключ',
      (tester) async {
    await tester.pumpWidget(const PiarApp());

    // «Магазин» уникален в навигации (во вкладках пулов его нет)
    await tester.tap(find.text('Магазин'));
    await tester.pumpAndSettle();

    // Ключа нет → открывается диалог ввода API-ключа dark.shopping
    expect(find.text('API-ключ dark.shopping'), findsWidgets);
  });
}
