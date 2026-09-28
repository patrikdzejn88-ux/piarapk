import 'dart:async';

import 'package:flutter/material.dart';

import '../../core/bridge.dart';
import 'accounts_controller.dart';

/// Диалог-визард добавления аккаунта:
/// [По номеру] телефон → код → (2FA) | [StringSession] | [tdata].
class AddAccountDialog extends StatefulWidget {
  const AddAccountDialog({
    super.key,
    required this.controller,
    required this.pool,
  });

  final AccountsController controller;
  final String pool;

  @override
  State<AddAccountDialog> createState() => _AddAccountDialogState();
}

enum _Step { choose, phone, code, password, session, tdata }

class _AddAccountDialogState extends State<AddAccountDialog> {
  _Step _step = _Step.choose;
  bool _busy = false;
  String? _error;

  /// обновление хвоста лога под спиннером, пока идёт операция
  Timer? _busyLogTimer;

  final _phoneCtrl = TextEditingController();
  final _codeCtrl = TextEditingController();
  final _passwordCtrl = TextEditingController();
  final _sessionCtrl = TextEditingController();
  final _apiIdCtrl = TextEditingController();
  final _apiHashCtrl = TextEditingController();
  final _tdataPathCtrl = TextEditingController();

  @override
  void dispose() {
    _busyLogTimer?.cancel();
    for (final c in [
      _phoneCtrl,
      _codeCtrl,
      _passwordCtrl,
      _sessionCtrl,
      _apiIdCtrl,
      _apiHashCtrl,
      _tdataPathCtrl,
    ]) {
      c.dispose();
    }
    super.dispose();
  }

  String get _poolLabel => widget.pool == 'parser' ? 'парсер' : 'пиар';

  Future<void> _run(Future<void> Function() action) async {
    setState(() {
      _busy = true;
      _error = null;
    });
    _busyLogTimer?.cancel();
    _busyLogTimer = Timer.periodic(const Duration(seconds: 1), (_) {
      if (mounted && _busy) setState(() {});
    });
    try {
      await action();
    } catch (e) {
      if (mounted) {
        setState(() => _error = e.toString());
      }
    } finally {
      _busyLogTimer?.cancel();
      if (mounted) {
        setState(() => _busy = false);
      }
    }
  }

  void _fail(String message) {
    if (!mounted) return;
    setState(() {
      _busy = false;
      _error = message;
    });
  }

  /// Последние строки лога ядра для отображения под спиннером.
  List<Widget> _recentLogs() {
    final logs = PiarCore.instance.lastLogs;
    if (logs.isEmpty) {
      return const [SizedBox.shrink()];
    }
    final tail = logs.length <= 4 ? logs : logs.sublist(logs.length - 4);
    return [
      for (final line in tail)
        Text(
          line,
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          style: const TextStyle(fontFamily: 'monospace', fontSize: 10),
        ),
      const SizedBox(height: 4),
      const Text('полный лог — кнопка «Лог ядра» в разделе Аккаунты',
          textAlign: TextAlign.center,
          style: TextStyle(fontSize: 10)),
    ];
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text('Добавить аккаунт ($_poolLabel пул)'),
      content: SizedBox(
        width: 460,
        child: _busy
            ? Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const CircularProgressIndicator(),
                  const SizedBox(height: 16),
                  const Text('Выполняется…',
                      textAlign: TextAlign.center),
                  const SizedBox(height: 12),
                  // хвост лога ядра: диагностика прямо под спиннером
                  ..._recentLogs(),
                ],
              )
            : Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  if (_error != null) ...[
                    Text(
                      _error!,
                      style: const TextStyle(color: Colors.redAccent),
                    ),
                    const SizedBox(height: 12),
                  ],
                  switch (_step) {
                    _Step.choose => _choose(),
                    _Step.phone => _phoneForm(),
                    _Step.code => _codeForm(),
                    _Step.password => _passwordForm(),
                    _Step.session => _sessionForm(),
                    _Step.tdata => _tdataForm(),
                  },
                ],
              ),
      ),
      actions: [
        TextButton(
          onPressed: _busy
              ? null
              : () => Navigator.pop(context),
          child: const Text('Закрыть'),
        ),
      ],
    );
  }

  Widget _choose() => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          OutlinedButton.icon(
            onPressed: () => setState(() => _step = _Step.phone),
            icon: const Icon(Icons.smartphone),
            label: const Text('По номеру телефона (код + 2FA)'),
          ),
          const SizedBox(height: 8),
          OutlinedButton.icon(
            onPressed: () => setState(() => _step = _Step.session),
            icon: const Icon(Icons.vpn_key_outlined),
            label: const Text('Из StringSession (telethon/gramjs)'),
          ),
          const SizedBox(height: 8),
          OutlinedButton.icon(
            onPressed: () => setState(() => _step = _Step.tdata),
            icon: const Icon(Icons.folder_zip_outlined),
            label: const Text('Из tdata (zip-архив)'),
          ),
        ],
      );

  Widget _phoneForm() => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            controller: _phoneCtrl,
            autofocus: true,
            decoration: const InputDecoration(
                labelText: 'Телефон', hintText: '+79991234567'),
            keyboardType: TextInputType.phone,
          ),
          const SizedBox(height: 12),
          FilledButton(
            onPressed: _sendCode,
            child: const Text('Отправить код'),
          ),
          TextButton(
            onPressed: () => setState(() => _step = _Step.choose),
            child: const Text('Назад'),
          ),
        ],
      );

  Future<void> _sendCode() async {
    await _run(() async {
      final res = await widget.controller
          .addPhone(pool: widget.pool, phone: _phoneCtrl.text.trim());
      if (res.ok && res.isNeed2fa == false) {
        setState(() => _step = _Step.code);
      } else if (res.isNeed2fa) {
        // сразу 2FA (не тратя код) — маловероятно, но обработаем
        setState(() => _step = _Step.password);
      } else {
        _fail(res.errorText);
      }
    });
  }

  Widget _codeForm() => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text('Код отправлен на ${_phoneCtrl.text}'),
          const SizedBox(height: 12),
          TextField(
            controller: _codeCtrl,
            autofocus: true,
            decoration: const InputDecoration(labelText: 'Код из Telegram'),
          ),
          const SizedBox(height: 12),
          FilledButton(
            onPressed: _submitCode,
            child: const Text('Войти'),
          ),
          TextButton(
            onPressed: () => setState(() => _step = _Step.choose),
            child: const Text('Отмена'),
          ),
        ],
      );

  Future<void> _submitCode() async {
    await _run(() async {
      final res = await widget.controller.submitCode(
        phone: _phoneCtrl.text.trim(),
        code: _codeCtrl.text.trim(),
      );
      if (res.ok) {
        if (mounted) Navigator.pop(context);
      } else if (res.isNeed2fa) {
        setState(() => _step = _Step.password);
      } else {
        _fail(res.errorText);
      }
    });
  }

  Widget _passwordForm() => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Text('У аккаунта включена двухэтапная аутентификация.'),
          const SizedBox(height: 12),
          TextField(
            controller: _passwordCtrl,
            autofocus: true,
            obscureText: true,
            decoration:
                const InputDecoration(labelText: '2FA-пароль'),
          ),
          const SizedBox(height: 12),
          FilledButton(
            onPressed: _submitPassword,
            child: const Text('Продолжить'),
          ),
        ],
      );

  Future<void> _submitPassword() async {
    await _run(() async {
      final res = await widget.controller.submitPassword(
        phone: _phoneCtrl.text.trim(),
        password: _passwordCtrl.text,
      );
      if (res.ok) {
        if (mounted) Navigator.pop(context);
      } else {
        _fail(res.errorText);
      }
    });
  }

  Widget _sessionForm() => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            controller: _sessionCtrl,
            autofocus: true,
            maxLines: 4,
            decoration: const InputDecoration(
              labelText: 'StringSession',
              hintText: '1BQANOAu...',
            ),
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _apiIdCtrl,
                  decoration:
                      const InputDecoration(labelText: 'api_id (необяз.)'),
                ),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: TextField(
                  controller: _apiHashCtrl,
                  decoration:
                      const InputDecoration(labelText: 'api_hash (необяз.)'),
                ),
              ),
            ],
          ),
          const SizedBox(height: 12),
          FilledButton(
            onPressed: _importSession,
            child: const Text('Импортировать'),
          ),
          TextButton(
            onPressed: () => setState(() => _step = _Step.choose),
            child: const Text('Назад'),
          ),
        ],
      );

  Future<void> _importSession() async {
    await _run(() async {
      final res = await widget.controller.importSession(
        pool: widget.pool,
        session: _sessionCtrl.text.trim(),
        apiId: int.tryParse(_apiIdCtrl.text.trim()),
        apiHash: _apiHashCtrl.text.trim(),
      );
      if (res.ok) {
        if (mounted) Navigator.pop(context);
      } else {
        _fail(res.errorText);
      }
    });
  }

  Widget _tdataForm() => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            controller: _tdataPathCtrl,
            autofocus: true,
            decoration: const InputDecoration(
              labelText: 'Путь к zip-архиву tdata',
              hintText: r'C:\downloads\tdata.zip',
            ),
          ),
          const SizedBox(height: 4),
          Text(
            'Архив с папкой tdata (Telegram Desktop) внутри.',
            style: Theme.of(context).textTheme.bodySmall,
          ),
          const SizedBox(height: 12),
          FilledButton(
            onPressed: _importTdata,
            child: const Text('Импортировать'),
          ),
          TextButton(
            onPressed: () => setState(() => _step = _Step.choose),
            child: const Text('Назад'),
          ),
        ],
      );

  Future<void> _importTdata() async {
    await _run(() async {
      final res = await widget.controller.importTdata(
        pool: widget.pool,
        zipPath: _tdataPathCtrl.text.trim(),
      );
      if (res.ok) {
        if (mounted) Navigator.pop(context);
      } else {
        _fail(res.errorText);
      }
    });
  }
}
