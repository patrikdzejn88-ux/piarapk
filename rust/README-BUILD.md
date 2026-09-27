# Сборка piarcore и встраивание в бандл приложения

Crate `piarcore` (`rust/Cargo.toml`, `crate-type = ["cdylib", "lib"]`) собирается
cargo'ом и кладётся в бандл приложения. Dart загружает библиотеку в рантайме
через `dart:ffi` (`DynamicLibrary.open`) — линковки на этапе сборки нет, нужно
только наличие файла в бандле.

## Артефакты

| Платформа | Команда (из корня репо) | Артефакт | Куда попадает |
|---|---|---|---|
| Windows | `cargo build --release --manifest-path rust/Cargo.toml` | `rust/target/release/piarcore.dll` | рядом с `piarapk.exe` |
| macOS | та же | `rust/target/release/libpiarcore.dylib` | `piarapk.app/Contents/Frameworks/` |

Путь артефакта — стандартный cargo-путь; переопределение `CARGO_TARGET_DIR`
не поддерживается.

## Как встроено в сборку Flutter

**Windows** — `windows/CMakeLists.txt` (в конце файла, секция "Rust library"):
custom target `piarapk_rust` вызывает `cargo build --release
--manifest-path rust/Cargo.toml` (рабочий каталог — корень репо), от него через
`add_dependencies` зависит target `piarapk`; POST_BUILD-команда копирует
`rust/target/release/piarcore.dll` в `$<TARGET_FILE_DIR:piarapk>` —
configuration-специфичный каталог exe для multi-config генератора Visual Studio
(Release-профиль cargo используется для всех режимов Flutter). Если cargo не
найден ни в PATH, ни в `%USERPROFILE%\.cargo\bin`, configure падает с понятным
`FATAL_ERROR`.

**macOS** — `macos/Runner.xcodeproj/project.pbxproj`: у target Runner добавлена
script build phase «Build Rust Library» (`alwaysOutOfDate = 1`, /bin/sh):
`export PATH="$HOME/.cargo/bin:$PATH"`, проверка наличия cargo, `cargo build
--release` в `$SRCROOT/../rust`, копирование `libpiarcore.dylib` в
`$BUILT_PRODUCTS_DIR/$PRODUCT_NAME.app/Contents/Frameworks/` и ad-hoc codesign
(`codesign --force --sign -`). Отдельная PBXCopyFilesBuildPhase не нужна —
dylib грузится Dart'ом по абсолютному пути (относительно executable), а не
через dyld-поиск.

## Локальная пересборка (на машине с рабочим Rust)

Из корня репозитория:

    cargo build --release --manifest-path rust/Cargo.toml
    cargo test --manifest-path rust/Cargo.toml

- Windows: `flutter build windows` (и `flutter run -d windows`) сам вызовет
  cargo через CMake-цель и скопирует DLL рядом с exe.
- macOS: `flutter build macos` (и `flutter run -d macos`) вызовет Xcode-фазу
  автоматически.

## CI (Codemagic)

`codemagic.yaml` ставит rustup (шаг «Install Rust toolchain»), гоняет
`cargo test` (шаг «Rust unit tests»), а сама библиотека для `.app` собирается
Xcode-фазой во время `flutter build macos --release`. Кэшируются `~/.cargo` и
`rust/target`. Локально на машине с SAC (Smart App Control) Rust не собирается —
сборка только в CI.
