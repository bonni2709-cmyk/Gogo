# Lumen Tube

Rust + Slint, Windows / Android. **Preview: сетевое ядро, поиск и Windows WebView2 player для поддержанных progressive MP4. Android playback пока не реализован.**

## Скачать проверенную CI-сборку

Исходники бинарников: `c2b4aa970430aeee2b38ffa87087b7e2678c5589`.
[Успешный workflow 42089199084](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42089199084):

- [Windows x64 — ZIP с lumen-desktop.exe](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42089199084/artifacts/13445411045) (~4.9 MB).
- [Android arm64 — ZIP с preview APK](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42089199084/artifacts/13444273359) (~90 MB).
- [Отформатированные исходники и Cargo.lock](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42089199084/artifacts/13445125233).

Для скачивания Actions artifacts может потребоваться вход в GitHub; у артефактов ограниченный срок хранения. Это ссылки на CI-артефакты, не постоянные GitHub Releases. Android APK подписан отладочным ключом и требует Android 8+ / arm64. EXE не имеет подписи издателя. Успех сборки **не означает проверку запуска, поиска или обхода DPI на реальном устройстве**.

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

При открытии приложения самотестирование запускается автоматически, без ручных сетевых параметров. После ошибки кнопка позволяет повторить проверку. По очереди проверяются:

1. Прямое соединение.
2. Разбиение первой TLS handshake record на записи по 32 байта с задержкой 5 мс.
3. Разбиение по 8 байт с задержкой 10 мс.

Принимается первый вариант, успешно выполнивший TLS + HTTP HEAD для `www.youtube.com` и `i.ytimg.com`. Используется настоящий hostname/SNI и проверка сертификата через rustls/webpki-roots. Проверка каждого endpoint ограничена 12 секундами. При полном отказе возвращается ошибка, прокси не запускается. Нет подмены сертификатов или MITM.

После успеха UI показывает выбранную стратегию и динамический loopback-порт. Один порт распознаёт SOCKS5 (только domain CONNECT/no-auth) и HTTP/1.1 CONNECT. Разрешены только YouTube-домены из списка в `lumen-core`, порт 443. Установлены ограничения заголовка, параллельных соединений и времени handshake. Обычный HTTP forwarding, UDP ASSOCIATE и аутентификация не поддерживаются. Другие процессы на устройстве могут обращаться к loopback-прокси; это не граница безопасности между локальными приложениями. DNS разрешается один раз перед соединением: пустые ответы и наборы с локальными, частными или специальными IP отклоняются целиком. Соединение использует только проверенные адреса без повторного DNS lookup; фильтр консервативный, а не полный реестр IANA.

**Важно:** несколько `write()` с TCP_NODELAY не гарантируют несколько TCP-пакетов: стек и сеть могут объединять данные. Это не IP fragmentation. В split-режиме принимается первая TLS record (до 16 KiB), не произвольный протокол.

## Чего пока нет

- Fake SNI, raw TCP sequence injection, TTL tricks и их автоматическое тестирование.
- Код ZeroDPI / sni-spoofing-rust не заимствован и совместимость с ними не заявляется. Обозначения `[reference:N]` в исходном задании не содержали URL.
- Обход настоящего IP allowlist: изменение ClientHello не делает недоступный IP доступным. Отдельный разрешённый relay/tunnel мог бы потребоваться, но не входит в эту версию.
- Проверка всех CDN/googlevideo, адаптация к смене сети, кэширование стратегий и автоматическое переключение после отказа активного транспорта.
- Официальный YouTube API, история, подписки, полноценный media resolver и Android-воспроизведение. Ограниченный resolver progressive MP4 описан ниже. Поиск metadata реализован через неофициальную HTML-схему, которая может измениться.
- Android VpnService, TUN forwarding, `protect()` для исключения прокси-сокетов из VPN и JNI-интеграция Media3/ExoPlayer.
- mpv и внешние плеерные бинарники не используются. Windows WebView2 backend реализован; он требует установленного WebView2 Runtime. На Android нативного player backend пока нет.

**Не обещаем обход любого DPI или «белых списков».** Успешная HEAD-проба не означает доступность видео. На Android сейчас работает только собственный loopback-сервис; системный трафик он не перехватывает. Windows media gateway автоматически направляет CDN-запросы через выбранный транспорт; Android media integration пока впереди.

## Сборка и проверки — только CI

Локальные тяжёлые сборки не запускались. В рабочей среде отсутствует Rust toolchain. Первый CI-прогон исходного ядра успешно выполнил тесты и Clippy; расширенные TLS/proxy-тесты и Clippy также прошли в запуске [42084805927](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42084805927). Сборки платформ отслеживаются отдельно. Workflow `.github/workflows/build.yml`:

- форматирует исходники в checkout CI, проверяет форматирование, запускает тесты и Clippy сетевых crates;
- собирает Windows MSVC release `.exe`;
- компилирует Android arm64 release библиотеку через cargo-ndk, отдельно упаковывает **debug-signed preview APK** через cargo-apk;
- сохраняет бинарники, отформатированные исходники и разрешённый Cargo.lock как Actions artifacts на странице запуска.

Rust stable; Slint закреплён на 1.9.2. Cargo.lock создан в CI и сохранён в артефактах, но пока не добавлен в репозиторий. Скачать его в песочницу не удалось из-за недоступности внешнего хранилища Actions; последующие сборки без lockfile могут получить другие транзитивные версии. Следующий шаг — сохранить проверенный lockfile. Автоматическое форматирование CI не меняет исходники ветки.

Команды для CI/полноценной машины разработчика:

```sh
cargo fmt --all
cargo test -p lumen-core -p lumen-proxy -p lumen-youtube
cargo clippy -p lumen-core -p lumen-proxy -p lumen-youtube --all-targets -- -D warnings
cargo build -p lumen-desktop --release --target x86_64-pc-windows-msvc
cargo ndk -t arm64-v8a -p 26 build -p lumen-android --release
cargo apk build -p lumen-android --target aarch64-linux-android
```

Android SDK/NDK/JDK настраиваются workflow. Preview APK подписывается отладочным ключом CI; для публикации нужен отдельный безопасно хранимый release key. Runtime на реальных Windows/Android ещё не проверен. EXE и APK успешно собраны; ссылки приведены выше. Загрузка в Drive не требуется и не настроена.

## Следующие этапы

1. CI зелёный: теперь зафиксировать проверенный Cargo.lock и проверить APK/EXE на устройствах.
2. Расширить существующие тесты настоящего TLS на loopback: добавить смену сетевых условий и проверку ошибок сертификатов.
3. Проверить metadata provider и Windows WebView2 playback на реальной сети; расширить resolver для современных YouTube-форматов.
4. Добавить Android Media3 backend с маршрутизацией через Rust gateway; затем VpnService с явным системным согласием и обработкой lifecycle.
5. Реализовывать привилегированные платформенные методы только с отдельным capability detection и сетевыми испытаниями.

## Лицензирование

Собственный код — MIT, см. LICENSE. Slint имеет отдельные лицензионные условия (включая условия royalty-free для desktop/mobile); перед распространением проверьте их применимость. Android SDK и будущие media-компоненты имеют собственные лицензии.

### Текущий статус CI

Первый запуск: [42084433135](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42084433135) — исходные core tests и Clippy прошли; Android остановился на настройке SDK до компиляции. Linux runner закреплён на Ubuntu 24.04, используется предустановленный SDK вместо отдельного setup action. В запуске 42084805927 расширенные core/proxy tests прошли, но Android остановился на sdkmanager. Добавлены явный путь к инструменту, Java 17 и аннотации SDK-ошибок; эти изменения требуют подтверждения новым CI-прогоном.

В последующие изменения добавлены локальные TLS roundtrip-тесты всех стратегий, отказ при недоверенном сертификате, запрет пересылки не-TLS данных в split-режиме, проверки частных IP/DNS и запрещённых CONNECT/SOCKS5-направлений. Они не требуют внешнего YouTube и выполняются в CI. Статус актуальной версии смотрите в [Actions](https://github.com/bonni2709-cmyk/Gogo/actions).


## Поиск видео (новый этап)

После успешного самотестирования доступны поле запроса и список результатов: заголовок, канал, длительность, кнопка проверки медиапотока. Внешний браузер не запускается, автоматическое прямое соединение не выполняется. На Windows доступна кнопка Play через WebView2; на Android — только Check stream.

`lumen-youtube` выполняет HTTPS GET `/results?search_query=...` через созданный приложением CONNECT-прокси. Системные proxy settings игнорируются. Нет API-ключа, логина или передачи cookie. Redirect, consent и challenge страницы не обходятся — показывается ошибка. Если прокси недоступен, fallback на прямую сеть запрещён. Ввод ограничен 200 символами, ответ 4 MiB, запрос 25 секундами, выдача 30 уникальными video ID. Полученные названия показываются как обычный текст, не HTML.

Извлекается `ytInitialData` из desktop HTML. Это не официальный стабильный API: изменение схемы, региональный consent или ограничения YouTube могут сделать поиск недоступным даже при успешном сетевом тесте. Отдельные offline fixtures покрывают дедупликацию, невалидные ID, пустую выдачу, изменённую схему, лимит ответа и обязательность proxy. Сетевая работоспособность поиска на реальном YouTube пока не подтверждена.

UI использует один сетевой worker с ограниченной очередью команд. Поиск не блокирует графический поток; повторный запрос блокируется до завершения предыдущего. Proxy task завершается при закрытии UI/worker и отслеживается на предмет неожиданной остановки.


### Подтверждение текущей сборки

Запуск **42085840430** завершён с `success`: 15 offline-тестов (ядро, прокси, поиск), Clippy, проверка типов Slint UI, Windows release EXE, Android release Rust library через cargo-ndk и упаковка debug-signed preview APK. Исправление sdkmanager подтверждено. End-to-end поиск на YouTube и запуск на устройствах ещё не проверялись.

## Подготовка медиапотоков (следующий этап)

Кнопка **Check stream** в результатах получает watch page через тот же loopback-прокси и извлекает `ytInitialPlayerResponse`. Принимаются только прямые progressive MP4 (AVC + AAC) из `streamingData.formats`. Проверяются video ID, playability status, HTTPS/googlevideo endpoint и срок действия URL. URL с `n`, cipher/signatureCipher, DRM, live и adaptive-only выдачи не поддерживаются: приложение сообщает об ограничении, не пытаясь выдавать их за рабочие.

Для первых трёх подходящих форматов последовательно выполняется реальный GET с `Range: bytes=0-0`. Успех требует HTTP 206, корректного Content-Range, типа video/mp4 и ровно одного полученного байта. Redirect и прямой fallback запрещены. Signed media URLs не показываются в UI и убираются из сетевых ошибок; формат не реализует Debug, чтобы случайно не записать URL в лог. Общий таймаут подготовки в UI — 75 секунд. Проверка байта подтверждает доступность CDN на момент запроса, **не декодируемость или полное воспроизведение**.

Сам resolver — диагностический/подготовительный слой. Windows WebView2 backend и передача медиаданных через Rust gateway добавлены следующим этапом; Android backend пока не реализован. На современном YouTube прямая progressive выдача может отсутствовать; полноценная поддержка JS signature/n transforms и DASH не реализована.

Дополнительно исправлена CDN-проба самотестирования: вместо корня `i.ytimg.com` проверяется конкретный thumbnail path. HTTP 404 по-прежнему считается ошибкой; это не ослабление проверки доступности.


### Подтверждение этапа media diagnostics

[CI 42087540673](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42087540673) завершился успешно: **20 тестов**, Clippy, type-check общего Slint UI и обе платформенные сборки. Ссылки в начале README обновлены на этот запуск. Проверка реального YouTube/CDN из песочницы недоступна, поэтому успешное end-to-end получение видео не заявляется. Запуск APK/EXE и работа player backend на устройствах также пока не проверены.

## Windows player и Rust media gateway

Новый crate `lumen-player` открывает отдельное встроенное окно WebView2 на Windows: HTML5 `<video>` с play/pause/seek/volume. Требуется установленный **Microsoft Edge WebView2 Runtime**; он не скачивается приложением автоматически. Если runtime отсутствует, UI сообщает об ошибке. Внешние mpv, браузер и прокси-бинарники не запускаются.

На Windows кнопка результата называется **Play**. Приложение выбирает поддержанный progressive MP4, проверяет CDN, создаёт `PlaybackSession` и передаёт окну только адрес `http://127.0.0.1:<port>/<random-capability>/media`. Ключ сессии содержит 256 случайных бит; URL подписанного CDN остаётся внутри Rust. Gateway ограничен четырьмя активными медиазапросами, проверяет Host и одиночный byte Range, не следует redirect, копирует только Content-Type/Length/Range и не отдаёт CDN cookies/прочие заголовки. Данные передаются потоково с backpressure и ограничением ожидаемой длины, а не загружаются целиком в RAM. Idle timeout — 20 секунд; для длинного видео нет общего 25-секундного timeout метаданных. Переходы к внешним страницам, новые окна, JS и удалённые ресурсы player-документа запрещены. Управление браузерным runtime и его фоновыми службами не входит в Rust transport.

Закрытие player-окна отменяет сессию и закрывает listener/активные медиазапросы. Одновременно разрешено одно player-окно. Срок подписанного URL не обновляется автоматически: после его истечения новые Range-запросы завершатся ошибкой, потребуется повторно открыть видео. UI подтверждает создание окна, **не успешное декодирование**; ошибки самого `<video>` отображаются его нативными controls. Автозапуск может быть ограничен политикой WebView2 — используйте Play в окне плеера.

Android и другие платформы пока сохраняют **Check stream**; Media3/ExoPlayer и VpnService всё ещё не реализованы. Windows playback не снимает ограничений resolver: cipher/n-transform/DASH/live/DRM не поддерживаются. Реальный запуск WebView2, seek и end-to-end YouTube нужно проверить на Windows-устройстве отдельно от компиляции CI.


### Подтверждение Windows playback build

[CI 42089199084](https://github.com/bonni2709-cmyk/Gogo/actions/runs/42089199084) завершён с `success`: **24 теста**, Clippy сетевых crates/player stub, Linux type-check Slint UI, Windows release EXE с Wry/Tao backend и Android preview APK. Windows backend компилируется в Windows job; Linux Clippy не проверяет Windows-specific код. Устройство для end-to-end проверки воспроизведения, seek и lifecycle пока не использовалось.

Быстрая проверка на Windows: установить WebView2 Runtime при необходимости, распаковать EXE, запустить, дождаться сетевой проверки, найти видео и нажать Play. Если доступна поддерживаемая прямая progressive выдача, откроется встроенное окно video. В случае неподдерживаемой выдачи приложение сообщит об ограничении. Наличие окна не гарантирует, что кодек/поток успешно декодируется.

## Android Media3 host (новый этап)

Добавлена Gradle-обёртка `android/`: `MainActivity extends NativeActivity` запускает Slint, отдельная **неэкспортируемая** `PlayerActivity` — Media3/ExoPlayer 1.5.1 с нативными controls. В новом APK кнопка результата — **Play**. Rust JNI bridge хранит ровно одну `PlaybackSession` и передаёт Java только loopback URL; signed CDN URL остаётся в Rust.

`LoopbackDataSource` принимает ровно один адрес сессии `http://127.0.0.1:<port>/<64 hex>/media`, игнорирует системный proxy, запрещает redirect и любые другие URL, поддерживает последовательное чтение/Range/seek. Это Java HTTP только до локального Rust gateway; все внешние YouTube/CDN-запросы выполняет Rust через встроенный прокси. Android network security config разрешает cleartext только для 127.0.0.1, глобально cleartext запрещён. Строгое требование «даже loopback HTTP только на Rust» потребовало бы отдельного JNI DataSource вместо этой локальной обёртки.

При уходе в фон ExoPlayer освобождается, позиция сохраняется; при возврате Activity player создаётся заново, пока сессия действительна. При закрытии Activity/ошибке playback JNI освобождает Rust-сессию и закрывает gateway. Неизвестный/stale session ID не может закрыть новую сессию. Если Android уничтожит весь процесс, старый URL не восстанавливается — видео нужно открыть снова. Background playback, PiP, audio focus customization, обновление expired URL и Android VpnService пока не реализованы.

Основной Android CI теперь собирает Rust через `cargo-ndk -o android/app/src/main/jniLibs`, затем Gradle 8.11.1 / AGP 8.9.1 / JDK 17 выполняет JVM unit tests, assembleDebug и Android lint. APK по-прежнему debug-signed, arm64, API 26+. Локальные тяжёлые сборки не запускались. Legacy cargo-apk путь не включает Java/Media3 и остаётся только диагностическим вариантом. Реальное воспроизведение, переходы фон/возврат и lifecycle требуют проверки на телефоне.
