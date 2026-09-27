import 'dart:async';

import 'package:flutter/material.dart';

import '../../settings/settings.dart';
import '../../shopapi/client.dart';
import '../../shopapi/exceptions.dart';
import '../../shopapi/models.dart';

/// Раздел «Магазин»: dark.shopping — витрины «Аккаунты» и «Чаты»,
/// баланс, заказы с опросом статуса и выдачей.
class ShopScreen extends StatefulWidget {
  const ShopScreen({super.key});

  @override
  State<ShopScreen> createState() => _ShopScreenState();
}

class _ShopScreenState extends State<ShopScreen> {
  DarkShoppingClient? _client;
  double? _balance;
  bool _loading = true;
  String? _error;

  Group? _accountsGroup;
  Group? _chatsGroup;

  @override
  void initState() {
    super.initState();
    _bootstrap();
  }

  Future<void> _bootstrap() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    final key = await SettingsStorage.getApiKey();
    if (key == null || key.isEmpty) {
      if (mounted) {
        setState(() => _loading = false);
        await _askApiKey();
      }
      return;
    }
    await _connect(key);
  }

  Future<void> _connect(String key) async {
    final client = DarkShoppingClient(apiKey: key);
    try {
      final categories = await client.getCategories();
      final tg = categories.where((c) =>
          c.name.toLowerCase().contains('telegram')).toList();
      if (tg.isEmpty) {
        throw DarkShoppingApiException(
            message: 'категория Telegram не найдена в каталоге');
      }
      final groups =
          await client.getGroups(categoryId: tg.first.id);
      Group? accounts;
      Group? chats;
      for (final g in groups) {
        final n = g.name.toLowerCase();
        if (accounts == null && n.contains('ручная регистрация')) {
          accounts = g;
        }
        if (chats == null &&
            (n.contains('каналы') || n.contains('группы') || n.contains('чаты'))) {
          chats = g;
        }
      }
      final balance = await client.getBalance();
      if (!mounted) return;
      setState(() {
        _client = client;
        _balance = balance.amount;
        _accountsGroup = accounts;
        _chatsGroup = chats;
        _loading = false;
      });
    } on DarkShoppingApiException catch (e) {
      if (!mounted) return;
      setState(() {
        _error = e.toString();
        _loading = false;
      });
    }
  }

  Future<void> _askApiKey() async {
    final ctrl = TextEditingController();
    final key = await showDialog<String>(
      context: context,
      builder: (_) => AlertDialog(
        title: const Text('API-ключ dark.shopping'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Text(
              'Ключ выдаётся на странице настроек аккаунта dark.shopping '
              '(нужна заявка на API).',
              style: TextStyle(fontSize: 13),
            ),
            const SizedBox(height: 8),
            TextField(
              controller: ctrl,
              decoration: const InputDecoration(
                  labelText: 'Ключ', hintText: '7dc8d2ee...'),
            ),
          ],
        ),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(context), child: const Text('Отмена')),
          FilledButton(
              onPressed: () => Navigator.pop(context, ctrl.text.trim()),
              child: const Text('Сохранить')),
        ],
      ),
    );
    if (key != null && key.isNotEmpty) {
      await SettingsStorage.setApiKey(key);
      await _connect(key);
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_loading) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_client == null) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(_error ?? 'API-ключ dark.shopping не задан'),
            const SizedBox(height: 12),
            FilledButton.icon(
              onPressed: _askApiKey,
              icon: const Icon(Icons.key),
              label: Text(_error == null ? 'Указать ключ' : 'Сменить ключ'),
            ),
          ],
        ),
      );
    }
    return Scaffold(
      backgroundColor: Colors.transparent,
      appBar: AppBar(
        backgroundColor: Colors.transparent,
        title: const Text('Магазин dark.shopping'),
        actions: [
          if (_balance != null)
            Padding(
              padding: const EdgeInsets.only(right: 8),
              child: Center(
                child: Chip(
                  avatar: const Icon(Icons.account_balance_wallet_outlined,
                      size: 18),
                  label: Text('${_balance!.toStringAsFixed(2)} ₽'),
                ),
              ),
            ),
          IconButton(
            tooltip: 'Обновить',
            onPressed: () => _connect(_client!.apiKey),
            icon: const Icon(Icons.refresh),
          ),
          IconButton(
            tooltip: 'API-ключ',
            onPressed: _askApiKey,
            icon: const Icon(Icons.key_outlined),
          ),
        ],
      ),
      body: DefaultTabController(
        length: 2,
        child: Column(
          children: [
            const TabBar(
              isScrollable: false,
              tabs: [
                Tab(icon: Icon(Icons.person_outline), text: 'Аккаунты'),
                Tab(icon: Icon(Icons.forum_outlined), text: 'Чаты'),
              ],
            ),
            Expanded(
              child: TabBarView(
                children: [
                  _Storefront(
                      client: _client!,
                      group: _accountsGroup,
                      title: 'Аккаунты'),
                  _Storefront(client: _client!, group: _chatsGroup, title: 'Чаты'),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Витрина одной группы товаров.
class _Storefront extends StatefulWidget {
  const _Storefront({
    required this.client,
    required this.group,
    required this.title,
  });

  final DarkShoppingClient client;
  final Group? group;
  final String title;

  @override
  State<_Storefront> createState() => _StorefrontState();
}

class _StorefrontState extends State<_Storefront> {
  final List<Product> _products = [];
  bool _loading = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    if (widget.group == null) {
      setState(() => _error = 'Группа товаров не найдена в каталоге');
      return;
    }
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final page = await widget.client.getProducts(ProductFilter(
        groupId: widget.group!.id,
      ));
      if (!mounted) return;
      setState(() {
        _products
          ..clear()
          ..addAll(page.items);
        _loading = false;
      });
    } on DarkShoppingApiException catch (e) {
      if (!mounted) return;
      setState(() {
        _error = e.toString();
        _loading = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_loading) return const Center(child: CircularProgressIndicator());
    if (_error != null) {
      return Center(child: Text(_error!));
    }
    if (_products.isEmpty) {
      return const Center(child: Text('Товаров нет (нет в наличии).'));
    }
    return RefreshIndicator(
      onRefresh: _load,
      child: ListView.separated(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 24),
        itemCount: _products.length,
        separatorBuilder: (_, _) => const SizedBox(height: 8),
        itemBuilder: (context, i) {
          final p = _products[i];
          return Card(
            child: ListTile(
              title: Text(
                p.name,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
              ),
              subtitle: Text(
                '${p.price.toStringAsFixed(2)} ₽ · в наличии: ${p.quantity} · '
                'мин. заказ: ${p.minimumOrder}'
                '${p.autoDelivery ? ' · авто-выдача' : ' · ручная выдача'}'
                '${p.qualityPercent != null ? ' · качество: ${p.qualityPercent!.toStringAsFixed(0)}%' : ''}',
              ),
              trailing: const Icon(Icons.chevron_right),
              onTap: () => _openProduct(context, p),
            ),
          );
        },
      ),
    );
  }

  Future<void> _openProduct(BuildContext context, Product p) async {
    final qtyCtrl = TextEditingController(text: '${p.minimumOrder}');
    final bought = await showDialog<bool>(
      context: context,
      builder: (_) => _ProductDialog(
        product: p,
        qtyCtrl: qtyCtrl,
        onBuy: (qty) => _buy(p, qty),
      ),
    );
    if (bought == true) {
      _load();
    }
  }

  Future<void> _buy(Product p, int qty) async {
    final order = await widget.client.createOrder(product: p.id, quantity: qty);
    var orderId = order.id;
    if (orderId == null) {
      // возможно, ответ содержит только ссылку — считаем завершённым
      return;
    }
    // опрос статуса: макс 10 минут
    for (var i = 0; i < 200; i++) {
      await Future<void>.delayed(const Duration(seconds: 3));
      final st = await widget.client.getOrderStatus(orderId);
      switch (st.kind) {
        case OrderStatusKind.completed:
          final link = await widget.client.getOrderDownload(orderId);
          if (link.isNotEmpty) {
            await _showDelivery(link);
          }
          return;
        case OrderStatusKind.canceled ||
              OrderStatusKind.error ||
              OrderStatusKind.refund:
          throw DarkShoppingApiException(
              message: 'заказ ${st.kind.name}: ${st.raw}');
        default:
          break;
      }
    }
    throw DarkShoppingApiException(message: 'выдача не завершилась за 10 минут');
  }

  Future<void> _showDelivery(String link) async {
    await showDialog<void>(
      context: context,
      builder: (_) => AlertDialog(
        title: const Text('Заказ выдан'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Text('Ссылка на файл заказа:'),
            const SizedBox(height: 4),
            SelectableText(link),
            const SizedBox(height: 12),
            const Text(
              'Импорт купленных аккаунтов/чатов в приложение появится '
              'в следующей фазе.',
              style: TextStyle(fontSize: 12),
            ),
          ],
        ),
        actions: [
          FilledButton(
              onPressed: () => Navigator.pop(context),
              child: const Text('Понятно')),
        ],
      ),
    );
  }
}

/// Диалог товара: описание + количество + покупка.
class _ProductDialog extends StatefulWidget {
  const _ProductDialog({
    required this.product,
    required this.qtyCtrl,
    required this.onBuy,
  });

  final Product product;
  final TextEditingController qtyCtrl;
  final Future<void> Function(int qty) onBuy;

  @override
  State<_ProductDialog> createState() => _ProductDialogState();
}

class _ProductDialogState extends State<_ProductDialog> {
  bool _busy = false;
  String? _status;

  @override
  Widget build(BuildContext context) {
    final p = widget.product;
    return AlertDialog(
      title: Text(p.name, style: const TextStyle(fontSize: 16)),
      content: SizedBox(
        width: 420,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (p.description != null && p.description!.isNotEmpty)
              Text(
                p.description!,
                maxLines: 8,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(fontSize: 12),
              ),
            const SizedBox(height: 8),
            Text('Цена: ${p.price.toStringAsFixed(2)} ₽ × количество'),
            const SizedBox(height: 8),
            TextField(
              controller: widget.qtyCtrl,
              keyboardType: TextInputType.number,
              decoration: InputDecoration(
                labelText: 'Количество (мин. ${p.minimumOrder})',
                border: const OutlineInputBorder(),
                suffixText:
                    '≈ ${(p.price * (int.tryParse(widget.qtyCtrl.text) ?? p.minimumOrder)).toStringAsFixed(2)} ₽',
              ),
            ),
            if (_status != null) ...[
              const SizedBox(height: 8),
              Text(_status!, style: const TextStyle(fontSize: 12)),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: _busy ? null : () => Navigator.pop(context, false),
          child: const Text('Закрыть'),
        ),
        FilledButton(
          onPressed: _busy ? null : _buy,
          child: const Text('Купить'),
        ),
      ],
    );
  }

  Future<void> _buy() async {
    final qty = int.tryParse(widget.qtyCtrl.text.trim());
    if (qty == null || qty < widget.product.minimumOrder) {
      setState(() => _status = 'Минимум для заказа: ${widget.product.minimumOrder}');
      return;
    }
    setState(() {
      _busy = true;
      _status = 'Оформляем заказ…';
    });
    try {
      setState(() => _status = 'Ожидаем выдачу (опрос каждые 3 сек)…');
      await widget.onBuy(qty);
      if (mounted) Navigator.pop(context, true);
    } catch (e) {
      if (mounted) {
        setState(() {
          _busy = false;
          _status = 'Ошибка: $e';
        });
      }
    }
  }
}
