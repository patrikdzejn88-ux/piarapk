import 'package:flutter/material.dart';

/// Раздел «Пиар» — инвайты в чат + сообщение (в разработке).
class PiarScreen extends StatelessWidget {
  const PiarScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Icons.campaign_outlined,
              size: 64,
              color: Theme.of(context).colorScheme.primary.withValues(alpha: 0.5)),
          const SizedBox(height: 12),
          Text('Раздел «Пиар» в разработке',
              style: Theme.of(context).textTheme.titleMedium),
          const SizedBox(height: 8),
          Text(
            'Инвайты пользователей в чат, сообщение, чат только для чтения.',
            style: Theme.of(context).textTheme.bodySmall,
          ),
        ],
      ),
    );
  }
}
