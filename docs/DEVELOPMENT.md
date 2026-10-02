# Snj Voice Changer - development handoff

Цей файл створений як handoff-документ для нового чату з чистим контекстом. Якщо продовжувати розробку після закриття старого чату, починати треба саме звідси.

## Початкова ідея

Snj Voice Changer - це Windows WinForms застосунок для real-time voice changer.

Початкова ціль:

1. Захопити звук з обраного системного input device, як у Google Meet/Chrome selector.
2. Пропустити голос через ланцюг VST-плагінів.
3. Віддати оброблений звук у Windows як virtual microphone, щоб у Google Meet, Discord, Chrome тощо можна було вибрати `Snj Voice Changer` як мікрофон.
4. У майбутньому зробити VST chain UI: додати VST, увімкнути/вимкнути, міняти місцями, відкривати editor/parameters, приблизно як FX chain у REAPER.

Поточний робочий результат - не готовий voice changer, але вже є корисний WinForms-прототип:

- застосунок стартує;
- бачить реальні системні input devices;
- дозволяє обрати input device;
- показує live input signal meter для вибраного мікрофона;
- має placeholder для virtual microphone status;
- має базову структуру, з якої можна будувати capture -> process -> output pipeline.

## Що зараз вважаємо корисною гілкою

Корисна частина - тільки C# WinForms проєкт:

- `SnjVoiceChanger.sln`
- `SnjVoiceChanger/SnjVoiceChanger.csproj`
- `SnjVoiceChanger/*.cs`
- `README.md`
- `.gitignore`
- цей `DEVELOPMENT.md`

Експерименти з власним драйвером більше не вважаються частиною MVP-напрямку. Вони були видалені з робочої папки, щоб новий чат і майбутня розробка не тягнули за собою зайвий WDK/SYSVAD контекст.

Видалені driver/VM артефакти:

- `drivers/`
- `wil/`
- `SnjDriverPackage.zip`
- `SnjDriverPackageBaseline.zip`
- `SnjDriverPackageSafe.zip`
- `SnjDriverPackageSmoke.zip`
- `SnjDriverPackageBaseline/`
- `SnjDriverPackageSafe/`
- `SnjDriverPackageSmoke/`
- `SysvadMicrosoftControl.zip`
- `SysvadMicrosoftControl/`
- `vm-check-audio-endpoints.cmd`
- `vm-diagnose-driver.cmd`
- `vm-install-baseline-driver.cmd`
- `vm-install-full-componentized-driver.cmd`
- `vm-install-ms-sysvad-control.cmd`
- `vm-install-smoke-driver.cmd`

Також видалена зайва `.github/` папка з Azure/Copilot інструкціями, бо вона не належала до C# WinForms проєкту.

Папку `.vs/` не видаляли: це локальний кеш/налаштування Visual Studio, він не має потрапляти в git і вже ігнорується через `.gitignore`.

## Поточна технологічна база

Проєкт:

- Windows Forms
- .NET 9
- C#
- NAudio `2.2.1`

Файл проєкту:

- `SnjVoiceChanger/SnjVoiceChanger.csproj`

Важливі налаштування:

- `TargetFramework`: `net9.0-windows`
- `UseWindowsForms`: `true`
- `ApplicationTitle`: `Snj Voice Changer v1.2`
- `Nullable`: `enable`
- NuGet dependency: `NAudio`

Запуск:

- відкрити `SnjVoiceChanger.sln` у Visual Studio;
- запустити проєкт `SnjVoiceChanger`;
- NuGet restore можна робити з Visual Studio або командою `dotnet restore`.

Перевірка збірки з консолі:

```cmd
dotnet build SnjVoiceChanger.sln
```

## Поточний UI

Головна форма називається `MainForm`, title window:

```text
Snj Voice Changer v1.2
```

Файли:

- `SnjVoiceChanger/Form1.cs`
- `SnjVoiceChanger/Form1.Designer.cs`
- `SnjVoiceChanger/Form1.resx`

Поточна форма має:

- ліву панель;
- label `InputDevice`;
- combo box зі списком input devices;
- кнопку `Refresh`;
- секцію `Virtual microphone`;
- секцію `Input signal`;
- live audio meter під секцією virtual microphone;
- праву основну область з placeholder text `VST chain will appear here later`.

Поточний UI навмисно простий. Правий panel поки не реалізований, бо VST chain ще не будували.

## Input device scanning

Файли:

- `SnjVoiceChanger/AudioInputDeviceScanner.cs`
- `SnjVoiceChanger/AudioInputDevice.cs`

Що зроблено:

- input devices скануються через NAudio/CoreAudio:

```csharp
using var enumerator = new MMDeviceEnumerator();

enumerator.EnumerateAudioEndPoints(DataFlow.Capture, DeviceState.Active)
```

- беруться тільки active capture endpoints;
- кожен device представлений record-типом:

```csharp
public sealed record AudioInputDevice(string Id, string Name)
{
    public override string ToString() => Name;
}
```

- у combo box показується повна friendly name;
- назви не обрізаються вручну;
- drop-down width розраховується за найдовшою назвою, щоб довгі назви пристроїв були видимі.

Важливе рішення:

- ручний COM interop для CoreAudio був відкинутий;
- NAudio стабільніше і простіше для цього етапу.

## Input signal meter

Файли:

- `SnjVoiceChanger/AudioInputLevelMonitor.cs`
- `SnjVoiceChanger/AudioLevelMeterControl.cs`

Що зроблено:

- при виборі input device створюється `AudioInputLevelMonitor`;
- він відкриває конкретний MMDevice через device id;
- захоплення йде через `WasapiCapture`;
- на `DataAvailable` рахується peak level з raw audio buffer;
- WinForms `Timer` з інтервалом 33 ms оновлює meter приблизно 30 разів на секунду;
- при зміні input device попередній monitor зупиняється і dispose-иться;
- при закритті форми monitor також dispose-иться.

Підтримані sample formats у meter calculation:

- PCM 16-bit;
- PCM 24-bit;
- PCM 32-bit;
- float 32-bit.

Поточна логіка meter:

- `AudioInputLevelMonitor.GetPeakLevel()` повертає float `0..1`;
- `AudioLevelMeterControl.Level` приймає float `0..1`;
- meter переводить level у display position через dB mapping;
- нижня межа для meter position зараз `-60 dB`;
- у нижній частині meter показує `RMS` label і dB text, хоча фактично зараз використовується peak value, не справжній RMS. Це треба або перейменувати у UI, або реалізувати реальний RMS пізніше.

Візуал:

- темний фон, як у DAW meter;
- зелена основна зона;
- жовта зона при високому рівні;
- червона зона при near-clipping;
- біла peak-hold планка.

Peak hold:

- планка тримається 1 секунду;
- після цього повільно падає вниз;
- швидкість падіння зараз `0.38f` meter-position units per second.

Важливий build fix:

`AudioLevelMeterControl.Level` має атрибути:

```csharp
[Browsable(false)]
[DesignerSerializationVisibility(DesignerSerializationVisibility.Hidden)]
```

Це прибрало WinForms designer/build помилку `WFO1000: Property 'Level' does not configure the code serialization for its property content`.

## Virtual microphone status

Файли:

- `SnjVoiceChanger/VirtualMicrophoneService.cs`
- `SnjVoiceChanger/VirtualMicrophoneStatus.cs`

Поточна реалізація тільки детектить, чи існує input device з назвою:

- `Snj Voice Changer`
- або `SnjVoiceChanger`

Якщо такий device знайдено:

- status: `Detected`
- колір зелений.

Якщо не знайдено:

- name: `Snj Voice Changer`
- status: `Driver required`
- колір червоний.

Важливо:

- WinForms application сам по собі не може створити Windows recording endpoint;
- цей service зараз не створює virtual microphone;
- він тільки показує, чи такий endpoint вже є в системі.

## Що вже було виправлено під час розробки

1. Не збиралось через CoreAudio COM interop.

   Причина: ручні COM interface definitions легко помилитися, був `InvalidCastException`/cast mismatch.

   Рішення: перейти на NAudio `MMDeviceEnumerator`.

2. Назви input devices здавались обрізаними.

   Зараз код не обрізає назви вручну. ComboBox показує friendly name, а drop-down width розраховується динамічно.

3. Input meter спочатку не показував звук.

   Рішення: не покладатися на device audio meter interface, а реально відкрити capture через `WasapiCapture` і рахувати level з audio buffer.

4. Peak line залипала нагорі.

   Рішення: додано hold на 1 секунду і потім decay вниз.

5. Build ламався через WinForms property serialization.

   Рішення: `Level` у custom control позначено як non-browsable і hidden для designer serialization.

## Драйверний експеримент, коротко

Ми пробували шлях власного virtual audio driver на базі Microsoft SYSVAD/WDK.

Що вдалось:

- встановили WDK/SDK;
- зібрали SYSVAD sample;
- підняли Hyper-V VM;
- увімкнули test signing у VM;
- встановили тестовий driver package;
- добились появи audio endpoints у VM.

Чому відмовились:

- власний kernel driver потребує test mode під час розробки і Microsoft-trusted signing для production;
- без Microsoft Hardware Dev Center / attestation / WHQL такий драйвер не буде нормально встановлюватися у користувачів;
- були BSOD у VM на `tabletaudiosample.sys`, навіть на близькому до Microsoft sample сценарії;
- це занадто ризиково і довго для MVP;
- вимога продукту: має працювати "з коробки", без того щоб користувачі вмикали test mode в admin console.

Висновок:

Власний драйвер зупинено як MVP-напрямок. Якщо колись повертатися, це окремий довгий R&D/driver-signing трек, не частина першої робочої версії.

## Поточний результат research щодо virtual microphone

Ключовий факт:

Звичайний user-mode WinForms застосунок не може сам створити системний recording endpoint, який Chrome/Google Meet побачить як мікрофон. Для цього потрібен або signed virtual audio driver, або вже встановлений virtual audio cable/virtual microphone product.

Для MVP найкращий шлях: використати готовий virtual audio cable.

### Рекомендований варіант: VB-Audio VB-CABLE

VB-CABLE створює пару Windows audio endpoints:

- playback endpoint: `CABLE Input`
- recording endpoint: `CABLE Output`

Логіка для нашого застосунку:

```text
Real microphone
    -> Snj Voice Changer input capture
    -> future VST chain
    -> render to "CABLE Input"
    -> VB-CABLE driver
    -> "CABLE Output"
    -> Google Meet / Chrome / Discord selected microphone
```

Плюси:

- готовий signed driver;
- не потрібен test mode;
- не треба писати kernel code;
- швидко перевіряється на реальній машині;
- ідеально підходить для audio routing MVP.

Мінуси:

- virtual mic у Windows буде називатися `CABLE Output`, не `Snj Voice Changer`;
- це стороння залежність;
- треба перевірити license/redistribution, якщо ми захочемо бандлити installer разом із Snj Voice Changer;
- для повністю branded "Snj Voice Changer" endpoint все одно потрібен або власний signed driver, або домовленість/white-label/custom driver з постачальником.

### Альтернатива: Virtual Audio Cable by Eugene Muzychenko

Плюси:

- зрілий комерційний продукт;
- signed driver;
- є багато можливостей routing/cables;
- потенційно можна домовлятися про custom/proprietary version.

Мінуси:

- платний;
- складніше для простого MVP;
- також стороння залежність.

### Альтернатива: Voicemeeter

Плюси:

- потужний routing/mixer;
- створює virtual inputs/outputs;
- популярний серед стрімерів.

Мінуси:

- занадто великий комбайн для нашого UX;
- користувачу доведеться мати ще один мікшер;
- гірше для простого "запустив Snj Voice Changer і вибрав mic у Meet".

### Open-source Windows virtual audio drivers

Є різні open-source драйвери, часто SYSVAD-based.

Мінус для нашої мети той самий:

- signing;
- kernel risk;
- підтримка;
- сумісність з Windows updates;
- production install без test mode.

Тому open-source driver не вирішує головну проблему MVP.

### ASIO не вирішує задачу

ASIO може бути корисний для low-latency audio у DAW, але Chrome/Google Meet очікують Windows recording endpoint через WASAPI/MMDevice stack. ASIO-only virtual device не є достатнім шляхом для "вибрати мікрофон у Chrome".

## Поточне архітектурне рішення для MVP

Не створювати власний virtual microphone.

Замість цього:

1. Захоплювати real microphone у Snj Voice Changer.
2. Обробляти звук у user-mode audio pipeline.
3. Виводити processed audio у готовий signed virtual cable playback endpoint, наприклад `CABLE Input`.
4. У Chrome/Google Meet користувач обирає paired recording endpoint, наприклад `CABLE Output`.

Це не ідеальний branded шлях, але це найкоротший шлях до реально працюючого voice changer.

## Наступний великий етап: audio output routing

Потрібно додати `OutputDevice` selector.

Очікуваний UI:

- ліворуч після/під input секцією додати `OutputDevice`;
- показувати active render endpoints;
- refresh має оновлювати і input, і output devices;
- бажано окремо показувати detection status для VB-CABLE:
  - якщо `CABLE Input` і `CABLE Output` знайдено - `Virtual cable ready`;
  - якщо не знайдено - `Install VB-CABLE`;
  - якщо знайдено тільки один endpoint - `Virtual cable incomplete`.

Для output scanning потрібно створити:

- `AudioOutputDevice.cs`
- `AudioOutputDeviceScanner.cs`

Scanner має використовувати:

```csharp
MMDeviceEnumerator.EnumerateAudioEndPoints(DataFlow.Render, DeviceState.Active)
```

Для VB-CABLE detection треба окремо сканувати:

- render endpoints для `CABLE Input`;
- capture endpoints для `CABLE Output`.

Не варто шукати тільки exact string, бо назви можуть бути:

- `CABLE Input (VB-Audio Virtual Cable)`
- `CABLE Output (VB-Audio Virtual Cable)`
- інші локалізовані/варіантні назви.

Краще зробити detection через contains:

- render: contains `CABLE Input` або `VB-Audio Virtual Cable`;
- capture: contains `CABLE Output` або `VB-Audio Virtual Cable`.

## Наступний великий етап: passthrough без VST

Перед VST треба зробити простий real-time passthrough:

```text
selected input device -> Snj app -> selected output device
```

Ціль:

- вибрати мікрофон у `InputDevice`;
- вибрати `CABLE Input` у `OutputDevice`;
- натиснути Start;
- у Windows/Google Meet вибрати `CABLE Output`;
- почути/побачити, що звук з мікрофона доходить до virtual microphone.

Це критичний milestone. Без нього VST не має сенсу.

Можлива NAudio архітектура:

- capture: `WasapiCapture` з selected input `MMDevice`;
- buffer: `BufferedWaveProvider` або власний lock-free/ring buffer;
- output: `WasapiOut` на selected render `MMDevice`;
- формат: на старті можна спробувати capture wave format напряму;
- якщо output device не приймає capture format - додати resampling/conversion.

Питання, які треба вирішити:

- як стабільно узгодити sample rate;
- як узгодити channel count;
- як уникнути buffer underrun/overrun;
- який latency target ставити;
- чи робити exclusive mode або shared mode;
- як поводитись при device disconnect;
- що робити, якщо користувач вибрав speakers замість virtual cable і отримав feedback loop.

Для MVP краще:

- shared mode;
- latency target 50-100 ms;
- явний Start/Stop button;
- status label: `Stopped`, `Starting`, `Running`, `Error`;
- output warning, якщо selected output не схожий на virtual cable.

Потрібні нові класи:

- `AudioOutputDevice`
- `AudioOutputDeviceScanner`
- `AudioRoutingService`
- можливо `AudioRouteStatus`

`AudioRoutingService` має:

- приймати selected input id;
- приймати selected output id;
- стартувати capture;
- стартувати render;
- передавати audio frames;
- мати `Start()`, `Stop()`, `Dispose()`;
- прокидати помилки у UI без падіння застосунку.

## Наступний великий етап: VST chain

VST не треба додавати, поки не працює passthrough.

Коли passthrough запрацює:

```text
input capture -> processing graph -> output render
```

Тоді між capture і output треба вставити VST processing graph:

```text
input capture
    -> format conversion / interleaved-to-float
    -> VST chain
    -> float-to-output-format
    -> output render
```

Потрібно визначитись з VST strategy:

1. VST2
   - простіше знайти старі .NET wrappers;
   - licensing/history складні;
   - багато старих плагінів.

2. VST3
   - сучасний стандарт;
   - офіційний SDK C++;
   - у C# може бути складніше;
   - можливо треба native host layer + C# interop.

3. Готовий .NET VST host library
   - треба окремо дослідити актуальність;
   - важливо, чи підтримує x64, VST3, сучасний Windows;
   - важливо, чи не abandoned.

Для MVP VST-chain можна почати з одного простого plugin slot, не з повного chain UI.

Мінімальний VST milestone:

- користувач вибирає один VST plugin file/folder;
- app завантажує plugin;
- звук проходить через plugin;
- якщо plugin не завантажився - app не падає;
- є bypass.

Потім:

- список plugins;
- enable/disable checkbox;
- reorder;
- remove;
- plugin editor window;
- presets;
- persistence у config file.

## UI roadmap

Поточний UI має бути розширений поступово.

### Етап 1: OutputDevice і Start/Stop

Ліва панель:

- `InputDevice` combo;
- `OutputDevice` combo;
- `Refresh`;
- `Start` / `Stop`;
- `Input signal` meter;
- `Output signal` meter, опціонально;
- `Virtual cable status`.

Права панель:

- поки placeholder.

### Етап 2: Basic routing diagnostics

Додати:

- current input format;
- current output format;
- routing status;
- latency/buffer indicator;
- error messages.

Це можна робити не дуже красиво, але дуже корисно для debug.

### Етап 3: VST chain list

Права панель:

- вертикальний список plugins;
- checkbox enabled;
- plugin name;
- buttons: add, remove, move up, move down;
- selected plugin details/editor area.

### Етап 4: Presets/config

Зберігати:

- last selected input device id;
- last selected output device id;
- VST plugin folders;
- VST chain order;
- enabled/disabled state;
- plugin-specific state/preset, якщо можливо.

Можливе місце config:

```text
%AppData%\SnjVoiceChanger\config.json
```

## Product/install roadmap

Для MVP без власного драйвера:

1. Snj Voice Changer installer встановлює тільки наш app.
2. App при старті перевіряє наявність VB-CABLE.
3. Якщо VB-CABLE нема:
   - показати status `VB-CABLE not installed`;
   - дати кнопку або link `Install VB-CABLE`;
   - не намагатися тихо ставити драйвер без зрозумілої license/permission.
4. Якщо VB-CABLE є:
   - автоматично запропонувати `CABLE Input` як OutputDevice;
   - підказати, що у Google Meet треба обрати `CABLE Output`.

Для "works out of box" у сильному сенсі:

- треба мати право бандлити signed virtual cable driver;
- або мати домовленість з VB-Audio/VAC;
- або повернутись до власного Microsoft-signed driver, але це довгий production-signing шлях.

На поточному етапі не планувати власний драйвер.

## Recommended next steps for a fresh chat

Почати новий чат і сказати приблизно так:

```text
Прочитай DEVELOPMENT.md. Продовжуємо Snj Voice Changer тільки як C# WinForms app. Драйверну гілку не чіпаємо. Наступна задача: додати OutputDevice selector і VB-CABLE detection.
```

Після цього робити кроки:

1. Переконатися, що C# проєкт збирається:

   ```cmd
   dotnet build SnjVoiceChanger.sln
   ```

2. Додати output device model/scanner:

   - `AudioOutputDevice.cs`
   - `AudioOutputDeviceScanner.cs`

3. Додати UI controls:

   - label `OutputDevice`;
   - output combo box;
   - можливо `Start` / `Stop`.

4. Оновити virtual microphone detection:

   - перестати шукати тільки `Snj Voice Changer`;
   - додати detection для VB-CABLE;
   - показувати окремий status для virtual cable.

5. Реалізувати passthrough:

   - selected input -> selected output;
   - спочатку без VST;
   - основна перевірка: output у `CABLE Input`, browser/Meet input у `CABLE Output`.

6. Лише після стабільного passthrough переходити до VST.

## Current limitations

- Нема audio output routing.
- Нема VST host.
- Нема VST chain UI.
- Нема persistence/config.
- Нема installer.
- Нема автоматичного virtual microphone creation.
- Virtual microphone status зараз фактично legacy placeholder з driver experiment.
- Meter caption показує `RMS`, але значення зараз peak-based.

## Important principle

Не повертатись до власного Windows kernel audio driver як до основного MVP-шляху.

Для першої реально корисної версії треба довести до роботи цей user-mode pipeline:

```text
Real mic -> Snj WinForms app -> VB-CABLE Input -> VB-CABLE Output -> Google Meet
```

Коли цей шлях стабільно працює, тоді Snj Voice Changer вже стає корисним застосунком, навіть без branded virtual microphone.
