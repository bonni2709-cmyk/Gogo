# Lumen Tube

Rust + Slint, Windows / Android. **Preview: сетевое ядро, поиск, Windows WebView2 и Android Media3 player для поддержанных progressive MP4.**

## Скачать проверенную CI-сборку

Исходники бинарников: `ca2c2ebd8d119e60499c7932d40a5fd751774afe`.
[Успешный workflow 42461067070](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42461067070):

- [Windows x64 — ZIP с lumen-desktop.exe](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42461067070/artifacts/13610160458) (~4.9 MB).
- [Android arm64 — ZIP с preview APK](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42461067070/artifacts/13610260643) (~14.3 MB).
- [Отформатированные исходники и Cargo.lock](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42461067070/artifacts/13610235176).

Для скачивания Actions artifacts может потребоваться вход в GitHub; у артефактов ограниченный срок хранения. Это ссылки на CI-артефакты, не постоянные GitHub Releases. Android APK подписан отладочным ключом и требует Android 8+ / arm64. EXE не имеет подписи издателя. NativeActivity и Media3 проверены в x86_64/API 35 эмуляторе на локальном тестовом MP4. **Это не проверка физического устройства, YouTube, DPI или production Rust→CDN пути.**

## Workspace

| Crate | Назначение |
|---|---|
| `lumen-core` | TLS-пробы с проверкой сертификатов, автоматический выбор транспорта |
| `lumen-proxy` | Loopback HTTP CONNECT / SOCKS5, один динамический порт |
| `lumen-ui` | Общий интерфейс Slint, проверка сети, поиск и список видео |
| `lumen-youtube` | Анонимный поиск metadata через обязательный loopback-прокси |
| `lumen-desktop` | Windows entry point |
| `lumen-player` | Встроенное окно HTML5 video / WebView2 на Windows |
| `lumen-android` | Android NativeActivity, arm64, API 26+ |

## Реализовано

При открытии приложения самотестирование запускается автоматически, без ручных сетевых параметров. При сетевой ошибке повторная проверка планируется автоматически с backoff; ручная настройка транспорта не требуется. По очереди проверяются:

1. Прямое соединение.
2. Разбиение первой TLS handshake record на записи по 32 байта с задержкой 5 мс.
3. Разбиение по 8 байт с задержкой 10 мс.

Принимается первый вариант, успешно выполнивший TLS + HTTP HEAD для `www.youtube.com` и `i.ytimg.com`. Используется настоящий hostname/SNI и проверка сертификата через rustls/webpki-roots. Проверка каждого endpoint ограничена 12 секундами. При полном отказе новых upstream-соединений нет: локальный listener остаётся на своём порту, но не принимает ни одну стратегию до успешной повторной проверки. Нет подмены сертификатов или MITM.

После успеха UI показывает выбранную стратегию и динамический loopback-порт. Один порт распознаёт SOCKS5 (только domain CONNECT/no-auth) и HTTP/1.1 CONNECT. Разрешены только YouTube-домены из списка в `lumen-core`, порт 443. Установлены ограничения заголовка, параллельных соединений и времени handshake. Обычный HTTP forwarding, UDP ASSOCIATE и аутентификация не поддерживаются. Другие процессы на устройстве могут обращаться к loopback-прокси; это не граница безопасности между локальными приложениями. DNS разрешается один раз перед соединением: пустые ответы и наборы с локальными, частными или специальными IP отклоняются целиком. Соединение использует только проверенные адреса без повторного DNS lookup; фильтр консервативный, а не полный реестр IANA.

**Важно:** несколько `write()` с TCP_NODELAY не гарантируют несколько TCP-пакетов: стек и сеть могут объединять данные. Это не IP fragmentation. В split-режиме принимается первая TLS record (до 16 KiB), не произвольный протокол.

## Чего пока нет

- Fake SNI, raw TCP sequence injection, TTL tricks и их автоматическое тестирование.
- Код ZeroDPI / sni-spoofing-rust не заимствован и совместимость с ними не заявляется. Обозначения `[reference:N]` в исходном задании не содержали URL.
- Обход настоящего IP allowlist: изменение ClientHello не делает недоступный IP доступным. Отдельный разрешённый relay/tunnel мог бы потребоваться, но не входит в эту версию.
- Проверка всех CDN/googlevideo, системные события смены сети и сохранение стратегий между запусками. Повторный подбор после наблюдаемых транспортных ошибок уже реализован, но не гарантирует непрерывное воспроизведение.
- Официальный YouTube API, история, подписки, полноценный media resolver (cipher/n/DASH/live/DRM). Media3 playback для поддержанных progressive MP4 уже реализован. Ограниченный resolver progressive MP4 описан ниже. Поиск metadata реализован через неофициальную HTML-схему, которая может измениться.
- Android VpnService, TUN forwarding и `protect()` для исключения прокси-сокетов из VPN. Media3/ExoPlayer и JNI lifecycle bridge реализованы отдельным этапом.
- mpv и внешние плеерные бинарники не используются. Windows WebView2 backend реализован; он требует установленного WebView2 Runtime. На Android Gradle APK включает Media3 backend; legacy cargo-apk вариант — только диагностику.

**Не обещаем обход любого DPI или «белых списков».** Успешная HEAD-проба не означает доступность видео. На Android сейчас работает только собственный loopback-сервис; системный трафик он не перехватывает. Windows media gateway автоматически направляет CDN-запросы через выбранный транспорт; Android Media3 также получает данные только из локального gateway.

## Сборка и проверки — только CI

Локальные тяжёлые сборки не запускались. В рабочей среде отсутствует Rust toolchain. Первый CI-прогон исходного ядра успешно выполнил тесты и Clippy; расширенные TLS/proxy-тесты и Clippy также прошли в запуске [42084805927](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42084805927). Сборки платформ отслеживаются отдельно. Workflow `.github/workflows/build.yml`:

- форматирует исходники в checkout CI, проверяет форматирование, запускает тесты и Clippy сетевых crates;
- собирает Windows MSVC release `.exe`;
- компилирует Android arm64 release библиотеку через cargo-ndk, затем собирает **debug-signed Media3 preview APK** через Gradle, включая JVM unit tests и lint;
- сохраняет бинарники, отформатированные исходники и разрешённый Cargo.lock как Actions artifacts на странице запуска.

Rust stable; Slint закреплён на 1.9.2. Cargo.lock создан в CI и сохранён в артефактах, но пока не добавлен в репозиторий. Скачать его в песочницу не удалось из-за недоступности внешнего хранилища Actions; последующие сборки без lockfile могут получить другие транзитивные версии. Следующий шаг — сохранить проверенный lockfile. Автоматическое форматирование CI не меняет исходники ветки.

Команды для CI/полноценной машины разработчика:

```sh
cargo fmt --all
cargo test -p lumen-core -p lumen-proxy -p lumen-youtube
cargo clippy -p lumen-core -p lumen-proxy -p lumen-youtube --all-targets -- -D warnings
cargo build -p lumen-desktop --release --target x86_64-pc-windows-msvc
cargo ndk -t arm64-v8a -p 26 -o android/app/src/main/jniLibs build -p lumen-android --release
(cd android && gradle --no-daemon :app:testDebugUnitTest :app:assembleDebug :app:lintDebug)
```

Android SDK/NDK/JDK настраиваются workflow. Preview APK подписывается отладочным ключом CI; для публикации нужен отдельный безопасно хранимый release key. Runtime на реальных Windows/Android ещё не проверен. EXE и APK успешно собраны; ссылки приведены выше. Загрузка в Drive не требуется и не настроена.

## Следующие этапы

1. CI зелёный: теперь зафиксировать проверенный Cargo.lock и проверить APK/EXE на устройствах.
2. Расширить существующие тесты настоящего TLS на loopback: добавить смену сетевых условий и проверку ошибок сертификатов.
3. Проверить metadata provider и Windows WebView2 playback на реальной сети; расширить resolver для современных YouTube-форматов.
4. Проверить Android Media3 на телефоне (seek, фон/возврат, Back, expired URL); затем VpnService с явным системным согласием и обработкой lifecycle.
5. Реализовывать привилегированные платформенные методы только с отдельным capability detection и сетевыми испытаниями.

## Лицензирование

Собственный код — MIT, см. LICENSE. Slint имеет отдельные лицензионные условия (включая условия royalty-free для desktop/mobile); перед распространением проверьте их применимость. Android SDK и будущие media-компоненты имеют собственные лицензии.

### Текущий статус CI

Первый запуск: [42084433135](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42084433135) — исходные core tests и Clippy прошли; Android остановился на настройке SDK до компиляции. Linux runner закреплён на Ubuntu 24.04, используется предустановленный SDK вместо отдельного setup action. В запуске 42084805927 расширенные core/proxy tests прошли, но Android остановился на sdkmanager. Добавлены явный путь к инструменту, Java 17 и аннотации SDK-ошибок; эти изменения требуют подтверждения новым CI-прогоном.

В последующие изменения добавлены локальные TLS roundtrip-тесты всех стратегий, отказ при недоверенном сертификате, запрет пересылки не-TLS данных в split-режиме, проверки частных IP/DNS и запрещённых CONNECT/SOCKS5-направлений. Они не требуют внешнего YouTube и выполняются в CI. Статус актуальной версии смотрите в [Actions](https://github.com/bonni2709-cmyk/Gogo/actions).


## Поиск видео (новый этап)

После успешного самотестирования доступны поле запроса и список результатов: заголовок, канал, длительность, кнопка проверки медиапотока. Внешний браузер не запускается, автоматическое прямое соединение не выполняется. На Windows доступна кнопка Play через WebView2; на Android Gradle APK — Play через Media3.

`lumen-youtube` выполняет HTTPS GET `/results?search_query=...` через созданный приложением CONNECT-прокси. Системные proxy settings игнорируются. Нет API-ключа, логина или передачи cookie. Redirect, consent и challenge страницы не обходятся — показывается ошибка. Если прокси недоступен, fallback на прямую сеть запрещён. Ввод ограничен 200 символами, ответ 4 MiB, запрос 25 секундами, выдача 30 уникальными video ID. Полученные названия показываются как обычный текст, не HTML.

Извлекается `ytInitialData` из desktop HTML. Это не официальный стабильный API: изменение схемы, региональный consent или ограничения YouTube могут сделать поиск недоступным даже при успешном сетевом тесте. Отдельные offline fixtures покрывают дедупликацию, невалидные ID, пустую выдачу, изменённую схему, лимит ответа и обязательность proxy. Сетевая работоспособность поиска на реальном YouTube пока не подтверждена.

UI использует один сетевой worker с ограниченной очередью команд. Поиск не блокирует графический поток; повторный запрос блокируется до завершения предыдущего. Proxy task завершается при закрытии UI/worker и отслеживается на предмет неожиданной остановки.


### Подтверждение текущей сборки

Запуск **42085840430** завершён с `success`: 15 offline-тестов (ядро, прокси, поиск), Clippy, проверка типов Slint UI, Windows release EXE, Android release Rust library через cargo-ndk и упаковка debug-signed preview APK. Исправление sdkmanager подтверждено. End-to-end поиск на YouTube и запуск на устройствах ещё не проверялись.

## Подготовка медиапотоков (следующий этап)

Кнопка **Check stream** в результатах получает watch page через тот же loopback-прокси и извлекает `ytInitialPlayerResponse`. Принимаются только прямые progressive MP4 (AVC + AAC) из `streamingData.formats`. Проверяются video ID, playability status, HTTPS/googlevideo endpoint и срок действия URL. URL с `n`, cipher/signatureCipher, DRM, live и adaptive-only выдачи не поддерживаются: приложение сообщает об ограничении, не пытаясь выдавать их за рабочие.

Для первых трёх подходящих форматов последовательно выполняется реальный GET с `Range: bytes=0-0`. Успех требует HTTP 206, корректного Content-Range, типа video/mp4 и ровно одного полученного байта. Redirect и прямой fallback запрещены. Signed media URLs не показываются в UI и убираются из сетевых ошибок; формат не реализует Debug, чтобы случайно не записать URL в лог. Общий таймаут подготовки в UI — 75 секунд. Проверка байта подтверждает доступность CDN на момент запроса, **не декодируемость или полное воспроизведение**.

Сам resolver — диагностический/подготовительный слой. Windows WebView2 backend и передача медиаданных через Rust gateway добавлены следующим этапом; Android Media3 backend добавлен следующим этапом. На современном YouTube прямая progressive выдача может отсутствовать; полноценная поддержка JS signature/n transforms и DASH не реализована.

Дополнительно исправлена CDN-проба самотестирования: вместо корня `i.ytimg.com` проверяется конкретный thumbnail path. HTTP 404 по-прежнему считается ошибкой; это не ослабление проверки доступности.


### Подтверждение этапа media diagnostics

[CI 42087540673](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42087540673) завершился успешно: **20 тестов**, Clippy, type-check общего Slint UI и обе платформенные сборки. Ссылки в начале README обновлены на этот запуск. Проверка реального YouTube/CDN из песочницы недоступна, поэтому успешное end-to-end получение видео не заявляется. Запуск APK/EXE и работа player backend на устройствах также пока не проверены.

## Windows player и Rust media gateway

Новый crate `lumen-player` открывает отдельное встроенное окно WebView2 на Windows: HTML5 `<video>` с play/pause/seek/volume. Требуется установленный **Microsoft Edge WebView2 Runtime**; он не скачивается приложением автоматически. Если runtime отсутствует, UI сообщает об ошибке. Внешние mpv, браузер и прокси-бинарники не запускаются.

На Windows кнопка результата называется **Play**. Приложение выбирает поддержанный progressive MP4, проверяет CDN, создаёт `PlaybackSession` и передаёт окну только адрес `http://127.0.0.1:<port>/<random-capability>/media`. Ключ сессии содержит 256 случайных бит; URL подписанного CDN остаётся внутри Rust. Gateway ограничен четырьмя активными медиазапросами, проверяет Host и одиночный byte Range, не следует redirect, копирует только Content-Type/Length/Range и не отдаёт CDN cookies/прочие заголовки. Данные передаются потоково с backpressure и ограничением ожидаемой длины, а не загружаются целиком в RAM. Idle timeout — 20 секунд; для длинного видео нет общего 25-секундного timeout метаданных. Переходы к внешним страницам, новые окна, JS и удалённые ресурсы player-документа запрещены. Управление браузерным runtime и его фоновыми службами не входит в Rust transport.

Закрытие player-окна отменяет сессию и закрывает listener/активные медиазапросы. Одновременно разрешено одно player-окно. Срок подписанного URL не обновляется автоматически: после его истечения новые Range-запросы завершатся ошибкой, потребуется повторно открыть видео. UI подтверждает создание окна, **не успешное декодирование**; ошибки самого `<video>` отображаются его нативными controls. Автозапуск может быть ограничен политикой WebView2 — используйте Play в окне плеера.

Android Gradle APK теперь использует **Play** через Media3; прочие/legacy пакеты сохраняют **Check stream**. VpnService пока не реализован. Windows playback не снимает ограничений resolver: cipher/n-transform/DASH/live/DRM не поддерживаются. Реальный запуск WebView2, seek и end-to-end YouTube нужно проверить на Windows-устройстве отдельно от компиляции CI.


### Подтверждение Windows playback build

[CI 42089199084](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42089199084) завершён с `success`: **24 теста**, Clippy сетевых crates/player stub, Linux type-check Slint UI, Windows release EXE с Wry/Tao backend и Android preview APK. Windows backend компилируется в Windows job; Linux Clippy не проверяет Windows-specific код. Устройство для end-to-end проверки воспроизведения, seek и lifecycle пока не использовалось.

Быстрая проверка на Windows: установить WebView2 Runtime при необходимости, распаковать EXE, запустить, дождаться сетевой проверки, найти видео и нажать Play. Если доступна поддерживаемая прямая progressive выдача, откроется встроенное окно video. В случае неподдерживаемой выдачи приложение сообщит об ограничении. Наличие окна не гарантирует, что кодек/поток успешно декодируется.

## Android Media3 host (новый этап)

Добавлена Gradle-обёртка `android/`: `MainActivity extends NativeActivity` запускает Slint, отдельная **неэкспортируемая** `PlayerActivity` — Media3/ExoPlayer 1.5.1 с нативными controls. В новом APK кнопка результата — **Play**. Rust JNI bridge хранит ровно одну `PlaybackSession` и передаёт Java только loopback URL; signed CDN URL остаётся в Rust.

`LoopbackDataSource` принимает ровно один адрес сессии `http://127.0.0.1:<port>/<64 hex>/media`, игнорирует системный proxy, запрещает redirect и любые другие URL, поддерживает последовательное чтение/Range/seek. Это Java HTTP только до локального Rust gateway; все внешние YouTube/CDN-запросы выполняет Rust через встроенный прокси. Android network security config разрешает cleartext только для 127.0.0.1, глобально cleartext запрещён. Строгое требование «даже loopback HTTP только на Rust» потребовало бы отдельного JNI DataSource вместо этой локальной обёртки.

При уходе в фон ExoPlayer освобождается, позиция сохраняется; при возврате Activity player создаётся заново, пока сессия действительна. При закрытии Activity/ошибке playback JNI освобождает Rust-сессию и закрывает gateway. Неизвестный/stale session ID не может закрыть новую сессию. Если Android уничтожит весь процесс, старый URL не восстанавливается — видео нужно открыть снова. Background playback, PiP, audio focus customization, обновление expired URL и Android VpnService пока не реализованы.

Основной Android CI теперь собирает Rust через `cargo-ndk -o android/app/src/main/jniLibs`, затем Gradle 8.11.1 / AGP 8.9.1 / JDK 17 выполняет JVM unit tests, assembleDebug и Android lint. APK по-прежнему debug-signed, arm64, API 26+. Локальные тяжёлые сборки не запускались. Legacy cargo-apk путь не включает Java/Media3 и остаётся только диагностическим вариантом. Реальное воспроизведение, переходы фон/возврат и lifecycle требуют проверки на телефоне.


### Проверенная сборка Android Media3

[CI 42192758983](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42192758983) — `success`: 24 Rust-теста, Clippy сетевых crates, Slint type-check, JNI cross-compile arm64, JVM-тест LoopbackPolicy, Android assembleDebug/lintDebug и Windows release build. JNI-код компилируется для Android, но его вызовы и lifecycle ещё не проверены на устройстве. Воспроизведение и доступность YouTube не подтверждены этим CI.

#### Чек-лист на телефоне

1. Распаковать APK из артефакта и установить на Android 8+ / arm64. Если старая preview-версия не обновляется из-за другой debug-подписи/версии, удалить её перед установкой (данные старой установки будут удалены).
2. Дождаться самотестирования, выполнить поиск и нажать Play. При неподдерживаемом cipher/n/DASH должно появиться понятное сообщение, а не неработающий player.
3. Для поддержанного MP4 проверить старт, звук, pause/resume, перемотку, смену ориентации.
4. Уйти в фон и вернуться: воспроизведение в фоне должно остановиться, при возврате позиция восстановиться, если URL ещё действителен.
5. Нажать Back, открыть другое видео и убедиться, что старая сессия не блокирует новый player.
6. Отключить сеть во время playback: проверить сообщение об ошибке и возможность повторного открытия. После уничтожения процесса не ожидать восстановления старого loopback URL.

На физическом телефоне этот чек-лист **ещё не выполнен**. Часть lifecycle/playback проверок успешно автоматизирована в эмуляторе, как описано ниже. APK отладочно подписан, VpnService не запрашивается и системный трафик не перехватывается.

## Android runtime smoke в эмуляторе

Добавлен отдельный workflow `.github/workflows/android-smoke.yml`: x86_64/API 35 emulator с KVM, отдельная x86_64 Rust-библиотека и instrumentation APK. Публикуемый arm64 APK не меняет ABI и не содержит тестовое видео/тестовый сервер. Короткий H.264/AAC MP4 синтезируется через ffmpeg **только в CI** — runtime-зависимости приложения от ffmpeg нет.

Instrumentation smoke запускает настоящую MainActivity (NativeActivity/Slint), затем PlayerActivity, ожидает Media3 READY/playing, выполняет seek и проверяет release/recreate при stop/resume. Закрытие Activity вызывает настоящий JNI close callback с неизвестным session ID. Отчёты instrumentation и logcat сохраняются отдельным Actions artifact.

Границы теста: fixture отдаёт instrumentation-only Java loopback server, **не production Rust media gateway**. Этот сценарий не проверяет Rust→Java openPlayer вызов, signed CDN URLs, YouTube, DPI или реальные устройства. Он дополняет Rust unit tests шлюза и проверки сборки, а не заменяет end-to-end испытания. Успешный smoke workflow приведён ниже; на первом запуске была исправлена ошибка тестового harness с ActivityScenario CLEAR_TASK, а не ослаблены playback-проверки.


### Подтверждённый Android runtime smoke

- [Android runtime smoke 42197900164 — success](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42197900164).
- [JUnit-отчёт и logcat](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42197900164/artifacts/13493447207).
- [Основная сборка 42197900196 — success](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42197900196): Rust tests/Clippy, Slint, JVM unit test/lint, Windows EXE и Android arm64 APK.

В эмуляторе API 35/x86_64 подтверждены: запуск NativeActivity, Media3 READY + isPlaying для синтетического H.264/AAC MP4, seek, release ExoPlayer при stop, recreate/playing при resume и JNI close callback с неизвестным ID при закрытии player. Сам тест не вызывает production Rust→Java `openPlayer` и не использует production Rust gateway.

Первый прогон выявил ошибку тестового harness: `ActivityScenario.launch()` очищает предыдущий task (`NEW_TASK | CLEAR_TASK`), после чего обращение к прежней Activity через `onActivity` недопустимо. Тест теперь явно ожидает `DESTROYED` для прежнего host, сохраняя все проверки Media3. Это не доказательство поведения обычного перехода MainActivity→PlayerActivity в одном task; его ещё нужно проверить отдельно.

## Интеграционные тесты Rust gateway / CONNECT / TLS

Добавлен полностью локальный integration fixture: production `PlaybackSession` и reqwest-клиент → тестовый CONNECT-прокси → TLS origin с проверяемым сертификатом `rr1.googlevideo.com`. Proxy fixture перенаправляет только этот CONNECT на локальный TLS listener. Исключений для localhost/private IP в production DNS/allowlist не добавлено; доверенный тестовый сертификат существует только в `#[cfg(test)]`.

Покрытие: полное чтение MP4-подобных байтов, HEAD без body, bounded/open/suffix Range для seek, отказ при недоверенном сертификате, запрет CDN redirect, обрыв неполного body и отмена активного stalled stream при Drop сессии. Проверяется, что cookies/служебные CDN headers не передаются плееру. Это не настоящий MP4 decode-тест (его отдельно выполняет Android emulator smoke).

Исправление production gateway: Content-Range теперь сопоставляется с **исходным запросом**, а не проверяется только на внутреннюю согласованность. Шлюз отвергает другой диапазон, дублирующий Content-Range, partial headers на полном ответе и encoded body. Граничные диапазоны вычисляются без переполнения с учётом размера объекта. Если CDN вернул `bytes 0-9/100` на запрос `bytes=10-19`, плеер получит 502 без ошибочных медиабайтов.

Граница покрытия: CONNECT-прокси в этом fixture — тестовая реализация, **не** crate `lumen-proxy`. Production CONNECT/SOCKS handlers и transport core имеют отдельные тесты. Доступность YouTube, фактический DPI и физические устройства этими тестами не подтверждаются. Новые тесты выполнены успешно в CI 42395024107; подробности приведены ниже.


### Подтверждение CONNECT/TLS и строгой проверки Range

- [Основной CI 42395024107 — success](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42395024107): **32 Rust-теста**, Clippy, Slint type-check, JVM unit test/lint и обе платформенные сборки.
- [Android regression smoke 42395023983 — success](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42395023983): повторно подтверждены запуск NativeActivity, Media3 playback/seek и stop/resume на локальном MP4 в API 35/x86_64 эмуляторе.
- [Отчёты и logcat](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42395023983/artifacts/13583540931).

Новые TLS-тесты не ослабляют production certificate verification или DNS-политику: тестовый root certificate и локальное перенаправление CONNECT находятся исключительно в test fixture. Это проверка production gateway и reqwest-транспорта с test proxy, а не реального YouTube или полного production proxy/DPI пути. Поддержка cipher/n/DASH/live/DRM и испытания на физическом телефоне остаются отдельными задачами.

## Автоматическое восстановление сети (новый этап)

UI worker теперь создаёт один loopback listener и сохраняет его порт на протяжении работы. Динамический proxy controller публикует только проверенную стратегию. При отсутствии стратегии новые CONNECT/SOCKS5-запросы отклоняются **до DNS/upstream connect**; уже открытые TCP-туннели не меняют стратегию задним числом.

После неудачного стартового самотестирования приложение повторяет проверку автоматически: 5 → 10 → 20 → 40 → 60 секунд, затем максимум 60 секунд между неудачными попытками (плюс длительность самих проб). После успешной проверки повторный сетевой сбой не вызывает сканирование раньше чем через 30 секунд после последней успешной проверки. Повторные уведомления не продлевают дедлайн и не создают горячий цикл.

Reqwest connection/request/timeout/body errors поиска, подготовки видео и streaming gateway уведомляют controller без передачи URL/секретов. Приложение блокирует новые действия UI, убирает проверенную стратегию, выполняет повторное самотестирование и восстанавливает доступность поиска после успеха. HTTP 403/404, изменившаяся HTML-схема, неподдерживаемый cipher/n/DASH, неправильный Content-Range и намеренное закрытие player-сессии не запускают rescan.

Ограничения: это восстановление транспорта по наблюдаемым ошибкам, **не системный монитор смены Wi-Fi/mobile**. Рабочий старый TCP/TLS pool не закрывается принудительно; новые соединения получают новую стратегию. Запрос поиска не повторяется автоматически, неудавшийся player не открывается заново, позиция видео и signed URL не восстанавливаются автоматически. Если CDN недоступен, а HEAD-пробы YouTube/thumbnail успешны, выбор стратегии всё ещё не доказывает доступность конкретного CDN. Ошибки локального bind или завершение proxy service являются отдельными фатальными ошибками worker, а не бесконечно повторяемой сетевой проверкой.

Добавлены offline-тесты backoff/cooldown, обновления стратегии, запрета соединений при recovery, стабильного listener и завершения незаконченных handshake при остановке proxy. TLS integration fixtures проверяют, что обрыв body сигнализирует recovery, а HTTP/schema/range errors и намеренная отмена — нет. Новые проверки подтверждены CI 42461067070; результаты ниже.


### Подтверждённая сборка automatic recovery

- [Основной CI 42461067070 — success](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42461067070): **41 Rust-тест**, Clippy, Slint type-check, JVM test/Android lint, Windows EXE и Android arm64 APK.
- [Android regression smoke 42461067065 — success](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42461067065): NativeActivity и локальный Media3 playback/seek/stop/resume.
- [Smoke reports и logcat](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42461067065/artifacts/13609822286).

Первый прогон нового набора выявил пропущенное уведомление о truncated body: reqwest может классифицировать такую ошибку как decode, а не body. Исправлено на уровне чтения тела: transport errors и premature EOF с оставшимися ожидаемыми байтами уведомляют recovery controller независимо от категории reqwest. Проверка не была ослаблена.

Backoff/state/notifications проверены локальными Rust-тестами; Android smoke по-прежнему не имитирует смену реальной Wi-Fi/mobile сети и не проверяет восстановление YouTube playback после такого перехода. Физические устройства и конкретные DPI-сети остаются непроверенными.
