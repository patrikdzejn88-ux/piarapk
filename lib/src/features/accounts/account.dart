/// Аккаунт из ядра (метод list_accounts).
class Account {
  Account({
    required this.id,
    required this.phone,
    required this.firstName,
    required this.lastName,
    required this.username,
    required this.pool,
    required this.connected,
    required this.restricted,
  });

  final String id;
  final String phone;
  final String firstName;
  final String lastName;
  final String username;
  final String pool; // "piar" | "parser"
  final bool connected;
  final bool restricted;

  factory Account.fromMap(Map<String, dynamic> m) => Account(
        id: m['id']?.toString() ?? '',
        phone: m['phone']?.toString() ?? '',
        firstName: m['first_name']?.toString() ?? '',
        lastName: m['last_name']?.toString() ?? '',
        username: m['username']?.toString() ?? '',
        pool: m['pool']?.toString() ?? 'piar',
        connected: m['connected'] == true,
        restricted: m['restricted'] == true,
      );

  String get displayName {
    final n = '$firstName $lastName'.trim();
    return n.isNotEmpty ? n : (username.isNotEmpty ? '@$username' : phone);
  }
}
