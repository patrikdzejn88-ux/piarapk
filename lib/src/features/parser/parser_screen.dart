import 'dart:async';

import 'package:flutter/material.dart';

import '../../core/bridge.dart';
import '../../core/native.dart';
import '../../ui/theme.dart';

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

  bool _parsing = false;
  bool _chatsLoading = false;
  String? _progressLine;
  Map<String, dynamic>? _summary;
  StreamSubscription<PiarEvent>? _sub;

  List<Map<String, dynamic>> _databases = [];
  List<Map<String, dynamic>> _accountChats = [];
  Map<String, dynamic>? _selectedChat;

  @override
  void initState() {
    super.initState();
    unawaited(_reloadDatabases());
    _sub = PiarCore.instance.events.listen(_onEvent);
    unawaited(PiarCore.instance.init().then((_) {
      if (mounted) unawaited(_reloadDatabases());
    }));
  }

  @override
  void dispose() {
    unawaited(_sub?.cancel());
    _chatCtrl.dispose();
    _limitCtrl.dispose();
    super.dispose();
  }

  Future<void> _reloadDatabases() async {
    // B.2.8: файловый IO — через async-путь, чтобы не блокировать UI-изолят
    final res = await PiarCore.instance.callAsync('list_databases', {});
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
      final phase = d?['phase'] == 'resolve' ? 'резолв' : 'сообщений';
      setState(() => _progressLine = 'Обработано $phase: ${d?['done'] ?? 0}');
    }
    if (e.type == 'result' && mounted) {
      setState(() {
        _parsing = false;
        _summary = e.ok && e.data is Map ? e.data as Map<String, dynamic> : null;
        _progressLine = e.ok ? null : 'Ошибка: ${e.error}';
      });
      // сервис останавливает глобальный слушатель (app.dart) — работает
      // даже если этот экран уже закрыт
      unawaited(_reloadDatabases());
    }
  }

  Future<void> _start() async {
    final manual = _chatCtrl.text.trim();
    if (_selectedChat == null && manual.isEmpty) {
      _snack('Выбери чат аккаунта или укажи ссылку вручную');
      return;
    }
    setState(() {
      _parsing = true;
      _summary = null;
      _progressLine = 'Запуск…';
    });
    // держим процесс живым в фоне, пока идёт сбор
    final chatTitle =
        _selectedChat?['title']?.toString() ?? _chatCtrl.text.trim();
    await Native.parserServiceStart('Сбор базы: $chatTitle');
    final res = await PiarCore.instance.callAsync('parse_start', _selectedChat != null
        ? {
            'dialog_id': _selectedChat!['id'],
            'access_hash': _selectedChat!['access_hash'],
            'title': _selectedChat!['title'] ?? '',
            'limit': int.tryParse(_limitCtrl.text.trim()) ?? 30000,
          }
        : {
            'chat': manual,
            'limit': int.tryParse(_limitCtrl.text.trim()) ?? 30000,
          });
    if (!res.ok && mounted) {
      setState(() {
        _parsing = false;
        _progressLine = 'Ошибка: ${res.errorText}';
      });
      unawaited(Native.parserServiceStop());
    }
  }

  Future<void> _loadAccountChats() async {
    setState(() => _chatsLoading = true);
    final res = await PiarCore.instance.callAsync('list_account_chats', {});
    if (!mounted) return;
    if (res.ok && res.data is List) {
      setState(() {
        _accountChats = (res.data as List)
            .whereType<Map>()
            .map((m) => m.cast<String, dynamic>())
            .toList();
        // выбранный чат после перезагрузки списка — заново сматчить по id
        // (новые Map-инстанцы не совпадают по == со старым _selectedChat)
        final selId = _selectedChat?['id'];
        final matched =
            _accountChats.where((c) => c['id'] == selId).toList();
        _selectedChat = matched.isEmpty ? null : matched.first;
        _chatsLoading = false;
        if (_accountChats.isEmpty) {
          _progressLine = 'На аккаунте парсера нет чатов';
        }
      });
    } else {
      setState(() {
        _chatsLoading = false;
        _progressLine = 'Ошибка: ${res.errorText}';
      });
    }
  }

  Future<void> _downloadBase(Map<String, dynamic> db) async {
    final name = db['name']?.toString() ?? 'base';
    // B.2.8: чтение файла — через async-путь, чтобы не блокировать UI-изолят
    final res = await PiarCore.instance.callAsync('get_database', {'name': name});
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

  /// Просмотр базы (чтение + удаление людей по одному крестиком).
  Future<void> _viewBase(Map<String, dynamic> db) async {
    final name = db['name']?.toString() ?? '';
    // B.2.8: чтение файла — через async-путь, чтобы не блокировать UI-изолят
    final res = await PiarCore.instance.callAsync('get_database', {'name': name});
    if (!res.ok) {
      _snack('Ошибка чтения: ${res.errorText}');
      return;
    }
    final content =
        res.data is Map ? (res.data as Map)['content']?.toString() ?? '' : '';
    final lines =
        content.split('\n').where((l) => l.trim().isNotEmpty).toList();
    if (!mounted) return;
    await showDialog<void>(
      context: context,
      builder: (dialogContext) => StatefulBuilder(
        builder: (dialogContext, setDialogState) => AlertDialog(
          title: Text('База «$name» · ${lines.length} чел.'),
          content: SizedBox(
            width: 440,
            height: 460,
            child: lines.isEmpty
                ? const Center(child: Text('Пусто'))
                : ListView.builder(
                    itemCount: lines.length,
                    itemBuilder: (context, i) => Row(
                      children: [
                        Expanded(
                          child: Text(
                            '@${lines[i]}',
                            style: const TextStyle(
                                fontFamily: 'monospace', fontSize: 12),
                          ),
                        ),
                        IconButton(
                          tooltip: 'Убрать',
                          visualDensity: VisualDensity.compact,
                          icon: const Icon(Icons.close, size: 16),
                          onPressed: () async {
                            final r = await PiarCore.instance.callAsync(
                                'remove_from_database',
                                {'name': name, 'usernames': lines[i]});
                            if (!r.ok) {
                              _snack('Ошибка: ${r.errorText}');
                              return;
                            }
                            if (!dialogContext.mounted) return;
                            setDialogState(() => lines.removeAt(i));
                            unawaited(_reloadDatabases());
                          },
                        ),
                      ],
                    ),
                  ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(dialogContext),
              child: const Text('Закрыть'),
            ),
          ],
        ),
      ),
    );
  }

  /// Добавить людей в базу вручную.
  Future<void> _addPeople(Map<String, dynamic> db) async {
    final name = db['name']?.toString() ?? '';
    final ctrl = TextEditingController();
    final ok = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: Text('Добавить в «$name»'),
        content: SizedBox(
          width: 440,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Text('По одному username в строке (можно с @ или без):',
                  style: TextStyle(fontSize: 12)),
              const SizedBox(height: 8),
              TextField(
                controller: ctrl,
                maxLines: 6,
                decoration: const InputDecoration(
                    hintText: 'user1\nuser2\n@user3',
                    border: OutlineInputBorder()),
              ),
            ],
          ),
        ),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(dialogContext, false),
              child: const Text('Отмена')),
          FilledButton(
              onPressed: () => Navigator.pop(dialogContext, true),
              child: const Text('Добавить')),
        ],
      ),
    );
    final usernames = ctrl.text;
    ctrl.dispose();
    if (ok != true) return;
    final res = await PiarCore.instance
        .callAsync('add_to_database', {'name': name, 'usernames': usernames});
    if (!res.ok) {
      _snack('Ошибка: ${res.errorText}');
    }
    unawaited(_reloadDatabases());
  }

  /// Создать новую базу (имя + опционально первый список людей).
  Future<void> _createBase() async {
    final nameCtrl = TextEditingController();
    final peopleCtrl = TextEditingController();
    final ok = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: const Text('Новая база'),
        content: SizedBox(
          width: 440,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              TextField(
                controller: nameCtrl,
                autofocus: true,
                decoration: const InputDecoration(
                    labelText: 'Имя базы (латиницей/цифрами)',
                    border: OutlineInputBorder()),
              ),
              const SizedBox(height: 8),
              TextField(
                controller: peopleCtrl,
                maxLines: 4,
                decoration: const InputDecoration(
                    labelText: 'Люди (по одному в строке, необязательно)',
                    border: OutlineInputBorder()),
              ),
            ],
          ),
        ),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(dialogContext, false),
              child: const Text('Отмена')),
          FilledButton(
              onPressed: () => Navigator.pop(dialogContext, true),
              child: const Text('Создать')),
        ],
      ),
    );
    final name = nameCtrl.text.trim();
    final people = peopleCtrl.text;
    nameCtrl.dispose();
    peopleCtrl.dispose();
    if (ok != true || name.isEmpty) return;
    final res = PiarCore.instance
        .call('create_database', {'name': name, 'usernames': people});
    _snack(res.ok ? 'База «$name» создана' : 'Ошибка: ${res.errorText}');
    unawaited(_reloadDatabases());
  }

  /// Убрать людей из базы (списком, как добавление).
  Future<void> _removePeople(Map<String, dynamic> db) async {
    final name = db['name']?.toString() ?? '';
    final ctrl = TextEditingController();
    final ok = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: Text('Убрать из «$name»'),
        content: SizedBox(
          width: 440,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Text('По одному username в строке (можно с @ или без):',
                  style: TextStyle(fontSize: 12)),
              const SizedBox(height: 8),
              TextField(
                controller: ctrl,
                maxLines: 6,
                decoration: const InputDecoration(
                    hintText: 'user1\nuser2\n@user3',
                    border: OutlineInputBorder()),
              ),
            ],
          ),
        ),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(dialogContext, false),
              child: const Text('Отмена')),
          FilledButton(
              onPressed: () => Navigator.pop(dialogContext, true),
              child: const Text('Убрать')),
        ],
      ),
    );
    final usernames = ctrl.text;
    ctrl.dispose();
    if (ok != true) return;
    final res = await PiarCore.instance
        .callAsync('remove_from_database', {'name': name, 'usernames': usernames});
    if (!res.ok) {
      _snack('Ошибка: ${res.errorText}');
    }
    unawaited(_reloadDatabases());
  }

  /// Удаление базы с подтверждением.
  Future<void> _deleteBase(Map<String, dynamic> db) async {
    final name = db['name']?.toString() ?? '';
    final ok = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => AlertDialog(
        title: Text('Удалить базу «$name»?'),
        content: const Text('Файл базы будет удалён с устройства безвозвратно.'),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(dialogContext, false),
              child: const Text('Отмена')),
          FilledButton(
              style: FilledButton.styleFrom(
                  backgroundColor: Colors.redAccent),
              onPressed: () => Navigator.pop(dialogContext, true),
              child: const Text('Удалить')),
        ],
      ),
    );
    if (ok != true) return;
    final res = await PiarCore.instance.callAsync('delete_database', {'name': name});
    _snack(res.ok ? 'База «$name» удалена' : 'Ошибка: ${res.errorText}');
    unawaited(_reloadDatabases());
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
              GradientText('Парсер участников',
                  style: Theme.of(context).textTheme.headlineSmall),
              const SizedBox(height: 4),
              Text(
                'Собирает авторов последних сообщений чата. Работает на отдельном пуле «Парсер», чтобы не жечь пиар-аккаунты.',
                style: Theme.of(context).textTheme.bodySmall,
                textAlign: TextAlign.center,
              ),
              const SizedBox(height: 20),
              // чаты, которые уже есть на аккаунте парсера
              Row(
                children: [
                  Expanded(
                    child: Text('Чат аккаунта:',
                        style: Theme.of(context).textTheme.bodyMedium),
                  ),
                  TextButton.icon(
                    onPressed: _chatsLoading ? null : _loadAccountChats,
                    icon: _chatsLoading
                        ? const SizedBox(
                            width: 14,
                            height: 14,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : const Icon(Icons.refresh, size: 18),
                    label: const Text('Загрузить'),
                  ),
                ],
              ),
              DropdownButtonFormField<Map<String, dynamic>>(
                key: ValueKey(_selectedChat?['id']),
                initialValue: _selectedChat,
                isExpanded: true,
                decoration: const InputDecoration(
                    hintText: 'выбрать из чатов аккаунта',
                    border: OutlineInputBorder()),
                items: [
                  for (final c in _accountChats)
                    DropdownMenuItem(
                      value: c,
                      child: Text(
                        '${c['title'] ?? ''}',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                ],
                onChanged: (v) => setState(() {
                  _selectedChat = v;
                  if (v != null) _chatCtrl.clear();
                }),
              ),
              const SizedBox(height: 4),
              Text('…или ссылка вручную:',
                  style: Theme.of(context).textTheme.bodySmall),
              const SizedBox(height: 4),
              TextField(
                controller: _chatCtrl,
                decoration: const InputDecoration(
                  labelText: '@username / https://t.me/somechat',
                  border: OutlineInputBorder(),
                ),
                onChanged: (_) {
                  if (_selectedChat != null) {
                    setState(() => _selectedChat = null);
                  }
                },
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
              Row(
                children: [
                  Expanded(
                    child: GradientButton(
                      onPressed: _start,
                      busy: _parsing,
                      label: _parsing ? 'Собираем…' : 'Собрать',
                      icon: const Icon(Icons.play_arrow_outlined),
                    ),
                  ),
                  if (_parsing) ...[
                    const SizedBox(width: 8),
                    OutlinedButton.icon(
                      onPressed: () async {
                        final r = await PiarCore.instance.callAsync('parse_cancel', {});
                        _snack(r.ok
                            ? 'Останавливаю… (собранное сохранится)'
                            : 'Не удалось остановить: ${r.errorText}');
                      },
                      icon: const Icon(Icons.stop_circle_outlined),
                      label: const Text('Стоп'),
                    ),
                  ],
                ],
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
                      '${(_summary!['truncated'] == true || _summary!['partial'] == true) ? '⚠ База неполная (сбор прерван или ошибка сети) — повторите сбор. ' : ''}'
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
                    tooltip: 'Создать новую базу',
                    icon: const Icon(Icons.create_new_folder_outlined),
                    onPressed: _createBase,
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
                    trailing: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        IconButton(
                          tooltip: 'Просмотр',
                          icon: const Icon(Icons.visibility_outlined),
                          onPressed: () => _viewBase(d),
                        ),
                        IconButton(
                          tooltip: 'Добавить людей вручную',
                          icon: const Icon(Icons.person_add_alt_outlined),
                          onPressed: () => _addPeople(d),
                        ),
                        IconButton(
                          tooltip: 'Убрать людей',
                          icon: const Icon(Icons.person_remove_outlined),
                          onPressed: () => _removePeople(d),
                        ),
                        IconButton(
                          tooltip: 'Скачать на устройство',
                          icon: const Icon(Icons.download_outlined),
                          onPressed:
                              Native.supported ? () => _downloadBase(d) : null,
                        ),
                        IconButton(
                          tooltip: 'Удалить базу',
                          icon: const Icon(Icons.delete_outline),
                          onPressed: () => _deleteBase(d),
                        ),
                      ],
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
