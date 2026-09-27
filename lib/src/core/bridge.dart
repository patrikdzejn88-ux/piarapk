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

  late final ffi.DynamicLibrary _lib;
  late final int Function(ffi.Pointer<ffi.Uint8>) _piarInit;
  late final int Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
      ffi.Pointer<ffi.Pointer<ffi.Uint8>>) _piarCall;
  late final int Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
      ffi.Pointer<ffi.Uint64>) _piarCallAsync;
  late final int Function(
      int, ffi.Pointer<ffi.Pointer<ffi.Uint8>>) _piarPoll;
  late final void Function(ffi.Pointer<ffi.Uint8>) _piarFree;
  late final void Function() _piarShutdown;

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
  Future<void> init() => _initFuture ??= _doInit();

  Future<void> _doInit() async {
    if (available) return;
    final path = Platform.isAndroid ? 'libpiarcore.so' : _findLibrary();
    if (path == null) {
      available = false;
      return;
    }
    libraryPath = path;
    _lib = ffi.DynamicLibrary.open(path);

    _piarInit = _lib
        .lookupFunction<
            ffi.Int32 Function(ffi.Pointer<ffi.Uint8>),
            int Function(ffi.Pointer<ffi.Uint8>)>('piar_init');
    _piarCall = _lib.lookupFunction<
        ffi.Int32 Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
            ffi.Pointer<ffi.Pointer<ffi.Uint8>>),
        int Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
            ffi.Pointer<ffi.Pointer<ffi.Uint8>>)>('piar_call');
    _piarCallAsync = _lib.lookupFunction<
        ffi.Int32 Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
            ffi.Pointer<ffi.Uint64>),
        int Function(ffi.Pointer<ffi.Uint8>, ffi.Pointer<ffi.Uint8>,
            ffi.Pointer<ffi.Uint64>)>('piar_call_async');
    _piarPoll = _lib.lookupFunction<
        ffi.Int32 Function(
            ffi.Int32, ffi.Pointer<ffi.Pointer<ffi.Uint8>>),
        int Function(
            int, ffi.Pointer<ffi.Pointer<ffi.Uint8>>)>('piar_poll');
    _piarFree = _lib.lookupFunction<
        ffi.Void Function(ffi.Pointer<ffi.Uint8>),
        void Function(ffi.Pointer<ffi.Uint8>)>('piar_free');
    _piarShutdown = _lib.lookupFunction<
        ffi.Void Function(), void Function()>('piar_shutdown');

    String? dataDirPath;
    if (Platform.isAndroid) {
      try {
        dataDirPath =
            await _pathsChannel.invokeMethod<String>('getFilesDir');
      } catch (_) {
        dataDirPath = null;
      }
      dataDirPath ??= '/data/data/com.piarkapk.piarapk/files';
    }
    final dataDir = Directory(dataDirPath ?? 'data');
    if (!dataDir.existsSync()) {
      dataDir.createSync(recursive: true);
    }
    final cfg = jsonEncode({'data_dir': dataDir.absolute.path});
    final cfgPtr = cfg.toNativeUtf8().cast<ffi.Uint8>();
    try {
      _piarInit(cfgPtr);
    } finally {
      calloc.free(cfgPtr);
    }

    available = true;
    _pollTimer = Timer.periodic(const Duration(milliseconds: 100), (_) {
      _pollOnce();
    });
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
    if (!available) {
      return PiarResult(ok: false, error: 'ядро не загружено');
    }
    final mPtr = method.toNativeUtf8().cast<ffi.Uint8>();
    final pPtr = jsonEncode(params).toNativeUtf8().cast<ffi.Uint8>();
    final outPtr = calloc<ffi.Pointer<ffi.Uint8>>();
    try {
      final code = _piarCall(mPtr, pPtr, outPtr);
      if (code != 0) {
        return PiarResult(ok: false, error: 'piar_call: код $code');
      }
      final raw = outPtr.value;
      if (raw == ffi.nullptr) {
        return PiarResult(ok: false, error: 'пустой ответ ядра');
      }
      final s = raw.cast<Utf8>().toDartString();
      _piarFree(raw);
      final map = jsonDecode(s);
      if (map is Map<String, dynamic>) {
        return PiarResult.fromMap(map);
      }
      return PiarResult(ok: false, error: 'некорректный ответ: $s');
    } finally {
      calloc.free(mPtr);
      calloc.free(pPtr);
      calloc.free(outPtr);
    }
  }

  /// Асинхронный вызов: ответ придёт событием result через [events]/Future.
  Future<PiarResult> callAsync(String method, Map<String, dynamic> params) {
    if (!available) {
      return Future.value(PiarResult(ok: false, error: 'ядро не загружено'));
    }
    final mPtr = method.toNativeUtf8().cast<ffi.Uint8>();
    final pPtr = jsonEncode(params).toNativeUtf8().cast<ffi.Uint8>();
    final idPtr = calloc<ffi.Uint64>();
    try {
      final code = _piarCallAsync(mPtr, pPtr, idPtr);
      if (code != 0) {
        return Future.value(
            PiarResult(ok: false, error: 'piar_call_async: код $code'));
      }
      final id = idPtr.value;
      final completer = Completer<PiarResult>();
      _pending[id] = completer;
      return completer.future;
    } finally {
      calloc.free(mPtr);
      calloc.free(pPtr);
      calloc.free(idPtr);
    }
  }

  void _pollOnce() {
    if (!available || _polling) return;
    _polling = true;
    final outPtr = calloc<ffi.Pointer<ffi.Uint8>>();
    try {
      final code = _piarPoll(0, outPtr);
      if (code != 0) return;
      final raw = outPtr.value;
      if (raw == ffi.nullptr) return;
      final s = raw.cast<Utf8>().toDartString();
      _piarFree(raw);
      if (s.isEmpty || s == '[]') return;
      final decoded = jsonDecode(s);
      if (decoded is! List) return;
      for (final item in decoded) {
        if (item is! Map) continue;
        final ev = PiarEvent.fromMap(item.cast<String, dynamic>());
        if (ev.type == 'result') {
          final completer = _pending.remove(ev.requestId);
          completer?.complete(PiarResult(
            ok: ev.ok,
            data: ev.data,
            error: ev.error,
          ));
        }
        _events.add(ev);
      }
    } catch (_) {
      // поллинг не должен ронять приложение
    } finally {
      calloc.free(outPtr);
      _polling = false;
    }
  }

  void shutdown() {
    _pollTimer?.cancel();
    _pollTimer = null;
    if (available) {
      try {
        _piarShutdown();
      } catch (_) {}
    }
  }
}
