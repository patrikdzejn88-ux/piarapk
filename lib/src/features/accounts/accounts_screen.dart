import 'package:flutter/material.dart';

import '../../core/bridge.dart';
import 'account.dart';
import 'accounts_controller.dart';
import 'add_account_dialog.dart';

/// Раздел «Аккаунты»: два пула (Пиар / Парсер).
class AccountsScreen extends StatefulWidget {
  const AccountsScreen({super.key});

  @override
  State<AccountsScreen> createState() => _AccountsScreenState();
}

class _AccountsScreenState extends State<AccountsScreen> {
  final _controller = AccountsController();

  @override
  void initState() {
    super.initState();
    _controller.reload();
    // перерисоваться, когда ядро доинициализируется (UI-гонка: баннер
    // «ядро не найдено» иначе остаётся навсегда при успешной поздней загрузке)
    PiarCore.instance.init().then((_) {
      if (mounted) {
        setState(() {});
        _controller.reload();
      }
    });
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final core = PiarCore.instance;
    return DefaultTabController(
      length: 2,
      child: Scaffold(
        backgroundColor: Colors.transparent,
        body: Column(
          children: [
            if (!core.available)
              _NoticeBox(
                text: core.lastError != null
                    ? 'Ядро piarcore не загрузилось: ${core.lastError}'
                    : 'Ядро piarcore не найдено рядом с приложением — управление '
                        'аккаунтами недоступно (сборка без Rust-библиотеки).',
              ),
            const TabBar(
              isScrollable: false,
              tabs: [
                Tab(icon: Icon(Icons.campaign_outlined), text: 'Пиар'),
                Tab(icon: Icon(Icons.manage_search), text: 'Парсер'),
              ],
            ),
            Expanded(
              child: TabBarView(
                children: [
                  _PoolView(pool: 'piar', controller: _controller),
                  _PoolView(pool: 'parser', controller: _controller),
                ],
              ),
            ),
          ],
        ),
        floatingActionButton: FloatingActionButton.extended(
          onPressed: _openAddDialog,
          icon: const Icon(Icons.person_add_alt),
          label: const Text('Добавить аккаунт'),
        ),
      ),
    );
  }

  Future<void> _openAddDialog() async {
    final tabController = DefaultTabController.of(context);
    final pool = tabController.index == 0 ? 'piar' : 'parser';
    await showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (_) => AddAccountDialog(controller: _controller, pool: pool),
    );
  }
}

class _PoolView extends StatelessWidget {
  const _PoolView({required this.pool, required this.controller});

  final String pool;
  final AccountsController controller;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        if (controller.loading && controller.byPool(pool).isEmpty) {
          return const Center(child: CircularProgressIndicator());
        }
        if (controller.lastError != null) {
          return Center(child: Text('Ошибка: ${controller.lastError}'));
        }
        final accounts = controller.byPool(pool);
        if (accounts.isEmpty) {
          return const Center(child: Text('Аккаунтов нет. Добавьте первый.'));
        }
        return ListView.separated(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 88),
          itemCount: accounts.length,
          separatorBuilder: (_, _) => const SizedBox(height: 8),
          itemBuilder: (context, i) => _AccountCard(
            account: accounts[i],
            controller: controller,
          ),
        );
      },
    );
  }
}

class _AccountCard extends StatelessWidget {
  const _AccountCard({required this.account, required this.controller});

  final Account account;
  final AccountsController controller;

  @override
  Widget build(BuildContext context) {
    final (label, color) = switch ((account.connected, account.restricted)) {
      (false, true) => ('Ограничен', Colors.redAccent),
      (true, false) => ('Подключён', Colors.greenAccent),
      _ => ('Офлайн', Colors.orangeAccent),
    };
    return Card(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        child: Row(
          children: [
            CircleAvatar(
              child: Text(
                account.displayName.isNotEmpty ? account.displayName[0] : '?',
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    account.displayName,
                    style: Theme.of(context).textTheme.titleMedium,
                  ),
                  const SizedBox(height: 2),
                  Text(
                    [
                      if (account.phone.isNotEmpty) account.phone,
                      if (account.username.isNotEmpty) '@${account.username}',
                    ].join(' · '),
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                ],
              ),
            ),
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
              decoration: BoxDecoration(
                color: color.withValues(alpha: 0.15),
                borderRadius: BorderRadius.circular(12),
              ),
              child: Text(label,
                  style: TextStyle(color: color, fontSize: 12)),
            ),
            PopupMenuButton<String>(
              onSelected: (v) => _onMenu(context, v),
              itemBuilder: (_) => [
                if (!account.connected)
                  const PopupMenuItem(
                      value: 'connect', child: Text('Подключить')),
                if (account.connected)
                  const PopupMenuItem(
                      value: 'disconnect', child: Text('Отключить')),
                const PopupMenuItem(
                  value: 'delete',
                  child:
                      Text('Удалить', style: TextStyle(color: Colors.redAccent)),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  Future<void> _onMenu(BuildContext context, String action) async {
    switch (action) {
      case 'connect':
        await controller.connect(account.id);
      case 'disconnect':
        await controller.disconnect(account.id);
      case 'delete':
        final ok = await showDialog<bool>(
          context: context,
          builder: (_) => AlertDialog(
            title: const Text('Удалить аккаунт?'),
            content: Text(
                'Аккаунт ${account.displayName} будет удалён вместе с сессией.'),
            actions: [
              TextButton(
                  onPressed: () => Navigator.pop(context, false),
                  child: const Text('Отмена')),
              FilledButton(
                  onPressed: () => Navigator.pop(context, true),
                  child: const Text('Удалить')),
            ],
          ),
        );
        if (ok == true) {
          await controller.delete(account.id);
        }
    }
  }
}

class _NoticeBox extends StatelessWidget {
  const _NoticeBox({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: double.infinity,
      color: Colors.orangeAccent.withValues(alpha: 0.15),
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      child: Row(
        children: [
          const Icon(Icons.warning_amber_outlined, color: Colors.orangeAccent),
          const SizedBox(width: 8),
          Expanded(
            child: Text(text, style: const TextStyle(fontSize: 13)),
          ),
        ],
      ),
    );
  }
}
