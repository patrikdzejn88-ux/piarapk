/// Ошибка ответа API dark.shopping (конверт success=false)
/// или исчерпанные ретраи сети/429.
class DarkShoppingApiException implements Exception {
  DarkShoppingApiException(
      {required this.message, this.name, this.status, this.isRateLimit = false});

  final String message;
  final String? name;
  final int? status;
  final bool isRateLimit;

  @override
  String toString() {
    final n = name == null ? '' : '$name: ';
    final s = status == null ? '' : ' (HTTP $status)';
    return 'DarkShopping$n$message$s';
  }
}
