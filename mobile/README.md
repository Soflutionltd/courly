# Courly mobile

The Courly app for iOS (Android later), written in Flutter. The screens are
Dart; everything else (protocols, local database, stores, sync) is the shared
Courly Rust core, linked into the app through `crates/mobile-bridge` with
flutter_rust_bridge. There is no separate Service process on mobile.

## Layout

- `lib/` - Flutter screens. `lib/src/rust/` is generated, never edit it.
- `rust_builder/` - Cargokit plugin that compiles `crates/mobile-bridge` for
  each phone architecture during the Flutter build.
- `../crates/mobile-bridge/` - the Rust functions Dart calls (`src/api/`).

## Toolchain

- Flutter stable and Rust stable with the `aarch64-apple-ios` and
  `aarch64-apple-ios-sim` targets.
- `flutter_rust_bridge_codegen` at the exact version pinned in `pubspec.yaml`.

## Commands (run from `mobile/`)

- Regenerate the bridge after changing `crates/mobile-bridge/src/api/`:
  `flutter_rust_bridge_codegen generate`
- Static checks: `flutter analyze`
- Device test on a simulator: `flutter test integration_test -d <simulator id>`
