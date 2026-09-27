import 'package:flutter/material.dart';

/// Раздел «Магазин» (dark.shopping) — реализуется агентом магазина.
/// Эта заглушка будет заменена витринами «Аккаунты» / «Чаты».
class ShopScreen extends StatelessWidget {
  const ShopScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Icons.shopping_bag_outlined,
              size: 64,
              color: Theme.of(context).colorScheme.primary.withValues(alpha: 0.5)),
          const SizedBox(height: 12),
          Text('Магазин dark.shopping',
              style: Theme.of(context).textTheme.titleMedium),
          const SizedBox(height: 8),
          Text(
            'Витрины закупки аккаунтов и чатов подключаются.',
            style: Theme.of(context).textTheme.bodySmall,
          ),
        ],
      ),
    );
  }
}
