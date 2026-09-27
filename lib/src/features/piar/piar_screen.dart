import 'dart:async';

import 'package:flutter/material.dart';

import '../../core/bridge.dart';

/// Раздел «Пиар»: инвайты в чат по базе + сообщение.
/// Протокол ядра: list_chats, list_databases, invite_start, post_message.
class PiarScreen extends StatefulWidget {
  const PiarScreen({super.key});

  @override
  State<PiarScreen> createState() => _PiarScreenState();
}

class _PiarScreenState extends State<PiarScreen> {
  Map<String, dynamic>? _chat;
  String? _database;
  final _messageCtrl = TextEditingController();
  final _perAccountCtrl = TextEditingController(text: '20');
  final _pauseCtrl = TextEditingController(text: '5000');

  bool _busy = false;
  String? _progressLine;
  Map<String, dynamic>? _summary;
  StreamSubscription<PiarEvent>? _sub;

  List<Map<String, dynamic>> _chats = [];
  List<Map<String, dynamic>> _databases = [];

  @override
  void initState() {
    super.initState();
    _reload();
    _sub = PiarCore.instance.events.listen(_onEvent);
  }

  @override
  void dispose() {
    _sub?.cancel();
    _messageCtrl.dispose();
    _perAccountCtrl.dispose();
    _pauseCtrl.dispose();
    super.dispose();
  }

  void _reload() {
    final chats = PiarCore.instance.call('list_chats', {});
    if (chats.ok && chats.data is List) {
      _chats = (chats.data as List)
          .whereType<Map>()
          .map((m) => m.cast<String, dynamic>())
          .toList();
      if (_chat == null && _chats.isNotEmpty) {
        _chat = _chats.first;
      }
    }
    final dbs = PiarCore.instance.call('list_databases', {});
    if (dbs.ok && dbs.data is List) {
      _databases = (dbs.data as List)
          .whereType<Map>()
          .map((m) => m.cast<String, dynamic>())
          .toList();
      _database ??= _databases.isNotEmpty ? _databases.first['name'] as String? : null;
    }
    if (mounted) setState(() {});
  }

  void _onEvent(PiarEvent e) {
    if (e.method == 'invite_start') {
      if (e.type == 'progress' && mounted) {
        final d = e.data;
        setState(() => _progressLine =
            'Инвайты: ${d?['done'] ?? 0} ок, ${d?['failed'] ?? 0} ошибок'
                '${d?['current_account'] != null ? ' · аккаунт ${d!['current_account']}' : ''}');
      }
      if (e.type == 'result' && mounted) {
        setState(() {
          _busy = false;
          _summary = e.ok ? (e.data is Map ? e.data as Map<String, dynamic> : null) : null;
          _progressLine = e.ok ? null : 'Ошибка: ${e.error}';
        });
      }
    }
  }

  Future<void> _start() async {
    if (_chat == null || _database == null) {
      _snack('Выберите чат и базу получателей');
      return;
    }
    setState(() {
      _busy = true;
      _summary = null;
      _progressLine = 'Запуск…';
    });
    final res = await PiarCore.instance.callAsync('invite_start', {
      'chat_id': _chat!['id'],
      'database': _database,
      'message': _messageCtrl.text.trim(),
      'per_account': int.tryParse(_perAccountCtrl.text.trim()) ?? 20,
      'batch_pause_ms': int.tryParse(_pauseCtrl.text.trim()) ?? 5000,
    });
    if (!res.ok && mounted) {
      setState(() {
        _busy = false;
        _progressLine = 'Ошибка: ${res.errorText}';
      });
    }
  }

  Future<void> _postMessage() async {
    if (_chat == null) {
      _snack('Выберите чат');
      return;
    }
    final res = await PiarCore.instance
        .callAsync('post_message', {'chat_id': _chat!['id'], 'text': _messageCtrl.text.trim()});
    _snack(res.ok ? 'Сообщение отправлено' : 'Ошибка: ${res.errorText}');
  }

  Future<void> _addChat() async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (_) => const _AddChatDialog(),
    );
    if (ok == true) {
      _reload();
    }
  }

  void _snack(String text) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  @override
  Widget build(BuildContext context) {
    if (!PiarCore.instance.available) {
      return const Center(child: Text('Ядро не загружено — раздел недоступен.'));
    }
    return ListView(      padding: const EdgeInsets.all(16),
      children: [
        Text('Пиар-рассылка', style: Theme.of(context).textTheme.titleLarge),
        const SizedBox(height: 4),
        Text('Инвайты из базы в чат + сообщение. Чат настроен «только для чтения».',
            style: Theme.of(context).textTheme.bodySmall),
        const SizedBox(height: 16),
        DropdownButtonFormField<Map<String, dynamic>>(
          initialValue: _chat,
          decoration: const InputDecoration(
              labelText: 'Чат (куда инвайтим)', border: OutlineInputBorder()),
          items: [
            for (final c in _chats)
              DropdownMenuItem(
                value: c,
                child: Text('${c['title'] ?? c['id']} · ${c['members'] ?? 0} уч.'),
              ),
          ],
          onChanged: (v) => setState(() => _chat = v),
        ),
        const SizedBox(height: 12),
        DropdownButtonFormField<String>(
          initialValue: _database,
          decoration: const InputDecoration(
              labelText: 'База получателей', border: OutlineInputBorder()),
          items: [
            for (final d in _databases)
              DropdownMenuItem(
                value: d['name']?.toString(),
                child: Text('${d['name']} · ${d['entries'] ?? 0} записей'),
              ),
          ],
          onChanged: (v) => setState(() => _database = v),
        ),
        const SizedBox(height: 12),
        TextField(
          controller: _messageCtrl,
          maxLines: 3,
          decoration: const InputDecoration(
              labelText: 'Сообщение в чат (после инвайтов)',
              border: OutlineInputBorder()),
        ),
        const SizedBox(height: 12),
        Row(
          children: [
            Expanded(
              child: TextField(
                controller: _perAccountCtrl,
                decoration: const InputDecoration(
                    labelText: 'Инвайтов на аккаунт', border: OutlineInputBorder()),
              ),
            ),
            const SizedBox(width: 8),
            Expanded(
              child: TextField(
                controller: _pauseCtrl,
                decoration: const InputDecoration(
                    labelText: 'Пауза между сетами, мс',
                    border: OutlineInputBorder()),
              ),
            ),
          ],
        ),
        const SizedBox(height: 16),
        if (_progressLine != null) ...[
          Text(_progressLine!),
          const SizedBox(height: 12),
        ],
        if (_summary != null) ...[
          Card(
            child: Padding(
              padding: const EdgeInsets.all(12),
              child: Text(
                'Готово. Приглашено: ${_summary!['invited'] ?? 0}, '
                'ошибок: ${_summary!['failed'] ?? 0}'
                '${(_summary!['restricted_accounts'] as List?)?.isNotEmpty == true ? ', аккаунтов в карантине: ${(_summary!['restricted_accounts'] as List).length}' : ''}',
              ),
            ),
          ),
          const SizedBox(height: 12),
        ],
        Row(
          children: [
            FilledButton.icon(
              onPressed: _busy ? null : _start,
              icon: const Icon(Icons.send_outlined),
              label: const Text('Запустить инвайты'),
            ),
            const SizedBox(width: 8),
            OutlinedButton.icon(
              onPressed: _postMessage,
              icon: const Icon(Icons.post_add),
              label: const Text('Отправить сообщение'),
            ),
            const SizedBox(width: 8),
            OutlinedButton.icon(
              onPressed: _addChat,
              icon: const Icon(Icons.add_link),
              label: const Text('Чат'),
            ),
            const SizedBox(width: 8),
            IconButton(
              tooltip: 'Обновить списки',
              onPressed: () => setState(_reload),
              icon: const Icon(Icons.refresh),
            ),
          ],
        ),
      ],
    );
  }
}

/// Диалог добавления чата: существующий (@ссылка) или создание read-only канала.
class _AddChatDialog extends StatefulWidget {
  const _AddChatDialog();

  @override
  State<_AddChatDialog> createState() => _AddChatDialogState();
}

class _AddChatDialogState extends State<_AddChatDialog> {
  bool _create = false;
  bool _busy = false;
  String? _error;

  final _linkCtrl = TextEditingController();
  final _titleCtrl = TextEditingController();
  final _aboutCtrl = TextEditingController();

  @override
  void dispose() {
    _linkCtrl.dispose();
    _titleCtrl.dispose();
    _aboutCtrl.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    final res = _create
        ? await PiarCore.instance.callAsync('create_channel', {
            'title': _titleCtrl.text.trim(),
            'about': _aboutCtrl.text.trim(),
          })
        : await PiarCore.instance
            .callAsync('add_chat', {'link': _linkCtrl.text.trim()});
    if (!mounted) return;
    if (res.ok) {
      Navigator.pop(context, true);
    } else {
      setState(() {
        _busy = false;
        _error = res.errorText;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('Чат для инвайтов'),
      content: SizedBox(
        width: 440,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SegmentedButton<bool>(
              segments: const [
                ButtonSegment(
                  value: false,
                  icon: Icon(Icons.link),
                  label: Text('Существующий'),
                ),
                ButtonSegment(
                  value: true,
                  icon: Icon(Icons.add_circle_outline),
                  label: Text('Создать канал'),
                ),
              ],
              selected: {_create},
              onSelectionChanged: (s) => setState(() => _create = s.first),
            ),
            const SizedBox(height: 12),
            if (_create)
              Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  TextField(
                    controller: _titleCtrl,
                    autofocus: true,
                    decoration: const InputDecoration(
                      labelText: 'Название канала (read-only для подписчиков)',
                      border: OutlineInputBorder(),
                    ),
                  ),
                  const SizedBox(height: 8),
                  TextField(
                    controller: _aboutCtrl,
                    decoration: const InputDecoration(
                      labelText: 'Описание',
                      border: OutlineInputBorder(),
                    ),
                  ),
                ],
              )
            else
              TextField(
                controller: _linkCtrl,
                autofocus: true,
                decoration: const InputDecoration(
                  labelText: 'Ссылка на чат/канал',
                  hintText: '@mychannel или https://t.me/mychannel',
                  border: OutlineInputBorder(),
                ),
              ),
            if (_error != null) ...[
              const SizedBox(height: 8),
              Text(_error!, style: const TextStyle(color: Colors.redAccent)),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: _busy ? null : () => Navigator.pop(context, false),
          child: const Text('Отмена'),
        ),
        FilledButton(
          onPressed: _busy ? null : _submit,
          child: _busy
              ? const SizedBox(
                  width: 18,
                  height: 18,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : const Text('Добавить'),
        ),
      ],
    );
  }
}
