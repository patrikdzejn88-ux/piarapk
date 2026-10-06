import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:flutter/services.dart';

/// Результат вызова ядра (синхронного или асинхронного).
class PiarResult {
  PiarResult({required this.ok, this.data, this.error});

  final bool ok;
  final dynamic data;
  final dynamic error;

  factory PiarResult.fromMap(Map<String, dynamic> map) => PiarResult(
        ok: map['ok'] == true,
        data: map['data'],
        error: map['error'],
      );

  /// Ошибка означает необходимость ввода 2FA-пароля.
  bool get isNeed2fa {
    if (error is Map) return error['code'] == 'NEED_2FA';
    final s = error?.toString() ?? '';
    return s.contains('NEED_2FA');
  }

  String get errorText => error?.toString() ?? 'неизвестная ошибка';
}

/// Событие ядра: result (ответ на callAsync), progress, log.
class PiarEvent {
  PiarEvent({
    required this.requestId,
    required this.type,
    required this.method,
    required this.ok,
    this.data,
    this.error,
  });

  final int requestId;
  final String type; // result | progress | log
  final String method;
  final bool ok;
  final dynamic data;
  final dynamic error;

  factory PiarEvent.fromMap(Map<String, dynamic> m) => PiarEvent(
        requestId: (m['request_id'] as num?)?.toInt() ?? 0,
        type: m['type']?.toString() ?? '',
        method: m['method']?.toString() ?? '',
        ok: m['ok'] == true,
        data: m['data'],
        error: m['error'],
      );
}

/// Обёртка над C-ABI Rust-ядра (crate piarcore).
///
/// ABI: piar_init / piar_call / piar_call_async / piar_poll / piar_free /
/// piar_shutdown. Все строки UTF-8, ответы — JSON.
class PiarCore {
  PiarCore._();

  static final PiarCore instance = PiarCore._();

  static const _pathsChannel = MethodChannel('piarapk/paths');

  bool available = false;
  String? libraryPath;
  String? lastError;

  /// Последние log/panic события ядра (кольцевой буфер, для диагностики).
  final List<String> _lastLogs = <String>[];

  List<String> get lastLogs => List.unmodifiable(_lastLogs);

  /// Записать сообщение в журнал приложения (хлебные крошки UI, ошибки
  /// виджетов и платформы) — видно в «Логе ядра» и под спиннером.
  void note(String message) => _note('UI', message);

  /// Ошибка UI/платформы — тоже в журнал.
  void noteError(String message) => _note('UIERR', message);

  void _note(String level, String message) {
    _lastLogs.add('$level [ui] $message');
    if (_lastLogs.length > 200) {
      _lastLogs.removeRange(0, _lastLogs.length - 200);
    }
  }

  ffi.DynamicLibrary? _lib;
  // nullable: повторный init() после провала может переприсвоить их без
  // LateInitializationError (см. A.2.1).
  int Function(ffi.Pointer<ffi.Uint8>)? _piarInit;
  int Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
      ffi.Pointer<ffi.Pointer<ffi.Uint8>>)? _piarCall;
  int Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
      ffi.Pointer<ffi.Uint64>)? _piarCallAsync;
  int Function(int, ffi.Pointer<ffi.Pointer<ffi.Uint8>>)? _piarPoll;
  void Function(ffi.Pointer<ffi.Uint8>)? _piarFree;
  void Function()? _piarShutdown;

  Timer? _pollTimer;
  bool _polling = false;
  final Map<int, Completer<PiarResult>> _pending = {};
  final StreamController<PiarEvent> _events =
      StreamController<PiarEvent>.broadcast();

  /// Широковещательный поток событий ядра (result/progress/log).
  Stream<PiarEvent> get events => _events.stream;

  static Future<void>? _initFuture;

  /// Инициализация (идемпотентна: параллельные вызовы дают один и тот же
  /// Future — один поллер, одна загрузка библиотеки).
  /// apiId/apiHash — обязательная своя пара с my.telegram.org: без неё ядро
  /// не инициализируется (код -3), дефолтной пары нет.
  Future<void> init({int? apiId, String? apiHash}) {
    final existing = _initFuture;
    if (existing != null) return existing;
    final f = _doInit(apiId, apiHash).then((_) {
      // мягкий провал (available == false) тоже не кэшируем: следующий
      // init() сможет повторить попытку
      if (!available) _initFuture = null;
    }).catchError((Object e) {
      // не кэшируем провал: следующий init() сможет повторить попытку
      _initFuture = null;
      lastError = 'инициализация ядра: $e';
      available = false;
    });
    _initFuture = f;
    return f;
  }

  Future<void> _doInit(int? apiId, String? apiHash) async {
    ffi.DynamicLibrary? lib;
    if (Platform.isAndroid) {
      // libpiarcore.so предзагружается в MainActivity (System.loadLibrary);
      // dlopen по имени резолвится из nativeLibraryDir приложения.
      try {
        lib = ffi.DynamicLibrary.open('libpiarcore.so');
        libraryPath = 'libpiarcore.so';
      } catch (e) {
        lastError = 'по имени: $e';
        try {
          const ch = MethodChannel('piarapk/paths');
          final dir = await ch.invokeMethod<String>('getNativeLibraryDir');
          if (dir == null || dir.isEmpty) {
            lastError = 'libpiarcore.so не загрузилась: пустой nativeLibraryDir';
            available = false;
            _initFuture = null;
            return;
          }
          lib = ffi.DynamicLibrary.open('$dir/libpiarcore.so');
          libraryPath = '$dir/libpiarcore.so';
          lastError = null;
        } catch (e2) {
          lastError = 'libpiarcore.so не загрузилась ($e | $e2)';
          available = false;
          _initFuture = null;
          return;
        }
      }
    } else {
      final path = _findLibrary();
      if (path == null) {
        lastError = 'библиотека не найдена рядом с приложением';
        available = false;
        _initFuture = null;
        return;
      }
      libraryPath = path;
      lib = ffi.DynamicLibrary.open(path);
    }

    // nullable-поле с проверкой: повторный init() не бросит LateInitializationError
    _lib = lib;
    final nativeLib = _lib!;

    // Все lookup'ы — в локальные переменные: если что-то не найдётся,
    // поля не будут присвоены частично и повторный init() сможет повториться.
    final piarInit = nativeLib.lookupFunction<
        ffi.Int32 Function(ffi.Pointer<ffi.Uint8>),
        int Function(ffi.Pointer<ffi.Uint8>)>('piar_init');
    final piarCall = nativeLib.lookupFunction<
        ffi.Int32 Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
            ffi.Pointer<ffi.Pointer<ffi.Uint8>>),
        int Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
            ffi.Pointer<ffi.Pointer<ffi.Uint8>>)>('piar_call');
    final piarCallAsync = nativeLib.lookupFunction<
        ffi.Int32 Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
            ffi.Pointer<ffi.Uint64>),
        int Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
            ffi.Pointer<ffi.Uint64>)>('piar_call_async');
    final piarPoll = nativeLib.lookupFunction<
        ffi.Int32 Function(
            ffi.Int32, ffi.Pointer<ffi.Pointer<ffi.Uint8>>),
        int Function(
            int, ffi.Pointer<ffi.Pointer<ffi.Uint8>>)>('piar_poll');
    final piarFree = nativeLib.lookupFunction<
        ffi.Void Function(ffi.Pointer<ffi.Uint8>),
        void Function(ffi.Pointer<ffi.Uint8>)>('piar_free');
    final piarShutdown = nativeLib.lookupFunction<
        ffi.Void Function(), void Function()>('piar_shutdown');

    // Публикуем указатели только после успешных lookup'ов: повторный init()
    // после частичного провала не бросит LateInitializationError.
    _piarInit = piarInit;
    _piarCall = piarCall;
    _piarCallAsync = piarCallAsync;
    _piarPoll = piarPoll;
    _piarFree = piarFree;
    _piarShutdown = piarShutdown;

    String? dataDirPath;
    if (Platform.isAndroid) {
      try {
        dataDirPath =
            await _pathsChannel.invokeMethod<String>('getFilesDir');
      } catch (_) {
        dataDirPath = null;
      }
      dataDirPath ??= '/data/data/com.piarkapk.piarapk/files';
    } else {
      // desktop: каталог данных рядом с .exe — не зависит от CWD запуска
      final exeDir = File(Platform.resolvedExecutable).parent.path;
      dataDirPath = '$exeDir${Platform.pathSeparator}data';
    }
    final dataDir = Directory(dataDirPath);
    if (!dataDir.existsSync()) {
      dataDir.createSync(recursive: true);
    }
    final cfg = jsonEncode({
      'data_dir': dataDir.absolute.path,
      'api_id': ?apiId,
      if (apiHash != null && apiHash.isNotEmpty) 'api_hash': apiHash,
    });
    final initFn = _piarInit;
    if (initFn == null) {
      lastError = 'ядро не загружено: piar_init не найден';
      available = false;
      _initFuture = null;
      return;
    }
    final cfgPtr = cfg.toNativeUtf8().cast<ffi.Uint8>();
    final int code;
    try {
      code = initFn(cfgPtr);
    } finally {
      // указатель освобождаем всегда, независимо от кода возврата
      calloc.free(cfgPtr);
    }
    // B.1.2/B.2.9: ненулевой код — ядро не инициализировано. Не считаем
    // его доступным и не запускаем поллинг; -3 = не задана пара api_id/api_hash.
    if (code != 0) {
      lastError = code == -3
          ? 'Укажите api_id/api_hash в настройках API'
          : 'не удалось инициализировать ядро, код $code';
      available = false;
      _initFuture = null;
      return;
    }

    available = true;
    _startPolling();
  }

  /// Поиск нативной библиотеки рядом с исполняемым файлом (desktop).
  String? _findLibrary() {
    final exe = File(Platform.resolvedExecutable);
    final dir = exe.parent.path;
    final sep = Platform.pathSeparator;
    final names =
        Platform.isWindows ? <String>['piarcore.dll'] : <String>['libpiarcore.dylib'];
    final candidates = <String>[
      dir,
      '$dir${sep}Frameworks',
      '$dir$sep..${sep}Frameworks',
    ];
    for (final c in candidates) {
      for (final n in names) {
        final f = File('$c$sep$n');
        if (f.existsSync()) return f.path;
      }
    }
    return null;
  }

  /// Синхронный вызов метода ядра.
  PiarResult call(String method, Map<String, dynamic> params) {
    final piarCall = _piarCall;
    final piarFree = _piarFree;
    if (!available || piarCall == null || piarFree == null) {
      return PiarResult(ok: false, error: 'ядро не загружено');
    }
    final mPtr = method.toNativeUtf8().cast<ffi.Uint8>();
    final pPtr = jsonEncode(params).toNativeUtf8().cast<ffi.Uint8>();
    final outPtr = calloc<ffi.Pointer<ffi.Uint8>>();
    try {
      final code = piarCall(mPtr, pPtr, outPtr);
      if (code != 0) {
        return PiarResult(ok: false, error: 'piar_call: код $code');
      }
      final raw = outPtr.value;
      if (raw == ffi.nullptr) {
        return PiarResult(ok: false, error: 'пустой ответ ядра');
      }
      // toDartString() (невалидный UTF-8 → FormatException) и jsonDecode()
      // обёрнуты в единый try: любое исключение → ok:false, а указатель
      // освобождается ровно один раз в finally.
      try {
        final s = raw.cast<Utf8>().toDartString();
        final map = jsonDecode(s);
        if (map is Map<String, dynamic>) {
          return PiarResult.fromMap(map);
        }
        return PiarResult(ok: false, error: 'некорректный ответ ядра: $s');
      } catch (e) {
        return PiarResult(ok: false, error: 'некорректный ответ ядра: $e');
      } finally {
        piarFree(raw);
      }
    } finally {
      calloc.free(mPtr);
      calloc.free(pPtr);
      calloc.free(outPtr);
    }
  }

  /// Асинхронный вызов: ответ придёт событием result через [events]/Future.
  Future<PiarResult> callAsync(String method, Map<String, dynamic> params) {
    final piarCallAsync = _piarCallAsync;
    if (!available || piarCallAsync == null) {
      return Future.value(PiarResult(ok: false, error: 'ядро не загружено'));
    }
    final mPtr = method.toNativeUtf8().cast<ffi.Uint8>();
    final pPtr = jsonEncode(params).toNativeUtf8().cast<ffi.Uint8>();
    final idPtr = calloc<ffi.Uint64>();
    try {
      final code = piarCallAsync(mPtr, pPtr, idPtr);
      if (code != 0) {
        return Future.value(
            PiarResult(ok: false, error: 'piar_call_async: код $code'));
      }
      final id = idPtr.value;
      final completer = Completer<PiarResult>();
      _pending[id] = completer;
      return completer.future.timeout(
        const Duration(minutes: 15),
        onTimeout: () {
          _pending.remove(id);
          return PiarResult(ok: false, error: 'таймаут ответа ядра (15 мин)');
        },
      );
    } finally {
      calloc.free(mPtr);
      calloc.free(pPtr);
      calloc.free(idPtr);
    }
  }

  void _startPolling() {
    if (_pollTimer != null) return;
    _pollTimer = Timer.periodic(const Duration(milliseconds: 100), (_) {
      _pollOnce();
    });
  }

  /// Остановить поллинг при уходе приложения в бэкграунд.
  void pausePolling() {
    _pollTimer?.cancel();
    _pollTimer = null;
  }

  /// Возобновить поллинг при возврате из бэкграунда.
  void resumePolling() {
    if (available) _startPolling();
  }

  void _pollOnce() {
    final piarPoll = _piarPoll;
    final piarFree = _piarFree;
    if (!available || _polling || piarPoll == null || piarFree == null) return;
    _polling = true;
    final outPtr = calloc<ffi.Pointer<ffi.Uint8>>();
    try {
      final code = piarPoll(0, outPtr);
      if (code != 0) return;
      final raw = outPtr.value;
      if (raw == ffi.nullptr) return;
      final s = raw.cast<Utf8>().toDartString();
      piarFree(raw);
      if (s.isEmpty || s == '[]') return;
      final dynamic decoded;
      try {
        decoded = jsonDecode(s);
      } catch (e) {
        // битый JSON верхнего уровня: раньше пачка молча терялась
        _lastLogs.add('WARN [poll] пачка событий потеряна (битый JSON): $e');
        if (_lastLogs.length > 200) {
          _lastLogs.removeRange(0, _lastLogs.length - 200);
        }
        return;
      }
      if (decoded is! List) return;
      for (final item in decoded) {
        // каждое событие обрабатывается независимо: битое одно не должно
        // уничтожать всю пачку (иначе result зависшего запроса теряется)
        try {
          if (item is! Map) continue;
          final ev = PiarEvent.fromMap(item.cast<String, dynamic>());
          if (ev.type == 'log') {
            final d = ev.data;
            _lastLogs
                .add('${d?['level'] ?? ''} [${ev.method}] ${d?['message'] ?? ''}');
            if (_lastLogs.length > 200) {
              _lastLogs.removeRange(0, _lastLogs.length - 200);
            }
          }
          // B.1.3: паника ядра приходит error-событием — не теряем её молча
          if (ev.type == 'error') {
            final d = ev.data;
            final code = d is Map ? (d['code'] ?? d) : d;
            final msg = d is Map ? d['message'] : null;
            _lastLogs.add(
                'ERROR [poll] ядро сообщило об ошибке: $code${msg != null ? ' — $msg' : ''}');
            if (_lastLogs.length > 200) {
              _lastLogs.removeRange(0, _lastLogs.length - 200);
            }
          }
          if (ev.type == 'result') {
            final completer = _pending.remove(ev.requestId);
            completer?.complete(PiarResult(
              ok: ev.ok,
              data: ev.data,
              error: ev.error,
            ));
          }
          _events.add(ev);
        } catch (e) {
          _lastLogs.add('WARN [poll] событие потеряно: $e');
          if (_lastLogs.length > 200) {
            _lastLogs.removeRange(0, _lastLogs.length - 200);
          }
        }
      }
    } catch (_) {
      // поллинг не должен ронять приложение
    } finally {
      calloc.free(outPtr);
      _polling = false;
    }
  }

  void shutdown() {
    pausePolling();
    final piarShutdown = _piarShutdown;
    if (available && piarShutdown != null) {
      try {
        piarShutdown();
      } catch (_) {}
    }
  }
}
