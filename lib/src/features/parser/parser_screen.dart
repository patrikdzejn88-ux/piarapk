import 'package:flutter/material.dart';

/// Раздел «Парсер» — сбор участников чатов (в разработке).
class ParserScreen extends StatelessWidget {
  const ParserScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Icons.manage_search,
              size: 64,
              color: Theme.of(context).colorScheme.primary.withValues(alpha: 0.5)),
          const SizedBox(height: 12),
          Text('Раздел «Парсер» в разработке',
              style: Theme.of(context).textTheme.titleMedium),
          const SizedBox(height: 8),
          Text(
            'Сбор участников/авторов из чата на отдельном пуле аккаунтов.',
            style: Theme.of(context).textTheme.bodySmall,
          ),
        ],
      ),
    );
  }
}
