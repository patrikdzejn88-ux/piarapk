# Аудит ошибок и багов проекта piarapk

**Дата:** 2026-10-06
**Область:** Dart/Flutter (`lib/`, `test/`, `pubspec.yaml`), Rust-движок (`rust/`), Android/Kotlin (`android/`), границы Dart↔Rust↔Kotlin.
**Метод:** полное чтение всех исходных файлов двумя независимыми аудиторами + статический анализ (`flutter analyze` — чисто, `flutter test` — 3/3 зелёные).

**Шкала severity:**
- **CRITICAL** — потеря данных, компрометация аккаунтов, неработоспособность ключевой функции.
- **HIGH** — краш, зависание, утечка ресурсов, сломанный сценарий.
- **MEDIUM** — некорректное поведение в реальных условиях, вводящее в заблуждение.
- **LOW** — редкие/маловероятные проявления, гигиена кода.

---

## ТОП-10 самых важных проблем

| # | Severity | Место | Проблема |
|---|----------|-------|----------|
| 1 | HIGH | `AndroidManifest.xml:11-14` | Сессии Telegram (auth-ключи) попадают в бэкап Android (`allowBackup` не задан → `true`) → кража аккаунтов через Auto Backup / adb backup |
| 2 | HIGH | `rust/src/engine/sessions.rs:38,41` | Импорт Telethon StringSession сломан: декодируется STANDARD base64, а Telethon использует urlsafe (`-`/`_`) → почти каждая Telethon-сессия падает с ошибкой декодирования |
| 3 | HIGH | `rust/src/bridge.rs:102,147,215,456` | Паники Rust через FFI без `catch_unwind` (sync-функции + `.expect("tokio runtime")`) → на Rust ≥1.81 abort всего процесса |
| 4 | HIGH | `rust/src/engine/auth.rs:254-261`, `import.rs:183-200`, `inviter.rs:229-237` | Утечка живых MTProto-подключений: `LiveAccount` дропается без `quit()` → утечка tokio-тасок и TCP-соединений, «лишнее устройство» в Telegram |
| 5 | HIGH | `lib/src/features/piar/piar_screen.dart:61-63,71,207-246` | Выбранный чат в дропдауне «протухает» после `_reload()` (Map сравниваются по identity) → краш assert'ом в debug / потеря выбора в release |
| 6 | HIGH | `lib/src/core/bridge.dart:121-132,154-157,161-165` | Провал `PiarCore.init()` кэшируется навсегда (мягкий путь провала не сбрасывает `_initFuture`) → повторная инициализация невозможна до перезапуска приложения |
| 7 | MEDIUM | `rust/src/bridge.rs:13-14` | Захардкоженная пара `api_id`/`api_hash` в исходнике и бинаре — все установки делят одну пару, бан api_id ударит по всем пользователям |
| 8 | MEDIUM | `rust/src/engine/scraper.rs:79-86` | Парсер молча возвращает усечённую базу при FLOOD_WAIT/ошибке сети с флагом `stopped: false` — пользователь считает базу полной |
| 9 | MEDIUM | `rust/src/engine/inviter.rs:70-165` | Инвайтер нельзя отменить и он длится дольше таймаута Dart (15 мин): UI сообщает «таймаут», а рассылка продолжается в фоне |
| 10 | MEDIUM | `rust/src/engine/import.rs:27-30,54-71` | TOCTOU-гонка при импорте: двойной тап удаляет файл сессии, который первый клиент уже открыл → повреждение активной сессии |

---

## Часть A. Dart / Flutter

### A.1 Краш-риски

#### A.1.1 [HIGH] Протухший инстанс выбранного чата → краш дропдауна (debug) / потеря выбора (release)
- **Файл:** `lib/src/features/piar/piar_screen.dart:61-63` (и `:71` для `_database`), проявление в `:207-246`.
- **Суть:** `_reload()` пересоздаёт `_chats` новыми Map-инстансами, а `_chat` переустанавливается только если `== null`. Map в Dart сравниваются по identity → старый `_chat` отсутствует в новом `items`.
- **Проявление:** конструкторный assert `DropdownButtonFormField` требует ровно одного совпадения `initialValue` с `items` (подтверждено исходником Flutter SDK 3.47.5, `material/dropdown.dart:1851-1863`). После 2-го `_reload()` (кнопка «Обновить» `:302`; событие `result` инвайта `:101`) debug-сборка падает с красным экраном, release показывает пустой дропдаун; `_start` (`:124`) продолжает слать протухший `chat_id`. Аналогично `_database` (`:71`), если выбранная база удалена.
- **Исправление:** скопировать подход из `parser_screen.dart:124-129` — после загрузки списка делать re-match `_chat` по `id` (и проверять существование `_database`, иначе `null`), либо хранить в состоянии только `id`/`name`.

#### A.1.2 [MEDIUM] Незащищённый парсинг ответа ядра в синхронном `call()`
- **Файл:** `lib/src/core/bridge.dart:265-271` (внутри `call()`, `:249-277`).
- **Суть:** `toDartString()` и `jsonDecode()` не обёрнуты в try/catch (есть только `finally` для освобождения указателей).
- **Проявление:** невалидный UTF-8/не-JSON из Rust → синхронный `FormatException` во всех вызовах `reload()`. В `accounts_controller.dart:99` `reload()` вызван unawaited из слушателя стрима → unhandled async error, `loading` остаётся `true`.
- **Исправление:** обернуть декодирование в try/catch и возвращать `PiarResult(ok: false, error: 'некорректный ответ ядра: …')`.

#### A.1.3 [LOW] Force unwrap в обработчике progress-события
- **Файл:** `lib/src/features/piar/piar_screen.dart:83` (`d!['note']`). В `parser_screen.dart:64-65` используется безопасная навигация `d?['phase']`/`d?['done']` — проблемы там нет.
- **Суть:** если ядро пришлёт `data` не-Map (строку/число), `d?['note']` бросит `NoSuchMethodError` внутри слушателя broadcast-стрима — исключение уйдёт в Zone как uncaught, минуя `FlutterError.onError`. *[ТРЕБУЕТ ПРОВЕРКИ: присылает ли ядро не-Map `data`]*
- **Исправление:** `final d = e.data; if (d is! Map) return;` в начале обработчиков.

#### A.1.4 [LOW] `titleLarge!` в GradientText
- **Файл:** `lib/src/ui/theme.dart:111`. В стандартной Material-теме не null, риск минимален. Заменить на `?? const TextStyle()`.

### A.2 Асинхронные баги / гонки

#### A.2.1 [HIGH] Провал `init()` кэшируется навсегда — retry невозможен
- **Файл:** `lib/src/core/bridge.dart:121-132`, точки мягкого провала: `:154-157` (Android: оба dlopen неудачны), `:161-165` (desktop: библиотека не найдена).
- **Суть:** комментарий (`:125`) обещает «не кэшируем провал», но сброс `_initFuture = null` происходит только в `catchError`. Мягкие пути провала делают `return` без исключения → future завершается «успешно» с `available=false`, и все последующие `init()` (вызываются из `accounts_screen.dart:27`, `parser_screen.dart:36`, `piar_screen.dart:41`) получают тот же завершённый future.
- **Проявление:** если `libpiarcore.so` не загрузилась при старте (гонка с предзагрузкой в `MainActivity.kt:33-40`, APK без нужного ABI) — приложение навсегда в состоянии «ядро не загружено» до перезапуска. Дополнительно: на пути retry после исключения `late final _lib` (`:95`) при повторном присваивании бросит `LateInitializationError`.
- **Исправление:** в мягких путях провала бросать исключение (чтобы сработал `catchError`) или явно сбрасывать `_initFuture = null` перед `return`; `late final` заменить на nullable поля с проверкой.

#### A.2.2 [MEDIUM] Гонка в кэше настроек
- **Файл:** `lib/src/settings/settings.dart:29-44`.
- **Суть:** `_loaded = true` выставляется (`:31`) ДО завершения `await _dirPath()`/чтения файла. Конкурентный второй вызов `_load()` вернётся немедленно и прочитает пустой `_cache`; параллельный `setApiPair` запишет поверх непрочитанного файла, потеряв данные.
- **Исправление:** кэшировать сам Future: `static Future<void>? _loadFuture; static Future<void> _load() => _loadFuture ??= _doLoad();`

#### A.2.3 [MEDIUM] `_busy` в парсере объединяет две разные операции
- **Файл:** `lib/src/features/parser/parser_screen.dart:115` (установка), `:493-511` (UI).
- **Проявление:** нажатие «Загрузить» (список чатов) → главная кнопка показывает «Собираем…» со спиннером и появляется «Стоп», шлющий `parse_cancel` при незапущенном парсинге.
- **Исправление:** разделить на `_parsing` и `_chatsLoading` (как в `_AddChatDialog`, `piar_screen.dart:372`).

### A.3 Утечки ресурсов

#### A.3.1 [MEDIUM] Утечка TextEditingController при отмене диалогов
- **Файл:** `lib/src/features/parser/parser_screen.dart`:
  - `_addPeople`: контроллер создан `:233`, ранний `return` `:266`, `dispose` `:269` — при отмене не выполняется.
  - `_removePeople`: создан `:331`, `return` `:364`, `dispose` `:367` — та же утечка.
- **Исправление:** перенести `ctrl.dispose()` до `if (ok != true) return;` (сохранив `ctrl.text` в локальную переменную), как сделано правильно в `_createBase` (`:317-320`).

#### A.3.2 [LOW] `PiarCore.shutdown()` никогда не вызывается
- **Файл:** `lib/src/core/bridge.dart:362-370`; вызовов нет. Ядро не получает `piar_shutdown` при завершении приложения; `_events` (broadcast StreamController) и `_pollTimer` живут до смерти процесса.
- **Исправление:** вызывать из `dispose` корневого `_Shell` (`app.dart:60-63`) или через `AppLifecycleListener`.

#### A.3.3 [LOW] Обработчик канала магазина живёт вечно
- **Файл:** `lib/src/features/shop/shop_screen.dart:31-43`. `setMethodCallHandler` захватывает `State` в замыкании; в `dispose` нет `setMethodCallHandler(null)`.
- **Исправление:** сбрасывать обработчик в `dispose`.

### A.4 Логические ошибки

#### A.4.1 [LOW] Неверный приоритет статуса аккаунта
- **Файл:** `lib/src/features/accounts/accounts_screen.dart:307-311`. `(connected: true, restricted: true)` показывается как «Офлайн», хотя аккаунт подключён, но ограничен.
- **Исправление:** сначала проверять `restricted`.

#### A.4.2 [LOW] Несогласованные дефолты лимита парсера
- **Файл:** `lib/src/features/parser/parser_screen.dart`: поле по умолчанию `'30000'` (`:20`), fallback при нечисловом вводе — `10000` (`:99,103`). Привести к одному значению.

### A.5 Безопасность

#### A.5.1 [LOW] Логирование номера телефона
- **Файл:** `lib/src/features/accounts/add_account_dialog.dart:213`. Номер (PII) попадает в кольцевой лог, видимый в «Логе ядра». Пароли/коды не логируются — это хорошо.
- **Исправление:** маскировать номер (`+79***67`).

#### A.5.2 [LOW] api_hash в открытом виде на диске
- **Файл:** `lib/src/settings/settings.dart:46-55,88-101`. `settings.json` с `telegram_api_hash` хранится в открытом виде; на desktop — в CWD-относительной папке `data/`.
- **Исправление:** Keystore-backed хранилище на Android; не выносить файл за пределы приватной директории.

#### A.5.3 [MEDIUM] Хардкод пути данных и CWD-зависимость на desktop
- **Файлы:** `lib/src/core/bridge.dart:203` (хардкод `/data/data/com.piarkapk.piarapk/files`), `:205` (`Directory(dataDirPath ?? 'data')`); `lib/src/settings/settings.dart:19,24`.
- **Проявление:** на desktop путь `data` относителен к CWD запуска — сессии/настройки/базы «переезжают» в зависимости от того, откуда запущен .exe (при этом поиск DLL идёт от `resolvedExecutable` — несогласованно). Хардкод package name ломается при смене applicationId.
- **Исправление:** на desktop резолвить data-директорию от `Platform.resolvedExecutable`.

### A.6 Платформенные каналы (сверка Dart ↔ Kotlin)

Канал **`piarapk/paths`** — имена и сигнатуры совпадают везде, несоответствий **не найдено**. Замечания:

#### A.6.1 [MEDIUM] На desktop у канала нет реализации — функции молча мертвы
- **Файлы:** `lib/src/core/native.dart` (весь), `lib/src/features/shop/shop_screen.dart:50-57,156-160`.
- **Проверено:** в `windows/` и `macos/` обработчика `piarapk/paths` нет. Все вызовы падают с `MissingPluginException` и проглатываются.
- **Проявление:** на Windows/macOS кнопка «Открыть магазин» ВСЕГДА показывает «Не удалось открыть браузер» — единственное действие экрана неработоспособно; экспорт баз и выбор картинки также всегда неуспешны.
- **Исправление:** desktop-реализация канала или честный disable этих действий на desktop.

#### A.6.2 [LOW] Подпись «Открыть в системном браузере» врёт
- **Файл:** `lib/src/features/shop/shop_screen.dart:109,50-57`. `MainActivity.kt:186-198` — `openShop` открывает **внутреннюю** `WebViewActivity`, а не системный браузер.
- **Исправление:** переименовать подпись или слать `Intent.ACTION_VIEW` в системный браузер.

#### A.6.3 [LOW] Мёртвые методы канала
- **Файлы:** `MainActivity.kt:123` (`getCoreLibStatus`), `:128` (`shopLoad`), `:146-147` (`shopCanGoBack/Forward`) — из Dart не вызываются. `getCoreLibStatus` — полезная диагностика ошибки загрузки ядра, стоит задействовать в баннере ошибки.

#### A.6.4 [LOW] `pickImage` сливает отмену и ошибку
- **Файл:** `lib/src/core/native.dart:9-15`. При `PICK_BUSY`/ошибке копирования пользователь видит нейтральное «Картинка не выбрана».
- **Исправление:** различать `PlatformException` и `null`-результат.

### A.7 Конфигурация / тесты

- **[LOW] Мёртвая зависимость:** `pubspec.yaml:36` — `cupertino_icons` нигде не импортируется. Удалить.
- **[LOW] Мёртвый код:** `theme.dart:199-216` (`AppCard`), `theme.dart:15-20` (`AppGradients.headerBg`), `settings.dart:57-71` (`getApiKey`/`setApiKey`), `PiarCore.shutdown()` — не используются.
- **[LOW] Покрытие тестами:** 3 smoke-теста проходят, но не покрыты parser/piar-флоу, состояние после `_reload()`, диалоги. Ни один тест не поймал бы находки A.1.1 и A.3.1. Рекомендация: включить lints `unawaited_futures` и `discarded_futures`.
- **[INFO] Зависимости:** конфликтов в `pubspec.lock` нет; 5 пакетов имеют новые версии — некритично.
- TODO/FIXME/HACK в `lib/` не найдено.

---

## Часть B. Rust-движок

### B.1 Критические и высокие

#### B.1.1 [HIGH] Импорт Telethon StringSession сломан (urlsafe base64)
- **Файл:** `rust/src/engine/sessions.rs:38,41`.
- **Суть:** `base64::engine::general_purpose::STANDARD.decode(b64)`, а Telethon кодирует через `base64.urlsafe_b64encode` (алфавит с `-` и `_`). Вероятность того, что в 352-символьной строке нет ни одного `-`/`_`, ≈ (62/64)^352 ≈ 10⁻⁵ — почти каждая Telethon-сессия падает с `base64: Invalid byte`. *[ТРЕБУЕТ ПРОВЕРКИ только в части подтверждения алфавита конкретной версией Telethon]*
- **Исправление:** декодировать `URL_SAFE` (или fallback `STANDARD` → `URL_SAFE`/`URL_SAFE_NO_PAD` с восстановлением паддинга). Добавить тест на реальную Telethon-строку.

#### B.1.2 [HIGH] `.expect("tokio runtime")` в `piar_init`
- **Файл:** `rust/src/bridge.rs:97-103`.
- **Проявление:** при невозможности создать runtime — паника внутри `extern "C"`; на Rust ≥1.81 это abort процесса. Приложение падает целиком вместо кода ошибки.
- **Исправление:** `match` на `build()` с возвратом ошибки; Dart умеет показывать `lastError`.

#### B.1.3 [HIGH] FFI без `catch_unwind` (sync-функции)
- **Файл:** `rust/src/bridge.rs:147-211` (`piar_call`), `:215-232` (пролог `piar_call_async`), `:456-461` (`piar_poll`).
- **Суть:** async-путь обёрнут в `catch_unwind` (`:237-255`), а sync-функции — нет. Паника в `store::*` (fs/serde — внешние данные: файлы на диске, JSON от Dart) пересечёт FFI-границу → abort. Уточнение верификации: production `unwrap/expect/panic!` в `engine/` вне тестов отсутствуют, поэтому реальная панико-поверхность sync-путей мала — но структурный риск abort (Rust ≥1.81) и `.expect("tokio runtime")` в B.1.2 остаются.
- **Исправление:** обернуть тела sync FFI-функций в `std::panic::catch_unwind` с возвратом `{"code":"PANIC"}`.

#### B.1.4 [HIGH] Drop `LiveAccount` без `quit()` — утечка подключений
- **Файлы:**
  - `rust/src/engine/auth.rs:254-261` — удаление legacy-записи и `insert` поверх существующей: старый `live` дропается.
  - `rust/src/engine/import.rs:183-200` — тот же паттерн при повторном импорте.
  - `rust/src/engine/inviter.rs:229-237` — карантин `e.live = None`.
- **Суть:** `SenderPoolFatHandle` уничтожается без `quit()`, при этом `connect.rs:192-194` и `bridge.rs:481-483` вызывают `quit()` явно (комментарий `state.rs:50-51`: «хранится, чтобы уметь корректно завершить») — код сам признаёт, что Drop не завершает runner. Итог: утечка tokio-таски, TCP-соединения, «лишнее устройство» на стороне Telegram.
- **Дополнительные места того же класса (найдены верификацией):**
  - `rust/src/engine/connect.rs:79,83` — при ошибке `is_authorized().await?` / `get_me().await?` после `start_client` `StartedClient` дропается без `quit()`.
  - `rust/src/engine/import.rs:150-161` (`finalize_import`) — та же утечка; хуже: при `!authorized` файл сессии удаляется (`:156`), пока живой клиент не завершён.
  - `rust/src/engine/auth.rs:86-92` — при ошибке `request_login_code` полу-created SenderPool (runner + updates-таски + TCP) не `quit()` (см. также B.2.1).
- **Исправление:** перед `remove/insert/live = None` забирать `live` и вызывать `l._handle.quit()` (как в `move_account`); на error-путях `connect`/`import`/`auth` — `quit()` до `?`-возврата; в `finalize_import` — сначала `quit()`, потом удаление файла.

### B.2 Средние

#### B.2.1 [MEDIUM] Утечка заброшенных PendingAuth
- **Файл:** `rust/src/engine/auth.rs:72-107`.
- **Суть:** если пользователь запросил код и не завершил вход, `PendingAuth` живёт бесконечно: runner-таска, updates-таска, TCP-соединение, файл `pending_*.sqlite`. `pending_auths` — неограниченная HashMap. На пути ошибки `request_login_code` (`:86-92`) созданный файл сессии не удаляется.
- **Исправление:** TTL на PendingAuth (напр., 10 мин) с janitor'ом; на ошибке — `handle.quit()` + удаление файла.

#### B.2.2 [MEDIUM] Сетевые вызовы без таймаутов
- **Файлы:** `rust/src/engine/connect.rs:79,83`; `chats.rs:35-39,136-145,224-228`; `scraper.rs:79`.
- **Суть:** `is_authorized()`, `get_me()`, `resolve_username()`, `send_message()`, `iter_dialogs()`, `invoke(GetHistory)` не обёрнуты в `tokio::time::timeout` (в auth сделано правильно — `auth.rs:86-92`). При мёртвой сети задача висит бесконечно; Dart отваливается по 15-мин таймауту, а Rust продолжает держать задачу.
- **Исправление:** единый helper с таймаутом (30–60 с) для всех RPC.

#### B.2.3 [MEDIUM] Парсер молча возвращает усечённый результат
- **Файл:** `rust/src/engine/scraper.rs:79-86,267-274`.
- **Проявление:** при ошибке `invoke` (включая FLOOD_WAIT) воркер делает `break` с частичными данными, а итоговый JSON имеет `stopped: false` — UI показывает «готово», база неполная. FLOOD_WAIT не пережидается (в отличие от `inviter.rs:123-135`).
- **Исправление:** флаг `truncated`/`partial` в результате или retry с ожиданием FLOOD_WAIT.

#### B.2.4 [MEDIUM] Инвайтер нельзя отменить; длительность > таймаута Dart
- **Файл:** `rust/src/engine/inviter.rs:70-165`.
- **Суть:** цикл не проверяет `state.parser_cancelled()`. База 10 000 юзеров ≈ 1000 батчей × ~2 с ≈ 33+ мин при `BATCH_PAUSE_MS=2000`, а Dart-таймаут — 15 мин (`bridge.dart:297`). UI сообщает «таймаут», инвайты продолжают идти в фоне.
- **Исправление:** проверять флаг отмены в цикле; согласовать таймауты или дать invite свой cancel-метод.

#### B.2.5 [MEDIUM] Result-события вытесняются из очереди
- **Файл:** `rust/src/engine/state.rs:23-29`.
- **Суть:** `EventQueue.push` при `len > 2000` выбрасывает старейшие события, не различая `log/progress/result`. Парсер шлёт progress на каждую страницу (~10⁴ событий при limit=1M). Если поллер Dart остановлен (приложение в бэкграунде), финальный `result` вытесняется → `callAsync` висит до таймаута.
- **Исправление:** не вытеснять события `type=="result"` (приоритетная очередь/отдельный буфер).

#### B.2.6 [MEDIUM] TOCTOU-гонка и удаление чужого файла сессии при импорте
- **Файл:** `rust/src/engine/import.rs:27-30,54-71`.
- **Суть:** `unique_stem` смотрит только `accounts`, не учитывая `pending_auths`. Двойной тап по «импорт» одной сессии: оба вызова получают один stem, второй удаляет файл, который первый уже открыл → повреждение сессии первого клиента. Шаг «проверил-удалил-создал» не атомарен.
- **Исправление:** учитывать `pending_auths` в `unique_stem`; не удалять существующий файл без проверки владельца; случайный суффикс в имени.

#### B.2.7 [INFO] `fs::rename` поверх существующего файла — не баг на современном Rust
- **Файл:** `rust/src/engine/store.rs:78-84,99-105,241-245`.
- **Верификация опровергла исходную формулировку:** паттерн «tmp + rename» на указанных строках точен, но современный Rust std на Windows использует `MoveFileExW` с флагом `MOVEFILE_REPLACE_EXISTING` (исходник `library/std/src/sys/fs/windows.rs`) — замена существующего файла работает. Сценарий «сохранение перестаёт работать после первой записи» не воспроизводится (целевой файл нигде не удерживается открытым). Оставлено как гигиеническое замечание: при желании можно упростить код через `tempfile::NamedTempFile::persist`.

#### B.2.8 [MEDIUM] Файловый IO в sync FFI на UI-изоляте Dart
- **Файл:** `rust/src/bridge.rs:176-195`.
- **Суть:** `list_databases` читает и парсит ВСЕ файлы баз; `get_database` читает весь файл в память и отдаёт одной JSON-строкой через FFI. `piar_call` выполняется синхронно на UI-изоляте → подвисания интерфейса на больших базах.
- **Исправление:** перевести в `piar_call_async`; `get_database` — отдавать путь для чтения на Dart-стороне.

#### B.2.9 [MEDIUM] Захардкоженная пара api_id/api_hash
- **Файл:** `rust/src/bridge.rs:13-14`.
- **Суть:** `DEFAULT_API_ID = 28614298`, `DEFAULT_API_HASH` — в исходнике и бинаре. Все установки делят одну пару: при abuse любого пользователя Telegram банит api_id для всех.
- **Исправление:** обязательная своя пара в настройках (механизм переопределения уже есть — `bridge.rs:81-90`) или внедрение при сборке из CI-секретов.

#### B.2.10 [MEDIUM] Сессии в открытом виде на диске
- **Файл:** `rust/src/engine/*` (`SqliteSession`, auth_key незашифрован в `filesDir/sessions`).
- В связке с находкой C.1 (бэкап) — компрометация. Пароль 2FA и коды через FFI не логируются ✓, но телефон логируется (B.3.1).
- **Исправление:** шифрование каталога sessions ключом Keystore; минимум — `allowBackup="false"`.

### B.3 Низкие

#### B.3.1 [LOW] Телефон (PII) в логах
- **Файл:** `rust/src/engine/auth.rs:22` (`log::info!("add_account_phone: {phone} ...")`), также `bridge.rs:335,362` (chat_id/база — менее чувствительно).
- **Исправление:** маскировать номер.

#### B.3.2 [LOW] Распаковка zip: блокировка async-потока, нет лимитов
- **Файл:** `rust/src/engine/import.rs:107-123`. Блокирующий IO в async-функции (воркер-тредов всего 2); нет лимитов на размер распаковки → zip-bomb заполнит filesDir. *[Zip-slip: `ZipArchive::extract` в zip 2.x фильтрует `..` — ТРЕБУЕТ ПРОВЕРКИ по минорной версии]*
- **Исправление:** `tokio::task::spawn_blocking` + лимиты размера/числа записей; явная проверка `enclosed_name`.

#### B.3.3 [LOW] Порт всегда 443, IPv6-сессии не импортируются
- **Файлы:** `rust/src/engine/import.rs:39-44`, `sessions.rs:64-76`. `sessions.rs` умеет декодировать gramjs-IPv6, но `create_session_file` парсит только `Ipv4Addr`. Порт захардкожен 443 (`:44`, `tdata.rs:329`).
- **Исправление:** поддержать `SocketAddr` (v4/v6) и пробрасывать порт.

#### B.3.4 [LOW] `move_account` осиротевляет файл сессии
- **Файл:** `rust/src/engine/connect.rs:191-197`. Если в целевом пуле уже есть запись аккаунта, её `session_file` выпадает из реестра — файл остаётся на диске навсегда.
- **Исправление:** при перезаписи удалять старый файл сессии, если он отличается от нового.

#### B.3.5 [LOW] Shutdown невосстановим
- **Файлы:** `rust/src/bridge.rs:471-484`, `lib/src/core/bridge.dart:362-370`. `piar_shutdown` гасит клиентов, но `STATE` (OnceLock) не очищается: повторный `piar_init` вернёт 0 на обесточенном состоянии; Dart не сбрасывает `_initFuture`.
- **Исправление:** документировать one-shot семантику или корректно поддержать реинициализацию.

#### B.3.6 [LOW] Ложная `CHAT_NOT_FOUND` через `try_lock`
- **Файл:** `rust/src/engine/chats.rs:96-99`. При занятом мьютексе возвращается `None` → inviter сообщает «чат не найден», хотя он есть (гонка с add_chat).
- **Исправление:** обычный `lock()` (секция короткая) или отдельный код ошибки.

#### B.3.7 [LOW] Мёртвый код
- **Файл:** `rust/src/engine/store.rs:250-252,28-40` — `remove_one` и `display_name` не вызываются нигде в crate.

---

## Часть C. Android / Kotlin / Манифест / Gradle

### C.1 [HIGH] Бэкап сессий Telegram
- **Файл:** `android/app/src/main/AndroidManifest.xml:11-14`.
- **Суть:** `<application>` не задаёт `android:allowBackup` → default `true`. В filesDir лежат `sessions/*.sqlite` с auth-ключами в открытом виде → Auto Backup (Google Drive) / adb backup (API ≤ 30) выгружает их → полный захват аккаунтов без рута.
- **Исправление:** `android:allowBackup="false"` (или `dataExtractionRules` с `<exclude domain="file" path="sessions/"/>`); в перспективе — шифрование сессий ключом Android Keystore.

### C.2 [MEDIUM] WebView никогда не уничтожается
- **Файл:** `android/app/src/main/kotlin/com/piarkapk/piarapk/WebViewActivity.kt:25-93`.
- **Суть:** WebView создан с Activity-контекстом, `onDestroy()` отсутствует — нет `webView.destroy()`. Каждое открытие магазина оставляет Activity+WebView в памяти до GC.
- **Исправление:** `override fun onDestroy() { webView.destroy(); super.onDestroy() }`.

### C.3 [MEDIUM] Mixed content + third-party cookies в WebView магазина
- **Файл:** `android/app/src/main/kotlin/com/piarkapk/piarapk/ShopWebView.kt:61,52-53`.
- **Суть:** `MIXED_CONTENT_COMPATIBILITY_MODE` разрешает подгрузку HTTP-ресурсов на HTTPS-странице → MITM-подмена контента. `setAcceptThirdPartyCookies(true)` включено глобально для процесса.
- **Исправление:** `MIXED_CONTENT_NEVER_ALLOW`; third-party cookies — только если реально нужны для оплаты.

### C.4 [MEDIUM] Release подписан debug-ключом
- **Файл:** `android/app/build.gradle.kts:33-37` (`signingConfig = signingConfigs.getByName("debug")`).
- **Проявление:** релизная сборка не сможет обновляться поверх будущей нормально подписанной; debug-ключ публично известен.
- **Исправление:** настроить release signingConfig (через `key.properties`, не коммитя секреты).

### C.5 [LOW] Потеря pendingPickResult при пересоздании Activity
- **Файл:** `android/app/src/main/kotlin/com/piarkapk/piarapk/MainActivity.kt:44,148-155`.
- **Суть:** `pendingPickResult` — поле экземпляра; при process death / пересоздании Activity Dart-Future `pickImage()` зависает навсегда (таймаута в `native.dart:9-15` нет).
- **Исправление:** таймаут на Dart-стороне или фейл результата в `onDestroy`.

### C.6 [LOW] Повторный запрос POST_NOTIFICATIONS при каждом старте сервиса
- **Файл:** `android/app/src/main/kotlin/com/piarkapk/piarapk/MainActivity.kt:156-165`. Результат запроса не обрабатывается (requestCode 4711 не покрыт в `onRequestPermissionsResult:267-280`, там только REQ_WRITE_EXPORT=4712); диалог дёргается снова после отказа — но лишь до финального отказа, далее Android сам перестаёт его показывать.
- **Исправление:** проверять `checkSelfPermission` перед запросом; запрашивать один раз.

### C.7 [LOW] Неотменяемый фоновый поток копирования картинки
- **Файл:** `android/app/src/main/kotlin/com/piarkapk/piarapk/MainActivity.kt:59-78`. `Thread{...}.start()` переживает Activity; файл-мусор `picked_image.*` остаётся.
- **Исправление:** корутина с lifecycleScope; чистка старых picked_image.

### C.8 [LOW] Drive-by download + файловый доступ не ограничен
- **Файл:** `android/app/src/main/kotlin/com/piarkapk/piarapk/ShopWebView.kt:96-112,55-66`. `DownloadListener` ставит загрузку любого файла по инициативе страницы без подтверждения; `allowFileAccess`/`allowContentAccess` не отключены явно.
- **Исправление:** подтверждение загрузки диалогом; `allowFileAccess = false`, `allowContentAccess = false`.

### C.9 [LOW] Произвольные схемы в систему
- **Файлы:** `ShopWebView.kt:67-87`, `WebViewActivity.kt:47-68`. Любая не-http(s) схема (`intent:`, `tel:` и т.д.) передаётся `ACTION_VIEW` наружу без белого списка.
- **Исправление:** белый список схем (`tel`, `mailto`), игнорировать `intent:`.

### C.10 [INFO] Ограничения Android 15 для dataSync FGS
- **Файл:** `android/app/src/main/kotlin/com/piarkapk/piarapk/ParserService.kt`. На API 35 `dataSync`-FGS ограничен ~6 часами в сутки — очень длинные парсинги система убьёт. Права/тип/канал оформлены правильно. Уточнение верификации: автостоп сервиса по событию результата уже реализован (`app.dart:52-56`, `parser_screen.dart:110`); проблема только в системной квоте и в потере result-события при переполнении очереди (см. B.2.5).
- **Исправление:** учитывать квоту в UX (предупреждение при очень больших лимитах парсинга).

### C.11 Сверено и чисто ✓
- `exported`: MainActivity `true` + LAUNCHER (корректно), ParserService и WebViewActivity `false` ✓.
- Разрешения соответствуют используемым API (INTERNET, FOREGROUND_SERVICE*, POST_NOTIFICATIONS, WRITE_EXTERNAL_STORAGE с `maxSdkVersion="28"`) ✓.
- `usesCleartextTraffic` не задан → default `false` для targetSdk 28+ ✓.
- minSdk/targetSdk через `flutter.*` — согласуется ✓. AGP 9.1.0 / Kotlin 2.4.0, jvmTarget 17 везде ✓. Секретов в Gradle-файлах нет ✓.

---

## Часть D. Границы Dart ↔ Rust ↔ Kotlin

- **FFI-сигнатуры:** `piar_init/piar_call/piar_call_async/piar_poll/piar_free/piar_shutdown` в `bridge.dart:96-104,170-193` точно соответствуют `bridge.rs:70,147,215,456,464,471` ✓. `piar_free` парит каждую выделенную строку ✓.
- **MethodChannel `piarapk/paths`:** имена методов совпадают ✓; входящий `shopUrl` имеет обработчик ✓.
- **[LOW] `bridge.dart:296-302,354-355`:** глобальный 15-мин таймаут `callAsync` короче реальной длительности invite/parse (см. B.2.4); `catch (_)` в `_pollOnce` при битом JSON верхнего уровня молча теряет всю пачку событий.
- **[LOW] `bridge.dart:222-225`:** `_pollTimer` не останавливается при уходе приложения в бэкграунд (вклад в B.2.5).

---

## Сводная статистика

| Severity | Dart/Flutter | Rust | Android/Gradle | Границы | Итого |
|----------|:---:|:---:|:---:|:---:|:---:|
| HIGH     | 2 | 4 | 1 | — | **7** |
| MEDIUM   | 6 | 9 | 3 | — | **18** |
| LOW      | 14 | 7 | 5 | 2 | **28** |
| INFO     | 1 | 1 | 1 | — | **3** |

**Всего: 56 находок.** (B.2.7 после верификации переведён из MEDIUM в INFO; LOW Dart включает 3 пункта-рекомендации из раздела A.7.)

## Верификация отчёта

Отчёт проверен независимым верификатором (полная сверка всех 10 топ-находок + выборочная сверка MEDIUM/LOW с исходным кодом). Результат: 9 из 10 топ-находок подтверждены полностью с точными номерами строк; выборочные MEDIUM/LOW — точны. Применённые исправления:
1. B.2.7 (`fs::rename` на Windows) — исходная формулировка опровергнута (std использует `MoveFileExW` с `MOVEFILE_REPLACE_EXISTING`), находка понижена до INFO; в ТОП-10 заменена на B.2.6 (TOCTOU при импорте).
2. A.1.3 — убрана ложная ссылка на `parser_screen.dart:64-65` (там безопасная навигация).
3. B.1.4 — дополнена местами `connect.rs:79,83`, `import.rs:150-161`, `auth.rs:86-92` (тот же класс утечки на error-путях).
4. Сводная статистика пересчитана (Dart LOW 14, Android LOW 5, Rust MEDIUM 9, итог 56).
5. C.10 — уточнено: автостоп сервиса по событию результата уже есть (`app.dart:52-56`, `parser_screen.dart:110`).

## Общая оценка

Кодовая база **выше среднего**: единый канал связи с ядром с кольцевым лог-буфером, аккуратные `mounted`-проверки, почти везде корректные dispose, `catch_unwind` в async-диспетчере Rust, атомарные (на Linux) сохранения JSON, scoped-гарды без удержания лока через `.await`, аккуратный порт tdata с тестами. Осмысленные комментарии с причинами решений.

Системные слабости:
1. **Жизненный цикл grammers-клиентов** — `quit()` вызывается не везде, где дропаются live-клиенты (B.1.4).
2. **Отсутствие таймаутов/отмены на длинных операциях** вне auth (B.2.2, B.2.4).
3. **FFI-граница защищена от паник только в async-половине** (B.1.2, B.1.3).
4. **Recovery-контракт `PiarCore.init()` сломан** на Dart-стороне (A.2.1).
5. **Платформенная гигиена Android** — бэкап сессий (C.1), WebView-lifecycle (C.2), mixed content (C.3), debug-подпись релиза (C.4).
6. **Управление выбором в дропдаунах по Map-инстансам** — уже решённая в `parser_screen` проблема не перенесена в `piar_screen` (A.1.1).

Критических уязвимостей «прямой компрометации по сети» не найдено; главный security-риск — утечка auth-ключей через бэкап (C.1) и хранение сессий в открытом виде (B.2.10). Все найденные проблемы локальны и исправляются малыми патчами.
