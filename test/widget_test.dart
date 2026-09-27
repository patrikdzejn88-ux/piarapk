import 'package:flutter_test/flutter_test.dart';

import 'package:piarapk/src/app.dart';

void main() {
  testWidgets('Приложение рендерит навигацию и разделы', (tester) async {
    await tester.pumpWidget(const PiarApp());

    // Навигация
    expect(find.text('Аккаунты'), findsWidgets);
    expect(find.text('Пиар'), findsWidgets);
    expect(find.text('Парсер'), findsWidgets);
    expect(find.text('Магазин'), findsWidgets);

    // Раздел аккаунтов открыт по умолчанию
    expect(find.text('Пиар'), findsWidgets); // вкладка пула
    expect(find.text('Добавить аккаунт'), findsOneWidget);
  });

  testWidgets('Заглушки разделов открываются', (tester) async {
    await tester.pumpWidget(const PiarApp());

    await tester.tap(find.text('Пиар').last);
    await tester.pumpAndSettle();
    expect(find.text('Раздел «Пиар» в разработке'), findsOneWidget);

    await tester.tap(find.text('Парсер').last);
    await tester.pumpAndSettle();
    expect(find.text('Раздел «Парсер» в разработке'), findsOneWidget);
  });
}
