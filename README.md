# VOXEL KNIGHT

Классический тёмный 2D-платформер (метроидвания) на **Rust + Macroquad**,
целевое устройство — Android (Infinix Note 30, MediaTek Helio G99, ARM64),
стабильные 60 FPS.

Стиль: тёмное фэнтези — подземелья, руины, факелы, землистая палитра,
пиксель-арт без сглаживания, многослойный параллакс. Неон и кислотные
цвета исключены.

## Модули

| Модуль | Назначение |
| --- | --- |
| `src/physics.rs` | AABB-коллизии с разделением осей (X → Y), подшаги против туннелирования, гравитация, broad-phase-сетка, рейкасты |
| `src/player.rs` | Контроллер: кривые разгона, Coyote Time, Jump Buffer, Variable Jump Height, Wall Slide / Wall Jump, Dash с i-frames, здоровье и i-frames урона |
| `src/ecs/mod.rs` | Лёгкий ECS: сущности с поколениями, типизированные хранилища компонентов (Transform / Velocity / Health) |
| `src/enemies/` | ИИ врагов на FSM поверх ECS: слизень (погоня + прыжок-выпад), летучая мышь (парение/охота), скелет-мечник (замах → выпад → откат); агро по дистанции и линии взгляда, разворот у обрывов, контактный урон |
| `src/game.rs` | Главный цикл с фиксированным шагом 1/60, демо-уровень с врагами, камера, параллакс, факелы, HUD (сердца) |
| `src/lib.rs` | Точка входа игры и Android-символ `quad_main` |
| `android/java/` | Java-glue miniquad (MainActivity + QuadNative), вендорен из upstream |
| `.github/workflows/build.yml` | Сборка подписанного APK в CI |

Следующие этапы по плану проекта: боевая система игрока (комбо, оружие,
парирование), генерация уровней (TMX/JSON), частицы, тач-интерфейс
(джойстик + кнопки) и инвентарь.

## Управление (десктопная отладка)

* **A/D** или **←/→** — бег;
* **SPACE / W / ↑** — прыжок (длина зависит от времени удержания);
* **SHIFT / X** — рывок (в воздухе — один, после приземления снова доступен).

## Сборка APK (GitHub Actions)

Протокол: в репозитории хранится только исходный код; APK собирается в CI.

1. `cargo ndk -t arm64-v8a build --release --lib` — кросс-компиляция под arm64;
2. `cargo apk build --release --lib` — упаковка и подпись;
3. Java-глue miniquad компилируется (`javac` + `d8`) и внедряется в APK,
   после чего пакет выравнивается (`zipalign`) и переподписывается
   release-ключом (`apksigner`);
4. keystore генерируется в CI (`keytool`) и в репозиторий не коммитится;
5. готовый APK публикуется как артефакт сборки и в GitHub Release
   `apk-latest`.

Локально (нужны Android SDK/NDK и JDK):

```sh
cargo install cargo-ndk cargo-apk
cargo ndk -t arm64-v8a build --release --lib
cargo apk build --release --lib
```

## Десктопная разработка

```sh
# системные зависимости macroquad (Linux):
# pkg-config libx11-dev libxi-dev libgl1-mesa-dev libasound2-dev
cargo run
cargo test --lib   # юнит-тесты физики и контроллера
```

## Лицензия

MIT OR Apache-2.0. Java-файлы в `android/java/` вендорены из
[miniquad](https://github.com/not-fl3/miniquad) (MIT/Apache-2.0).
