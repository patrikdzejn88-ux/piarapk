import 'dart:async';

import 'package:dio/dio.dart';
import 'package:uuid/uuid.dart';

import 'exceptions.dart';
import 'models.dart';

/// Клиент официального API dark.shopping (https://dark.shopping/developer).
///
/// Особенности протокола: ключ в параметре `key`; конверт
/// {"success":bool,"data":{...}}; пагинация page/per-page; лимит 2 запроса/сек
/// (троттлинг + ретраи на 429).
class DarkShoppingClient {
  DarkShoppingClient({
    this.baseUrl = 'https://dark.shopping',
    required this.apiKey,
    Dio? dio,
  })  : assert(apiKey.isNotEmpty, 'API-ключ не задан'),
        _dio = dio ??
            Dio(BaseOptions(
              connectTimeout: const Duration(seconds: 20),
              receiveTimeout: const Duration(seconds: 60),
              headers: {'Accept': 'application/json'},
            ));

  static const _minInterval = Duration(milliseconds: 550);

  final String baseUrl;
  final String apiKey;
  final Dio _dio;
  final _uuid = const Uuid();

  DateTime _lastRequestAt = DateTime.fromMillisecondsSinceEpoch(0);
  Future<void> _queue = Future.value();

  /// Сериализованная очередь запросов с троттлингом 2 rps.
  Future<T> _enqueue<T>(Future<T> Function() task) {
    final completer = Completer<T>();
    _queue = _queue.then((_) async {
      try {
        final sinceLast = DateTime.now().difference(_lastRequestAt);
        if (sinceLast < _minInterval) {
          await Future<void>.delayed(_minInterval - sinceLast);
        }
        _lastRequestAt = DateTime.now();
        final result = await task();
        completer.complete(result);
      } catch (e, st) {
        completer.completeError(e, st);
      }
    });
    return completer.future;
  }

  Future<Map<String, dynamic>> _call(
    String method, {
    Map<String, dynamic>? query,
    Object? postData,
  }) =>
      _enqueue(() async {
        final url = '$baseUrl/api/v1/$method';
        const rateWaits = [Duration(seconds: 1), Duration(seconds: 2), Duration(seconds: 4)];
        const netRetries = 2;
        var netTries = 0;
        var rateTries = 0;
        while (true) {
          try {
            final Response<Map<String, dynamic>> resp;
            if (postData != null) {
              resp = await _dio.post<Map<String, dynamic>>(
                url,
                queryParameters: {'_format': 'json'},
                data: {'key': apiKey, ...?_asMap(postData)},
              );
            } else {
              resp = await _dio.get<Map<String, dynamic>>(
                url,
                queryParameters: {'key': apiKey, '_format': 'json', ...?query},
              );
            }
            final body = resp.data;
            if (body == null) {
              throw DarkShoppingApiException(
                  message: 'пустой ответ от $method', status: resp.statusCode);
            }
            final success = body['success'] == true;
            final data = body['data'];
            if (!success) {
              final d = data is Map ? data : <String, dynamic>{};
              throw DarkShoppingApiException(
                message: d['message']?.toString() ?? 'ошибка запроса $method',
                name: d['name']?.toString(),
                status: (d['status'] as num?)?.toInt() ?? resp.statusCode,
              );
            }
            if (data is! Map) {
              return <String, dynamic>{};
            }
            return data.cast<String, dynamic>();
          } on DioException catch (e) {
            final code = e.response?.statusCode;
            if (code == 429 && rateTries < rateWaits.length) {
              await Future<void>.delayed(rateWaits[rateTries++]);
              continue;
            }
            if (netTries < netRetries &&
                (e.type == DioExceptionType.connectionTimeout ||
                    e.type == DioExceptionType.receiveTimeout ||
                    e.type == DioExceptionType.connectionError)) {
              await Future<void>.delayed(const Duration(seconds: 2));
              netTries++;
              continue;
            }
            throw DarkShoppingApiException(
              message: 'сеть: ${e.message}',
              status: code,
              isRateLimit: code == 429,
            );
          }
        }
      });

  Map<String, dynamic>? _asMap(Object? data) =>
      data is Map ? data.cast<String, dynamic>() : null;

  // ---- Методы API ----

  Future<List<Category>> getCategories() async {
    final data = await _call('category/list');
    final items = data['items'];
    return (items is List ? items : [])
        .whereType<Map>()
        .map((m) => Category.fromMap(m.cast<String, dynamic>()))
        .toList();
  }

  Future<List<Group>> getGroups({int? categoryId, String? name}) async {
    final data = await _call('group/list', query: {
      'category_id': ?categoryId,
      if (name != null && name.isNotEmpty) 'name': name,
    });
    final items = data['items'];
    return (items is List ? items : [])
        .whereType<Map>()
        .map((m) => Group.fromMap(m.cast<String, dynamic>()))
        .toList();
  }

  Future<Paged<Product>> getProducts(ProductFilter filter) async {
    final data = await _call('product/list',
        query: filter.toQuery(key: apiKey)..remove('key'));
    return Paged.fromEnvelope(data, Product.fromMap);
  }

  Future<Product> getProduct(int id) async {
    final data = await _call('product/view', query: {'id': id});
    return Product.fromMap(data);
  }

  Future<List<Product>> getTopProducts() async {
    final data = await _call('product/top');
    final items = data['items'] ?? data;
    return (items is List ? items : [])
        .whereType<Map>()
        .map((m) => Product.fromMap(m.cast<String, dynamic>()))
        .toList();
  }

  Future<Balance> getBalance() async {
    final data = await _call('user/balance');
    return Balance.fromMap(data);
  }

  Future<OrderResult> createOrder({
    required int product,
    required int quantity,
    String? promoCode,
  }) async {
    final data = await _call('order/create', postData: {
      'product': product,
      'quantity': quantity,
      if (promoCode != null && promoCode.isNotEmpty) 'promo_code': promoCode,
      'idempotence_id': _uuid.v4(),
    });
    return OrderResult.fromMap(data);
  }

  Future<OrderStatus> getOrderStatus(int id) async {
    final data = await _call('order/status', query: {'id': id});
    return OrderStatus.fromMap(data);
  }

  /// Ссылка на файл выдачи заказа.
  Future<String> getOrderDownload(int id) async {
    final data = await _call('order/download', query: {'id': id});
    return data['link']?.toString() ?? '';
  }
}
