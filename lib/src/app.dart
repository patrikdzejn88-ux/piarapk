import 'package:flutter/material.dart';

import 'core/bridge.dart';
import 'features/accounts/accounts_screen.dart';
import 'features/parser/parser_screen.dart';
import 'features/piar/piar_screen.dart';
import 'features/shop/shop_screen.dart';

class PiarApp extends StatelessWidget {
  const PiarApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'piarapk',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xFF3FA7FF),
          brightness: Brightness.dark,
        ),
        useMaterial3: true,
      ),
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

  static const _destinations = [
    (icon: Icon(Icons.people_outline), selected: Icon(Icons.people), label: 'Аккаунты'),
    (icon: Icon(Icons.campaign_outlined), selected: Icon(Icons.campaign), label: 'Пиар'),
    (icon: Icon(Icons.manage_search), selected: Icon(Icons.manage_search), label: 'Парсер'),
    (icon: Icon(Icons.shopping_bag_outlined), selected: Icon(Icons.shopping_bag), label: 'Магазин'),
  ];

  @override
  void initState() {
    super.initState();
    PiarCore.instance.init();
  }

  static Widget _page(int i) => switch (i) {
        0 => const AccountsScreen(),
        1 => const PiarScreen(),
        2 => const ParserScreen(),
        _ => const ShopScreen(),
      };

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final wide = constraints.maxWidth >= 720;
        final body = _page(_index);
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
