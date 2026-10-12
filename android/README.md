# OpenMic — Android-приложение (каркас)

Минимальный каркас по `docs/TZ.md` §4.1: захват микрофона → PCM по TCP
(протокол `openmic/1`, см. `docs/TZ.md` §5 и `crates/openmic-proto`).

Kotlin, minSdk 26, targetSdk 34, обычные Views (без Compose),
нуль внешних зависимостей (только AGP + Kotlin).

## Сборка

Нужны: JDK 17, Android SDK с platform-34 и build-tools 34.0.0,
Gradle (скачивается сам через wrapper).

```sh
. ./env.sh              # из корня репо: JAVA_HOME, ANDROID_SDK_ROOT
cd android
./gradlew assembleDebug # APK: app/build/outputs/apk/debug/app-debug.apk
```

Системный Gradle ставить не нужно: `gradlew` скачает дистрибутив
в `~/.gradle` при первом запуске. Если `gradlew` не запускается —
доустановить ничего нельзя, нужен только интернет и JDK 17.

## Что внутри

- `MainActivity` — поля IP/порт/токен (сохраняются), кнопки Старт/Стоп,
  статус, RMS-индикатор (ProgressBar), запрос RECORD_AUDIO (+ уведомления на 33+).
- `MicService` — foreground-сервис типа `microphone`: AudioRecord
  48 кГц/моно/16 бит (`VOICE_RECOGNITION`), handshake
  `{"hello":"openmic/1","token"}`, фреймы `[u32 LE len][PCM]` по 20 мс,
  ответы на `ping`, локальные gain/mute от сервера, `PARTIAL_WAKE_LOCK`.
- Пока нет (следующие итерации): самоконтроль в наушники, волна,
  выбор источника/частоты, QR-сопряжение, исключение из оптимизации батареи.
