/// Модели ответов API dark.shopping (гибкий парсинг: поля могут
/// отсутствовать или называться по-другому).
library;

class Category {
  Category({required this.id, required this.name, this.icon});

  final int id;
  final String name;
  final String? icon;

  factory Category.fromMap(Map<String, dynamic> m) => Category(
        id: (m['id'] as num?)?.toInt() ?? 0,
        name: m['name']?.toString() ?? '',
        icon: m['icon']?.toString(),
      );
}

class Group {
  Group({required this.id, required this.categoryId, required this.name});

  final int id;
  final int categoryId;
  final String name;

  factory Group.fromMap(Map<String, dynamic> m) => Group(
        id: (m['id'] as num?)?.toInt() ?? 0,
        categoryId: (m['category_id'] as num?)?.toInt() ?? 0,
        name: m['name']?.toString() ?? '',
      );
}

class Product {
  Product({
    required this.id,
    required this.name,
    required this.price,
    required this.minimumOrder,
    required this.quantity,
    required this.groupId,
    required this.isManualOrderDelivery,
    this.miniature,
    this.description,
    this.qualityPercent,
    this.url,
  });

  final int id;
  final String name;
  final double price;
  final int minimumOrder;
  final int quantity;
  final int groupId;
  final bool isManualOrderDelivery;
  final String? miniature;
  final String? description;
  final double? qualityPercent;
  final String? url;

  bool get autoDelivery => !isManualOrderDelivery;

  factory Product.fromMap(Map<String, dynamic> m) {
    final group = m['group'];
    return Product(
      id: (m['id'] as num?)?.toInt() ?? 0,
      name: m['name']?.toString() ?? '',
      price: (m['price'] as num?)?.toDouble() ?? 0,
      minimumOrder: (m['minimum_order'] as num?)?.toInt() ?? 1,
      quantity: (m['quantity'] as num?)?.toInt() ?? 0,
      groupId:
          group is Map ? (group['id'] as num?)?.toInt() ?? 0 : 0,
      isManualOrderDelivery: m['is_manual_order_delivery'] == true ||
          m['is_manual_order_delivery'] == 1 ||
          m['is_manual_order_delivery'] == '1',
      miniature: m['miniature']?.toString(),
      description: m['description']?.toString(),
      qualityPercent: (m['quality_percent'] as num?)?.toDouble(),
      url: m['url']?.toString(),
    );
  }
}

/// Фильтры каталога: product/list.
class ProductFilter {
  ProductFilter({
    this.groupId,
    this.categoryId,
    this.name,
    this.onlyInStock = true,
    this.deliveryType = 'auto',
    this.priceFrom,
    this.priceTo,
    this.page = 1,
    this.perPage = 50,
  });

  final int? groupId;
  final int? categoryId;
  final String? name;
  final bool onlyInStock;
  final String? deliveryType;
  final double? priceFrom;
  final double? priceTo;
  final int page;
  final int perPage;

  Map<String, dynamic> toQuery({required String key}) => {
        'key': key,
        '_format': 'json',
        if (groupId != null) 'group_id': groupId,
        if (categoryId != null) 'category_id': categoryId,
        if (name != null && name!.isNotEmpty) 'name': name,
        if (onlyInStock) 'only_in_stock': 1,
        if (deliveryType != null) 'delivery_type': deliveryType,
        if (priceFrom != null) 'price_from': priceFrom,
        if (priceTo != null) 'price_to': priceTo,
        'page': page,
        'per-page': perPage,
      };
}

/// Страница с пагинацией (конверт _meta/_links).
class Paged<T> {
  Paged({
    required this.items,
    required this.totalCount,
    required this.pageCount,
    required this.currentPage,
  });

  final List<T> items;
  final int totalCount;
  final int pageCount;
  final int currentPage;

  static Paged<R> fromEnvelope<R>(
    Map<String, dynamic> data,
    R Function(Map<String, dynamic>) parse,
  ) {
    final meta = data['_meta'];
    final items = data['items'];
    return Paged<R>(
      items: (items is List ? items : [])
          .whereType<Map>()
          .map((m) => parse(m.cast<String, dynamic>()))
          .toList(),
      totalCount: meta is Map ? (meta['totalCount'] as num?)?.toInt() ?? 0 : 0,
      pageCount: meta is Map ? (meta['pageCount'] as num?)?.toInt() ?? 0 : 0,
      currentPage: meta is Map ? (meta['currentPage'] as num?)?.toInt() ?? 0 : 0,
    );
  }
}

/// Баланс магазина (поле называется amount/value/balance — берём первое).
class Balance {
  Balance({required this.amount});

  final double amount;

  factory Balance.fromMap(Map<String, dynamic> m) {
    double? v;
    for (final k in ['amount', 'value', 'balance']) {
      final raw = m[k];
      if (raw is num) {
        v = raw.toDouble();
        break;
      }
      if (raw is String && double.tryParse(raw) != null) {
        v = double.parse(raw);
        break;
      }
    }
    return Balance(amount: v ?? 0);
  }
}

/// Результат order/create.
class OrderResult {
  OrderResult({this.id, this.status, this.link, required this.raw});

  final int? id;
  final String? status;
  final String? link;
  final Map<String, dynamic> raw;

  bool get pending => status != null && status != 'completed';

  factory OrderResult.fromMap(Map<String, dynamic> m) => OrderResult(
        id: (m['id'] as num?)?.toInt(),
        status: m['status']?.toString(),
        link: m['link']?.toString(),
        raw: m,
      );
}

/// Статус заказа: unpaid, in_process, completed, canceled, error, refund.
enum OrderStatusKind { unpaid, inProcess, completed, canceled, error, refund, unknown }

class OrderStatus {
  OrderStatus({required this.id, required this.kind, required this.raw});

  final int id;
  final OrderStatusKind kind;
  final Map<String, dynamic> raw;

  static OrderStatusKind parseKind(String? s) => switch (s) {
        'unpaid' => OrderStatusKind.unpaid,
        'in_process' => OrderStatusKind.inProcess,
        'completed' => OrderStatusKind.completed,
        'canceled' => OrderStatusKind.canceled,
        'error' => OrderStatusKind.error,
        'refund' => OrderStatusKind.refund,
        _ => OrderStatusKind.unknown,
      };

  factory OrderStatus.fromMap(Map<String, dynamic> m) => OrderStatus(
        id: (m['id'] as num?)?.toInt() ?? 0,
        kind: parseKind(m['status']?.toString()),
        raw: m,
      );
}
