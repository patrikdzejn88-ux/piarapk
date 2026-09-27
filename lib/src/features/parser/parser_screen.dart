import 'dart:async';

import 'package:flutter/material.dart';

import '../../core/bridge.dart';

/// Раздел «Парсер»: сбор участников/авторов сообщений из чата
/// на отдельном пуле аккаунтов.
/// Протокол ядра: parse_start, list_databases.
class ParserScreen extends StatefulWidget {
  const ParserScreen({super.key});

  @override
  State<ParserScreen> createState() => _ParserScreenState();
}

enum _ParseMode { participants, messages }

class _ParserScreenState extends State<ParserScreen> {
  final _chatCtrl = TextEditingController();
  final _limitCtrl = TextEditingController(text: '1000');

  _ParseMode _mode = _ParseMode.participants;
  bool _busy = false;
  String? _progressLine;
  Map<String, dynamic>? _summary;
  StreamSubscription<PiarEvent>? _sub;

  List<Map<String, dynamic>> _databases = [];

  @override
  void initState() {
    super.initState();
    _reloadDatabases();
    _sub = PiarCore.instance.events.listen(_onEvent);
  }

  @override
  void dispose() {
    _sub?.cancel();
    _chatCtrl.dispose();
    _limitCtrl.dispose();
    super.dispose();
  }

  void _reloadDatabases() {
    final res = PiarCore.instance.call('list_databases', {});
    if (res.ok && res.data is List) {
      _databases = (res.data as List)
          .whereType<Map>()
          .map((m) => m.cast<String, dynamic>())
          .toList();
    }
    if (mounted) setState(() {});
  }

  void _onEvent(PiarEvent e) {
    if (e.method != 'parse_start') return;
    if (e.type == 'progress' && mounted) {
      final d = e.data;
      setState(() => _progressLine = 'Собрано: ${d?['done'] ?? 0}');
    }
    if (e.type == 'result' && mounted) {
      setState(() {
        _busy = false;
        _summary = e.ok && e.data is Map ? e.data as Map<String, dynamic> : null;
        _progressLine = e.ok ? null : 'Ошибка: ${e.error}';
      });
      _reloadDatabases();
    }
  }

  Future<void> _start() async {
    final chat = _chatCtrl.text.trim();
    if (chat.isEmpty) {
      _snack('Укажите ссылку на чат (@username или t.me/...)');
      return;
    }
    setState(() {
      _busy = true;
      _summary = null;
      _progressLine = 'Запуск…';
    });
    final res = await PiarCore.instance.callAsync('parse_start', {
      'chat': chat,
      'mode': _mode.name,
      'limit': int.tryParse(_limitCtrl.text.trim()) ?? 1000,
    });
    if (!res.ok && mounted) {
      setState(() {
        _busy = false;
        _progressLine = 'Ошибка: ${res.errorText}';
      });
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
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        Text('Парсер участников', style: Theme.of(context).textTheme.titleLarge),
        const SizedBox(height: 4),
        Text(
          'Работает на отдельном пуле аккаунтов («Парсер»), чтобы не жечь пиар-аккаунты.',
          style: Theme.of(context).textTheme.bodySmall,
        ),
        const SizedBox(height: 16),
        TextField(
          controller: _chatCtrl,
          decoration: const InputDecoration(
            labelText: 'Чат (ссылка или @username)',
            hintText: '@somechat / https://t.me/somechat',
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 12),
        SegmentedButton<_ParseMode>(
          segments: const [
            ButtonSegment(
              value: _ParseMode.participants,
              icon: Icon(Icons.groups_outlined),
              label: Text('Участники'),
            ),
            ButtonSegment(
              value: _ParseMode.messages,
              icon: Icon(Icons.chat_bubble_outline),
              label: Text('Авторы сообщений'),
            ),
          ],
          selected: {_mode},
          onSelectionChanged: (s) => setState(() => _mode = s.first),
        ),
        const SizedBox(height: 12),
        TextField(
          controller: _limitCtrl,
          decoration: const InputDecoration(
            labelText: 'Лимит (участников или последних сообщений)',
            border: OutlineInputBorder(),
          ),
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
                'Готово. Сохранено: ${_summary!['saved'] ?? 0}, '
                'пропущено (дубликаты/без username): ${_summary!['skipped'] ?? 0}'
                '${_summary!['base'] != null ? ' · база: ${_summary!['base']}' : ''}',
              ),
            ),
          ),
          const SizedBox(height: 12),
        ],
        Row(
          children: [
            FilledButton.icon(
              onPressed: _busy ? null : _start,
              icon: const Icon(Icons.play_arrow_outlined),
              label: const Text('Собрать'),
            ),
            const SizedBox(width: 8),
            IconButton(
              tooltip: 'Обновить базы',
              onPressed: _reloadDatabases,
              icon: const Icon(Icons.refresh),
            ),
          ],
        ),
        const SizedBox(height: 24),
        Text('Базы получателей', style: Theme.of(context).textTheme.titleMedium),
        const SizedBox(height: 8),
        if (_databases.isEmpty)
          const Text('Баз ещё нет — соберите первый чат.')
        else
          for (final d in _databases)
            ListTile(
              dense: true,
              leading: const Icon(Icons.storage_outlined),
              title: Text('${d['name']}'),
              subtitle: Text('${d['entries'] ?? 0} записей'),
            ),
      ],
    );
  }
}
