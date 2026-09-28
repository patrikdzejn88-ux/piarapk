import 'dart:async';

import 'package:flutter/material.dart';

import '../../core/bridge.dart';
import '../../core/native.dart';

/// Раздел «Парсер»: сбор участников чата по авторам последних сообщений.
/// Работает на ОТДЕЛЬНОМ пуле аккаунтов («Парсер»).
class ParserScreen extends StatefulWidget {
  const ParserScreen({super.key});

  @override
  State<ParserScreen> createState() => _ParserScreenState();
}

class _ParserScreenState extends State<ParserScreen> {
  final _chatCtrl = TextEditingController();
  final _limitCtrl = TextEditingController(text: '30000');

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
    PiarCore.instance.init().then((_) {
      if (mounted) setState(_reloadDatabases);
    });
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
      setState(() => _progressLine = 'Обработано сообщений: ${d?['done'] ?? 0}');
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
      'limit': int.tryParse(_limitCtrl.text.trim()) ?? 10000,
    });
    if (!res.ok && mounted) {
      setState(() {
        _busy = false;
        _progressLine = 'Ошибка: ${res.errorText}';
      });
    }
  }

  Future<void> _downloadBase(Map<String, dynamic> db) async {
    final name = db['name']?.toString() ?? 'base';
    final res = PiarCore.instance.call('get_database', {'name': name});
    if (!res.ok) {
      _snack('Ошибка чтения базы: ${res.errorText}');
      return;
    }
    final content = res.data is Map ? (res.data as Map)['content']?.toString() : null;
    if (content == null) {
      _snack('База пуста');
      return;
    }
    final where = await Native.exportToDownloads('$name.txt', content);
    _snack(where != null
        ? 'Сохранено: $where'
        : 'Не удалось сохранить в «Загрузки»');
  }

  void _snack(String text) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  @override
  Widget build(BuildContext context) {
    if (!PiarCore.instance.available) {
      return Center(
        child: Text('Ядро не загружено — раздел недоступен'
            '${PiarCore.instance.lastError != null ? '\n${PiarCore.instance.lastError}' : ''}'),
      );
    }
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 560),
        child: SingleChildScrollView(
          padding: const EdgeInsets.all(20),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text('Парсер участников',
                  style: Theme.of(context).textTheme.titleLarge,
                  textAlign: TextAlign.center),
              const SizedBox(height: 4),
              Text(
                'Собирает авторов последних сообщений чата. Работает на отдельном пуле «Парсер», чтобы не жечь пиар-аккаунты.',
                style: Theme.of(context).textTheme.bodySmall,
                textAlign: TextAlign.center,
              ),
              const SizedBox(height: 20),
              TextField(
                controller: _chatCtrl,
                decoration: const InputDecoration(
                  labelText: 'Чат (ссылка или @username)',
                  hintText: '@somechat / https://t.me/somechat',
                  border: OutlineInputBorder(),
                ),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _limitCtrl,
                keyboardType: TextInputType.number,
                decoration: const InputDecoration(
                  labelText: 'Сколько последних сообщений обработать',
                  border: OutlineInputBorder(),
                ),
              ),
              const SizedBox(height: 16),
              FilledButton.icon(
                onPressed: _busy ? null : _start,
                icon: _busy
                    ? const SizedBox(
                        width: 18,
                        height: 18,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.play_arrow_outlined),
                label: Text(_busy ? 'Собираем…' : 'Собрать'),
              ),
              if (_progressLine != null) ...[
                const SizedBox(height: 12),
                Text(_progressLine!, textAlign: TextAlign.center),
              ],
              if (_summary != null) ...[
                const SizedBox(height: 12),
                Card(
                  child: Padding(
                    padding: const EdgeInsets.all(12),
                    child: Text(
                      'Готово. Уникальных сохранено: ${_summary!['saved'] ?? 0}, '
                      'пропущено (без username/дубли): ${_summary!['skipped'] ?? 0}'
                      '${_summary!['base'] != null ? ' · база: ${_summary!['base']}' : ''}',
                      textAlign: TextAlign.center,
                    ),
                  ),
                ),
              ],
              const SizedBox(height: 32),
              Row(
                children: [
                  Expanded(
                    child: Text('Базы (доступны в разделе «Пиар»)',
                        style: Theme.of(context).textTheme.titleMedium),
                  ),
                  IconButton(
                    tooltip: 'Обновить',
                    onPressed: _reloadDatabases,
                    icon: const Icon(Icons.refresh),
                  ),
                ],
              ),
              const SizedBox(height: 4),
              if (_databases.isEmpty)
                const Padding(
                  padding: EdgeInsets.symmetric(vertical: 12),
                  child: Text('Баз ещё нет — соберите первый чат.',
                      textAlign: TextAlign.center),
                )
              else
                ..._databases.map(
                  (d) => ListTile(
                    dense: true,
                    contentPadding: const EdgeInsets.symmetric(horizontal: 4),
                    leading: const Icon(Icons.storage_outlined),
                    title: Text('${d['name']}'),
                    subtitle: Text('${d['entries'] ?? 0} записей'),
                    trailing: IconButton(
                      tooltip: 'Скачать на устройство',
                      icon: const Icon(Icons.download_outlined),
                      onPressed: () => _downloadBase(d),
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}
