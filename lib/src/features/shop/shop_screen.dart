import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../core/native.dart';
import '../../ui/theme.dart';
/// Раздел «Магазин»: dark.shopping ВСТРОЕН в окно приложения (нативный
/// PlatformView WebView). Переключение вкладок не перезагружает сайт,
/// логин/баланс сохраняются, файлы заказов скачиваются в «Загрузки».
/// На не-Android (вне устройства) — заглушка с кнопкой внешнего открытия.
class ShopScreen extends StatefulWidget {
  const ShopScreen({super.key});

  @override
  State<ShopScreen> createState() => _ShopScreenState();
}

class _ShopScreenState extends State<ShopScreen> {
  String _url = 'https://dark.shopping/';
  bool _canBack = false;
  bool _canFwd = false;

  static const _shopChannel = MethodChannel('piarapk/paths');

  @override
  void initState() {
    super.initState();
    _shopChannel.setMethodCallHandler((call) async {
      if (call.method == 'shopUrl') {
        final args = call.arguments;
        if (args is Map && mounted) {
          setState(() {
            _url = args['url']?.toString() ?? _url;
            _canBack = args['canGoBack'] == true;
            _canFwd = args['canGoForward'] == true;
          });
        }
      }
      return null;
    });
  }

  @override
  void dispose() {
    // сбрасываем обработчик канала: иначе замыкание держит State вечно
    _shopChannel.setMethodCallHandler(null);
    super.dispose();
  }

  void _do(String method) {
    unawaited(Native.rawChannelCall(method));
  }

  Future<void> _openExternally(String url) async {
    final ok = await Native.openShop(url);
    if (!ok && mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('Не удалось открыть браузер')),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    // В тестах/на не-Android — заглушка запуска (тесты идут на хосте,
    // Platform.isAndroid там false)
    if (!kIsWeb && Platform.isAndroid) {
      return _embedded(context);
    }
    return _fallback(context);
  }

  /// Встроенный браузер.
  Widget _embedded(BuildContext context) {
    return Column(
      children: [
        // панель навигации
        Container(
          color: Theme.of(context).colorScheme.surface,
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
          child: Row(
            children: [
              IconButton(
                tooltip: 'Назад',
                visualDensity: VisualDensity.compact,
                onPressed: _canBack ? () => _do('shopBack') : null,
                icon: const Icon(Icons.arrow_back, size: 20),
              ),
              IconButton(
                tooltip: 'Вперёд',
                visualDensity: VisualDensity.compact,
                onPressed: _canFwd ? () => _do('shopForward') : null,
                icon: const Icon(Icons.arrow_forward, size: 20),
              ),
              IconButton(
                tooltip: 'Обновить',
                visualDensity: VisualDensity.compact,
                onPressed: () => _do('shopReload'),
                icon: const Icon(Icons.refresh, size: 20),
              ),
              Expanded(
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 6),
                  child: Text(
                    _url,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(fontSize: 12, fontFamily: 'monospace'),
                  ),
                ),
              ),
              IconButton(
                tooltip: 'Открыть в отдельном окне',
                visualDensity: VisualDensity.compact,
                onPressed: () => _openExternally(_url),
                icon: const Icon(Icons.open_in_new, size: 18),
              ),
            ],
          ),
        ),
        const Divider(height: 1),
        // сам WebView
        Expanded(
          child: AndroidView(
            viewType: 'shop-webview',
            gestureRecognizers: <Factory<OneSequenceGestureRecognizer>>{
              Factory<PanGestureRecognizer>(() => PanGestureRecognizer()),
              Factory<ScaleGestureRecognizer>(() => ScaleGestureRecognizer()),
            },
          ),
        ),
      ],
    );
  }

  /// Заглушка (десктоп/тесты): экран запуска внешнего браузера.
  Widget _fallback(BuildContext context) {
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
              'dark.shopping — аккаунты и чаты с отлегой.',
              style: Theme.of(context).textTheme.bodySmall,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 24),
            GradientButton(
              onPressed: Native.supported
                  ? () => _openExternally('https://dark.shopping/')
                  : null,
              label: 'Открыть магазин',
              icon: const Icon(Icons.open_in_browser),
            ),
            if (!Native.supported) ...[
              const SizedBox(height: 8),
              Text(
                'Магазин доступен только на Android.',
                style: Theme.of(context).textTheme.bodySmall,
                textAlign: TextAlign.center,
              ),
            ],
          ],
        ),
      ),
    );
  }
}
