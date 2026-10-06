import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/bridge.dart';
import 'account.dart';

/// Состояние раздела «Аккаунты»: два пула (piar / parser).
class AccountsController extends ChangeNotifier {
  AccountsController() {
    _eventsSub = PiarCore.instance.events.listen(_onEvent);
  }

  static const pools = ['piar', 'parser'];

  bool loading = false;
  String? lastError;
  final List<Account> _accounts = [];

  StreamSubscription<PiarEvent>? _eventsSub;

  List<Account> byPool(String pool) =>
      _accounts.where((a) => a.pool == pool).toList();

  Future<void> reload() async {
    loading = true;
    lastError = null;
    notifyListeners();
    final res = PiarCore.instance.call('list_accounts', {});
    loading = false;
    if (res.ok) {
      final list = res.data;
      _accounts
        ..clear()
        ..addAll((list is List ? list : [])
            .whereType<Map>()
            .map((m) => Account.fromMap(m.cast<String, dynamic>())));
    } else {
      lastError = res.errorText;
    }
    notifyListeners();
  }

  Future<PiarResult> addPhone({required String pool, required String phone}) =>
      PiarCore.instance
          .callAsync('add_account_phone', {'pool': pool, 'phone': phone});

  Future<PiarResult> submitCode({required String phone, required String code}) =>
      PiarCore.instance
          .callAsync('submit_auth_code', {'phone': phone, 'code': code});

  Future<PiarResult> submitPassword(
          {required String phone, required String password}) =>
      PiarCore.instance.callAsync(
          'submit_auth_password', {'phone': phone, 'password': password});

  Future<PiarResult> importSession(
          {required String pool,
          required String session,
          int? apiId}) =>
      PiarCore.instance.callAsync('import_string_session', {
        'pool': pool,
        'session': session,
        'api_id': ?apiId,
      });

  Future<PiarResult> importTdata(
          {required String pool, required String zipPath}) =>
      PiarCore.instance
          .callAsync('import_tdata', {'pool': pool, 'zip_path': zipPath});

  Future<PiarResult> connect(String id) =>
      PiarCore.instance.callAsync('connect_account', {'id': id});

  Future<PiarResult> disconnect(String id) =>
      PiarCore.instance.callAsync('disconnect_account', {'id': id});

  Future<PiarResult> delete(String id) =>
      PiarCore.instance.callAsync('delete_account', {'id': id});

  /// Перенос аккаунта в другой пул («piar» ↔ «parser»).
  Future<PiarResult> moveToPool(String id, String pool) => PiarCore.instance
      .callAsync('move_account', {'id': id, 'to_pool': pool});

  void _onEvent(PiarEvent event) {
    // после любого результата по аккаунтным методам — обновить список
    const methods = {
      'add_account_phone',
      'submit_auth_code',
      'submit_auth_password',
      'import_string_session',
      'import_tdata',
      'connect_account',
      'disconnect_account',
      'delete_account',
      'move_account',
    };
    if (event.type == 'result' && methods.contains(event.method)) {
      unawaited(reload());
    }
  }

  @override
  void dispose() {
    unawaited(_eventsSub?.cancel());
    super.dispose();
  }
}
