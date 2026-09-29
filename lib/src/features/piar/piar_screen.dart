import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';

import '../../core/bridge.dart';
import '../../core/native.dart';
import '../../ui/theme.dart';

/// Раздел «Пиар»: ОДИН аккаунт (владелец чата) добавляет людей из базы
/// в чат и отправляет сообщение. Чат — read-only для участников.
class PiarScreen extends StatefulWidget {
  const PiarScreen({super.key});

  @override
  State<PiarScreen> createState() => _PiarScreenState();
}

class _PiarScreenState extends State<PiarScreen> {
  Map<String, dynamic>? _chat;
  String? _database;
  final _messageCtrl = TextEditingController();
  final _countCtrl = TextEditingController(text: '0');
  String? _imagePath;
  String? _imageName;

  bool _busy = false;
  String? _progressLine;
  Map<String, dynamic>? _summary;
  List<String> _inviteLog = [];
  StreamSubscription<PiarEvent>? _sub;

  List<Map<String, dynamic>> _chats = [];
  List<Map<String, dynamic>> _databases = [];

  @override
  void initState() {
    super.initState();
    _reload();
    _sub = PiarCore.instance.events.listen(_onEvent);
    PiarCore.instance.init().then((_) {
      if (mounted) setState(_reload);
    });
  }

  @override
  void dispose() {
    _sub?.cancel();
    _messageCtrl.dispose();
    _countCtrl.dispose();
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
    if (e.method != 'invite_start') return;
    if (e.type == 'progress' && mounted) {
      final d = e.data;
      setState(() {
        _progressLine =
            'Добавлено: ${d?['done'] ?? 0} · ошибок: ${d?['failed'] ?? 0}'
                '${d?['note'] != null ? ' · ${d!['note']}' : ''}';
        _inviteLog = _logFrom(d);
      });
    }
    if (e.type == 'result' && mounted) {
      setState(() {
        _busy = false;
        _summary = e.ok && e.data is Map ? e.data as Map<String, dynamic> : null;
        _progressLine = e.ok ? null : 'Ошибка: ${e.error}';
        if (e.ok) {
          _inviteLog = _logFrom(e.data);
        } else {
          final err = e.error?.toString() ?? '';
          if (err.isNotEmpty) {
            _inviteLog = [..._inviteLog, err];
          }
        }
      });
      _reload();
    }
  }

  List<String> _logFrom(dynamic data) {
    if (data is Map && data['log'] is List) {
      return (data['log'] as List).map((e) => e.toString()).toList();
    }
    return _inviteLog;
  }

  Future<void> _start() async {
    if (_chat == null || _database == null) {
      _snack('Выберите чат и базу получателей (сначала раздел «Парсер»)');
      return;
    }
    setState(() {
      _busy = true;
      _summary = null;
      _inviteLog = [];
      _progressLine = 'Запуск…';
    });
    final res = await PiarCore.instance.callAsync('invite_start', {
      'chat_id': _chat!['id'],
      'database': _database,
      'message': _messageCtrl.text.trim(),
      'image_path': _imagePath ?? '',
      'count': int.tryParse(_countCtrl.text.trim()) ?? 0,
    });
    if (!res.ok && mounted) {
      setState(() {
        _busy = false;
        _progressLine = 'Ошибка: ${res.errorText}';
        _inviteLog = [..._inviteLog, res.errorText];
      });
    }
  }

  Future<void> _postMessage() async {
    if (_chat == null) {
      _snack('Выберите чат');
      return;
    }
    final res = await PiarCore.instance.callAsync('post_message', {
      'chat_id': _chat!['id'],
      'text': _messageCtrl.text.trim(),
      'image_path': _imagePath ?? '',
    });
    _snack(res.ok ? 'Сообщение отправлено' : 'Ошибка: ${res.errorText}');
  }

  Future<void> _pickImage() async {
    final path = await Native.pickImage();
    if (!mounted) return;
    if (path == null) {
      _snack('Картинка не выбрана');
      return;
    }
    setState(() {
      _imagePath = path;
      _imageName = path.split(Platform.pathSeparator).last;
    });
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
              GradientText('Пиар: инвайты в чат',
                  style: Theme.of(context).textTheme.headlineSmall),
              const SizedBox(height: 4),
              Text(
                'Работает ОДИН аккаунт из пула «Пиар» — он же владелец чата. '
                'Люди из базы добавляются в чат, затем отправляется сообщение.',
                style: Theme.of(context).textTheme.bodySmall,
                textAlign: TextAlign.center,
              ),
              const SizedBox(height: 20),
              DropdownButtonFormField<Map<String, dynamic>>(
                initialValue: _chat,
                decoration: const InputDecoration(
                    labelText: 'Чат (куда добавляем людей)',
                    border: OutlineInputBorder()),
                items: [
                  for (final c in _chats)
                    DropdownMenuItem(
                      value: c,
                      child: Text(
                          '${c['title'] ?? c['id']} · ${c['members'] ?? 0} уч.'),
                    ),
                ],
                onChanged: (v) => setState(() => _chat = v),
              ),
              Align(
                alignment: Alignment.centerRight,
                child: TextButton.icon(
                  onPressed: _addChat,
                  icon: const Icon(Icons.add_link, size: 18),
                  label: const Text('Добавить чат / создать read-only канал'),
                ),
              ),
              const SizedBox(height: 8),
              DropdownButtonFormField<String>(
                initialValue: _database,
                decoration: const InputDecoration(
                    labelText: 'База людей (собирается в разделе «Парсер»)',
                    border: OutlineInputBorder()),
                items: [
                  for (final d in _databases)
                    DropdownMenuItem(
                      value: d['name']?.toString(),
                      child: Text('${d['name']} · ${d['entries'] ?? 0} чел.'),
                    ),
                ],
                onChanged: (v) => setState(() => _database = v),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _countCtrl,
                keyboardType: TextInputType.number,
                decoration: const InputDecoration(
                  labelText: 'Сколько людей из базы добавить (0 = все)',
                  border: OutlineInputBorder(),
                ),
              ),
              const SizedBox(height: 12),
              TextField(
                controller: _messageCtrl,
                maxLines: 3,
                decoration: const InputDecoration(
                    labelText: 'Сообщение (текст к картинке / обычное сообщение)',
                    border: OutlineInputBorder()),
              ),
              const SizedBox(height: 8),
              // картинка к сообщению (фото + подпись)
              Row(
                children: [
                  OutlinedButton.icon(
                    onPressed: _pickImage,
                    icon: const Icon(Icons.image_outlined),
                    label: Text(_imagePath == null
                        ? 'Картинка'
                        : _imageName ?? 'выбрана'),
                  ),
                  if (_imagePath != null) ...[
                    const SizedBox(width: 8),
                    IconButton(
                      tooltip: 'Убрать картинку',
                      icon: const Icon(Icons.close),
                      onPressed: () =>
                          setState(() { _imagePath = null; _imageName = null; }),
                    ),
                  ],
                ],
              ),
              const SizedBox(height: 16),
              GradientButton(
                onPressed: _busy ? null : _start,
                busy: _busy,
                label: _busy ? 'Работаем…' : 'Запустить',
                icon: const Icon(Icons.send_outlined),
              ),
              const SizedBox(height: 8),
              OutlinedButton.icon(
                onPressed: _postMessage,
                icon: const Icon(Icons.post_add),
                label: const Text('Отправить сообщение'),
              ),
              const SizedBox(height: 8),
              IconButton(
                tooltip: 'Обновить списки',
                onPressed: () => setState(_reload),
                icon: const Icon(Icons.refresh),
              ),
              if (_progressLine != null) ...[
                const SizedBox(height: 8),
                Text(_progressLine!, textAlign: TextAlign.center),
              ],
              if (_inviteLog.isNotEmpty) ...[
                const SizedBox(height: 8),
                Container(
                  constraints: const BoxConstraints(maxHeight: 140),
                  padding:
                      const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
                  decoration: BoxDecoration(
                    color: Colors.black26,
                    borderRadius: BorderRadius.circular(8),
                  ),
                  child: ListView.builder(
                    shrinkWrap: true,
                    itemCount: _inviteLog.length,
                    itemBuilder: (_, i) => Text(
                      _inviteLog[i],
                      style: const TextStyle(
                        fontSize: 11,
                        fontFamily: 'monospace',
                        height: 1.35,
                      ),
                    ),
                  ),
                ),
              ],
              if (_summary != null) ...[
                const SizedBox(height: 8),
                Card(
                  child: Padding(
                    padding: const EdgeInsets.all(12),
                    child: Text(
                      'Готово. Добавлено: ${_summary!['invited'] ?? 0}, '
                      'ошибок: ${_summary!['failed'] ?? 0}'
                      '${_summary!['remaining'] != null ? ', осталось: ${_summary!['remaining']}' : ''}'
                      '${_summary!['stopped_reason'] != null ? '\nОСТАНОВЛЕНО: ${_summary!['stopped_reason']}' : ''}'
                      '${_summary!['message_sent'] == true ? '\nСообщение отправлено' : ''}',
                      textAlign: TextAlign.center,
                    ),
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// Диалог добавления чата: с аккаунта / существующий (@ссылка) / создать канал.
class _AddChatDialog extends StatefulWidget {
  const _AddChatDialog();

  @override
  State<_AddChatDialog> createState() => _AddChatDialogState();
}

enum _ChatMode { account, link, create }

class _AddChatDialogState extends State<_AddChatDialog> {
  _ChatMode _mode = _ChatMode.account;
  bool _busy = false;
  String? _error;

  bool _chatsLoading = false;
  List<Map<String, dynamic>> _accountChats = [];
  Map<String, dynamic>? _selected;

  final _linkCtrl = TextEditingController();
  final _titleCtrl = TextEditingController();
  final _aboutCtrl = TextEditingController();

  @override
  void initState() {
    super.initState();
    _loadAccountChats();
  }

  @override
  void dispose() {
    _linkCtrl.dispose();
    _titleCtrl.dispose();
    _aboutCtrl.dispose();
    super.dispose();
  }

  Future<void> _loadAccountChats() async {
    setState(() => _chatsLoading = true);
    final res = await PiarCore.instance
        .callAsync('list_account_chats', {'pool': 'piar'});
    if (!mounted) return;
    if (res.ok && res.data is List) {
      setState(() {
        _accountChats = (res.data as List)
            .whereType<Map>()
            .map((m) => m.cast<String, dynamic>())
            .toList();
        _chatsLoading = false;
        // перевыбор по id после перезагрузки
        final selId = _selected?['id'];
        final matched =
            _accountChats.where((c) => c['id'] == selId).toList();
        _selected = matched.isEmpty ? null : matched.first;
      });
    } else {
      setState(() {
        _chatsLoading = false;
        _error = res.errorText;
      });
    }
  }

  Future<void> _submit() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    final res = switch (_mode) {
      _ChatMode.account => _selected != null
          ? await PiarCore.instance.callAsync('add_chat_from_dialog', {
              'dialog_id': _selected!['id'],
              'access_hash': _selected!['access_hash'],
              'title': _selected!['title'] ?? '',
            })
          : null,
      _ChatMode.create => await PiarCore.instance.callAsync('create_channel', {
          'title': _titleCtrl.text.trim(),
          'about': _aboutCtrl.text.trim(),
        }),
      _ChatMode.link => await PiarCore.instance
          .callAsync('add_chat', {'link': _linkCtrl.text.trim()}),
    };
    if (!mounted) return;
    if (res == null) {
      setState(() {
        _busy = false;
        _error = 'Выберите чат из списка';
      });
      return;
    }
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
        width: 460,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SegmentedButton<_ChatMode>(
              segments: const [
                ButtonSegment(
                  value: _ChatMode.account,
                  icon: Icon(Icons.chat_bubble_outline),
                  label: Text('С аккаунта'),
                ),
                ButtonSegment(
                  value: _ChatMode.link,
                  icon: Icon(Icons.link),
                  label: Text('По ссылке'),
                ),
                ButtonSegment(
                  value: _ChatMode.create,
                  icon: Icon(Icons.add_circle_outline),
                  label: Text('Создать'),
                ),
              ],
              selected: {_mode},
              onSelectionChanged: (s) => setState(() => _mode = s.first),
            ),
            const SizedBox(height: 12),
            switch (_mode) {
              _ChatMode.account => _chatsLoading
                  ? const Center(child: Padding(
                      padding: EdgeInsets.all(12),
                      child: CircularProgressIndicator()))
                  : _accountChats.isEmpty
                      ? const Padding(
                          padding: EdgeInsets.symmetric(vertical: 8),
                          child: Text(
                            'На аккаунте «Пиар» нет чатов — добавь по ссылке или создай канал',
                            textAlign: TextAlign.center,
                            style: TextStyle(fontSize: 12),
                          ),
                        )
                      : DropdownButtonFormField<Map<String, dynamic>>(
                          initialValue: _selected,
                          isExpanded: true,
                          decoration: const InputDecoration(
                              hintText: 'выбрать чат аккаунта',
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
                          onChanged: (v) => setState(() => _selected = v),
                        ),
              _ChatMode.create => Column(
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
                ),
              _ChatMode.link => TextField(
                  controller: _linkCtrl,
                  autofocus: true,
                  decoration: const InputDecoration(
                    labelText: 'Ссылка на чат/канал',
                    hintText: '@mychannel или https://t.me/mychannel',
                    border: OutlineInputBorder(),
                  ),
                ),
            },
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
