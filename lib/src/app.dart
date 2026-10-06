import 'dart:async';

import 'package:flutter/material.dart';

import 'core/bridge.dart';
import 'core/native.dart';
import 'features/accounts/accounts_screen.dart';
import 'features/parser/parser_screen.dart';
import 'features/piar/piar_screen.dart';
import 'features/shop/shop_screen.dart';
import 'ui/theme.dart';

class PiarApp extends StatelessWidget {
  const PiarApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'piarapk',
      debugShowCheckedModeBanner: false,
      theme: AppTheme.dark(),
      home: const _Shell(),
    );
  }
}

class _Shell extends StatefulWidget {
  const _Shell();

  @override
  State<_Shell> createState() => _ShellState();
}

class _ShellState extends State<_Shell> {
  int _index = 0;
  StreamSubscription<PiarEvent>? _eventsSub;
  late final AppLifecycleListener _lifecycle;

  static const _destinations = [
    (icon: Icon(Icons.people_outline), selected: Icon(Icons.people), label: 'Аккаунты'),
    (icon: Icon(Icons.campaign_outlined), selected: Icon(Icons.campaign), label: 'Пиар'),
    (icon: Icon(Icons.manage_search), selected: Icon(Icons.manage_search), label: 'Парсер'),
    (icon: Icon(Icons.shopping_bag_outlined), selected: Icon(Icons.shopping_bag), label: 'Магазин'),
  ];

  @override
  void initState() {
    super.initState();
    unawaited(PiarCore.instance.init());
    unawaited(Native.shopWarmup());
    // останавливаем поллинг ядра в бэкграунде и возобновляем при возврате
    _lifecycle = AppLifecycleListener(
      onStateChange: (state) {
        switch (state) {
          case AppLifecycleState.resumed:
            PiarCore.instance.resumePolling();
          case AppLifecycleState.paused:
          case AppLifecycleState.hidden:
            PiarCore.instance.pausePolling();
          case AppLifecycleState.inactive:
          case AppLifecycleState.detached:
            break;
        }
      },
    );
    // глобальный слушатель: сбор закончен (в т.ч. остановлен/ошибка) —
    // останавливаем фоновый сервис, даже если экран парсера уже закрыт
    _eventsSub = PiarCore.instance.events.listen((e) {
      if (e.type == 'result' && e.method == 'parse_start') {
        unawaited(Native.parserServiceStop());
      }
    });
  }

  @override
  void dispose() {
    _lifecycle.dispose();
    unawaited(_eventsSub?.cancel());
    // ядро получает piar_shutdown при завершении приложения
    PiarCore.instance.shutdown();
    super.dispose();
  }

  static const _pages = [
    AccountsScreen(),
    PiarScreen(),
    ParserScreen(),
    ShopScreen(),
  ];

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final wide = constraints.maxWidth >= 720;
        // IndexedStack: все разделы живы одновременно — магазин не
        // перезагружается при переключении вкладок, состояние парсера
        // и пиара не теряется
        final body = IndexedStack(index: _index, children: _pages);
        if (wide) {
          return Scaffold(
            body: Row(
              children: [
                NavigationRail(
                  selectedIndex: _index,
                  onDestinationSelected: (i) => setState(() => _index = i),
                  labelType: NavigationRailLabelType.all,
                  destinations: [
                    for (final d in _destinations)
                      NavigationRailDestination(
                        icon: d.icon,
                        selectedIcon: d.selected,
                        label: Text(d.label),
                      ),
                  ],
                ),
                const VerticalDivider(width: 1),
                Expanded(child: body),
              ],
            ),
          );
        }
        return Scaffold(
          body: body,
          bottomNavigationBar: NavigationBar(
            selectedIndex: _index,
            onDestinationSelected: (i) => setState(() => _index = i),
            destinations: [
              for (final d in _destinations)
                NavigationDestination(
                  icon: d.icon,
                  selectedIcon: d.selected,
                  label: d.label,
                ),
            ],
          ),
        );
      },
    );
  }
}
