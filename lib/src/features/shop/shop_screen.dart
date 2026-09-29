import 'package:flutter/material.dart';

import '../../core/native.dart';
import '../../ui/theme.dart';

/// Раздел «Магазин»: запуск встроенного браузера dark.shopping.
/// Покупка расходников (аккаунты/чаты) идёт на самом сайте —
/// логин и баланс сохраняются между открытиями.
class ShopScreen extends StatelessWidget {
  const ShopScreen({super.key});

  Future<void> _open() async {
    await Native.openShop('https://dark.shopping/');
  }

  @override
  Widget build(BuildContext context) {
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 520),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ShaderMask(
              blendMode: BlendMode.srcIn,
              shaderCallback: (b) => AppGradients.primary.createShader(b),
              child: const Icon(Icons.shopping_bag_outlined, size: 84),
            ),
            const SizedBox(height: 16),
            GradientText('Магазин расходников',
                style: Theme.of(context).textTheme.headlineSmall),
            const SizedBox(height: 8),
            Text(
              'dark.shopping — аккаунты и чаты с отлегой.\n'
              'Открывается во встроенном браузере: логинишься один раз, '
              'покупаешь расходники — логин и баланс сохраняются.',
              style: Theme.of(context).textTheme.bodySmall,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 24),
            GradientButton(
              onPressed: _open,
              label: 'Открыть магазин',
              icon: const Icon(Icons.open_in_browser),
            ),
            const SizedBox(height: 12),
            Text(
              'Купленные аккаунты импортируй в разделе «Аккаунты»\n'
              '(StringSession или tdata-архив).',
              style: Theme.of(context).textTheme.bodySmall,
              textAlign: TextAlign.center,
            ),
          ],
        ),
      ),
    );
  }
}
