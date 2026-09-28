/// Версия приложения, зашивается при сборке через --dart-define:
///   --dart-define=APP_VERSION=1.0.1 --dart-define=BUILD_COMMIT=<хеш>
/// Отображается в шапке раздела «Аккаунты», чтобы всегда было видно,
/// какая именно сборка установлена.
class AppVersion {
  static const String app =
      String.fromEnvironment('APP_VERSION', defaultValue: 'dev');
  static const String commit =
      String.fromEnvironment('BUILD_COMMIT', defaultValue: 'local');

  static String get display {
    final c = commit.trim();
    if (c.isEmpty || c == 'local') {
      return 'v$app (локальная)';
    }
    final short = c.length > 7 ? c.substring(0, 7) : c;
    return 'v$app · $short';
  }
}
