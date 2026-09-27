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
    (icon: Icon(Icons.people_outline), label: 'Аккаунты'),
    (icon: Icon(Icons.campaign_outlined), label: 'Пиар'),
    (icon: Icon(Icons.manage_search), label: 'Парсер'),
    (icon: Icon(Icons.shopping_bag_outlined), label: 'Магазин'),
  ];

  @override
  void initState() {
    super.initState();
    PiarCore.instance.init();
  }

  @override
  Widget build(BuildContext context) {
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
                  label: Text(d.label),
                ),
            ],
          ),
          const VerticalDivider(width: 1),
          Expanded(
            child: switch (_index) {
              0 => const AccountsScreen(),
              1 => const PiarScreen(),
              2 => const ParserScreen(),
              _ => const ShopScreen(),
            },
          ),
        ],
      ),
    );
  }
}
