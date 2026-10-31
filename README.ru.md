# BoxPilot

[English](README.md) · [简体中文](README.zh-CN.md) · **Русский**

Аккуратный настольный прокси-клиент для Windows, macOS и Linux. Он управляет ядром
[sing-box](https://sing-box.sagernet.org), сам конвертирует подписки и написан на Rust с
[GPUI](https://gpui.rs) (через gpui-kit).

[![CI](https://github.com/BillBlueInnit/boxpilot/actions/workflows/ci.yml/badge.svg)](https://github.com/BillBlueInnit/boxpilot/actions/workflows/ci.yml)

**[Скачать](https://github.com/BillBlueInnit/boxpilot/releases/latest)** ·
**[Инструкция](docs/usage.ru.md)** ·
[Сборка из исходников](#сборка-из-исходников)

- [Установка](#установка)
- [Быстрый старт](#быстрый-старт)
- [Возможности](#возможности)
- [Поддержка платформ](#поддержка-платформ)
- [Сборка из исходников](#сборка-из-исходников)
- [Структура проекта](#структура-проекта)

## Установка

Файлы лежат на странице [последнего релиза](https://github.com/BillBlueInnit/boxpilot/releases/latest).
В инструкции у каждой системы есть свой раздел: первый запуск, обновления и удаление.

| Система | Установка | Подробнее |
| --- | --- | --- |
| Windows 10 / 11 | Распакуйте `BoxPilot-<версия>-windows-x64.zip` и запустите `boxpilot.exe` | [Windows](docs/usage.ru.md#windows) |
| macOS (Apple silicon / Intel) | Откройте `BoxPilot-<версия>-macos-arm64.dmg` / `-macos-x64.dmg` и перетащите BoxPilot в «Программы» | [macOS](docs/usage.ru.md#macos) |
| Ubuntu, Debian, Linux Mint | `sudo apt install ./boxpilot_<версия>-1_amd64.deb` | [Ubuntu, Debian](docs/usage.ru.md#ubuntu-debian-linux-mint) |
| Fedora, openSUSE | `sudo dnf install ./boxpilot-<версия>-1.x86_64.rpm` (openSUSE: `zypper`) | [Fedora, openSUSE](docs/usage.ru.md#fedora-opensuse) |
| Arch, Manjaro, EndeavourOS | `sudo pacman -Syu boxpilot-bin` из репозитория BoxPilot, см. ниже | [Arch](docs/usage.ru.md#arch-linux-manjaro-endeavouros) |
| NixOS | Архив для Linux через `nix-ld` | [NixOS](docs/usage.ru.md#nixos) |
| Любой другой Linux (x64) | Распакуйте `BoxPilot-<версия>-linux-x64.tar.gz` и запустите `./boxpilot` | [Другой Linux](docs/usage.ru.md#другие-дистрибутивы-linux) |

Пакетам для Linux нужна glibc 2.39 или новее: Ubuntu 24.04, Debian 13, Fedora 40, openSUSE Tumbleweed
и новее.

**Arch Linux.** Каждый релиз одновременно служит репозиторием pacman. Добавьте его один раз и установите;
новые версии придут с обычным `pacman -Syu` (или `yay`):

```bash
printf '\n[boxpilot]\nSigLevel = Optional TrustAll\nServer = https://github.com/BillBlueInnit/boxpilot/releases/latest/download\n' | sudo tee -a /etc/pacman.conf
sudo pacman -Syu boxpilot-bin
```

Пакеты не подписаны, поэтому эта запись доверяет тому, что отдают релизы проекта на GitHub.

## Быстрый старт

1. **Скачайте ядро.** Обзор → **Скачать ядро**. BoxPilot загрузит официальную сборку sing-box для вашей
   системы (если GitHub у вас медленный, укажите **Зеркало GitHub** в Настройки → Ядро sing-box).
2. **Добавьте узлы.** Подписки → **Добавить**, вставьте ссылку от провайдера или нажмите **Вставить**,
   чтобы импортировать из буфера обмена.
3. **Подключитесь.** Обзор → **Подключить**, затем включите **Системный прокси** или **Режим TUN** в быстрых
   настройках.

Всё по шагам — в [инструкции](docs/usage.ru.md).

## Возможности

**Подписки и узлы**

- Форматы: ссылки v2ray / Shadowrocket (обычные или base64), YAML Clash / Clash.Meta, JSON sing-box.
- Протоколы: VLESS (REALITY, Vision), VMess, Trojan, Shadowsocks (2022, obfs, v2ray-plugin), Hysteria 2
  (смена портов), Hysteria, TUIC, AnyTLS, WireGuard, SOCKS, HTTP(S) и SSH поверх ws, grpc, h2, httpupgrade
  и quic. Неподдерживаемые записи (SSR, xhttp…) пишутся в журнал и пропускаются.
- Обновления: свой интервал для каждой подписки, обновление при запуске, догоняющее обновление после
  простоя, быстрый повтор после ошибки, переход с прокси на прямое соединение, свой User-Agent, фильтры
  включения / исключения, трафик и срок действия из `subscription-userinfo`. Пакет обновлений
  перезагружает ядро один раз.
- Локальные узлы: отдельные узлы добавляются по ссылкам; любой узел можно скопировать обратно как ссылку.

**Маршрутизация**

- Режимы «Правила», «Глобальный» и «Напрямую», переключаются на лету.
- Для каждой подписки — группа выбора и группа **Авто**. «Авто» берёт самый быстрый узел, стабильный,
  первый рабочий по списку или с наибольшей скоростью, при желании в пределах одного региона.
  Переключение не обрывает открытые соединения.
- Наборы правил по ссылке или из файла: sing-box `.srs` / `.json`, провайдеры правил Clash и ACL4SSR
  `.list`. Каждый направляет трафик в прокси, напрямую, в блокировку или в любую группу — в вашем порядке
  и со своим интервалом обновления.
- **Импорт ACL4SSR** читает `.ini` subconverter (например, ACL4SSR_Online_Full) и создаёт все его наборы,
  выключенными, пока вы не выберете нужные.
- Набор правил может получить свою группу узлов — с выбором вручную или автоматически, а его файл
  открывается в текстовом редакторе; сохранённые изменения применяются сами.
- Встроенный обход материкового Китая и блокировка рекламы, а также свои правила (`ТИП,ЗНАЧЕНИЕ,ЦЕЛЬ`).
- DNS по HTTPS, TLS, QUIC или UDP, FakeIP и IPv6. Наборы правил кэшируются локально, поэтому
  заблокированный GitHub или нерабочий узел не мешают запуску ядра.

**Подключение**

- Системный прокси (восстанавливается при выходе и после сбоя), режим TUN, доступ из локальной сети,
  UDP через прокси или напрямую и необязательная блокировка QUIC.
- Проверка задержки, реальной задержки (новый HTTPS-запрос через узел) и скорости загрузки — через
  отдельный зонд, ваш выбор узла не меняется.
- Код страны у каждого узла: из названия, иначе по GeoIP, иначе по выходному IP.
- Изменения настроек объединяются в одну перезагрузку ядра; упавшее ядро перезапускается само.

**Приложение**

- English, 简体中文, 繁體中文 и Русский — следует языку системы, переключается на лету.
- Меню в трее с переключателями режима, системного прокси и TUN; сворачивание в трей; **Облегчённый
  режим** освобождает интерфейс, пока прокси продолжает работать.
- Автозапуск (можно свёрнутым), график трафика, список соединений, журнал.
- Светлая и тёмная темы, цвет темы и полупрозрачное размытое окно там, где система это умеет (см.
  [Поддержка платформ](#поддержка-платформ)).
- Обновления с GitHub Releases: устанавливать автоматически, только напоминать или не уведомлять.
- Каждый переход — затухающая пружина с учётом параметра «Уменьшить движение».

## Поддержка платформ

Одинаковые возможности везде, где работает GPUI. Там, где системы различаются, используется родной
механизм каждой:

| | Windows | macOS | Linux |
| --- | --- | --- | --- |
| Пакет | zip | `.dmg` с BoxPilot.app | `.deb`, `.rpm`, репозиторий Arch, tar.gz |
| Рамка окна | родная, макеты прикрепления | родные кнопки-«светофор» | рисует BoxPilot (кнопки Adwaita или Breeze), в тайлинговых композиторах — композитор |
| Полупрозрачное окно | Mica, Mica Alt, Acrylic (Windows 11 22H2+) | системное размытие | размытие, если композитор его предлагает (niri, KDE Plasma) |
| Системный прокси | WinINet | `networksetup` для каждой сетевой службы | GNOME `gsettings` и KDE `kioslaverc` |
| Права для TUN | запуск от имени администратора | ядро получает setuid root после одного запроса пароля | ядро получает `CAP_NET_ADMIN` после одного запроса пароля |
| Автозапуск | ключ Run / задача планировщика | LaunchAgent | XDG autostart |
| Очистка ядра после сбоя | объект задания | при следующем запуске | сигнал о смерти родителя и при следующем запуске |
| Папка данных | `%APPDATA%\BoxPilot` | `~/Library/Application Support/BoxPilot` | `~/.config/BoxPilot` |

Подробности для рабочих столов Linux (значок в трее, TUN без агента polkit, размытие, тайлинговые
композиторы, ноутбуки с двумя GPU) — в [инструкции](docs/usage.ru.md#8-рабочие-столы-linux).

## Сборка из исходников

Нужен stable Rust ([rustup](https://rustup.rs)). В Linux также нужны пакеты разработки, с которыми
собирается GPUI. В Debian или Ubuntu CI ставит:

```bash
sudo apt install build-essential pkg-config clang libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libvulkan-dev \
  libpipewire-0.3-dev libgtk-3-dev libayatana-appindicator3-dev libxdo-dev
```

Для Arch они перечислены в [сценарии `boxpilot`](packaging/aur/boxpilot/PKGBUILD).

Запуск:

```bash
cargo run --release -p boxpilot
```

**Тесты.** `cargo test --workspace` запускает модульные тесты. Проверить сгенерированные конфиги
настоящим ядром:

```bash
cargo run -p boxpilot-core --example gen_config -- out
sing-box check -c out/everything-1.14.json
```

Конфиги рассчитаны на sing-box 1.12 и новее (рекомендуется 1.14).

**Пакеты для Linux.** После `cargo install cargo-deb cargo-generate-rpm` команда
`packaging/linux/build.sh [tar] [deb] [rpm] [arch]` кладёт в `dist/` архив, `.deb`, `.rpm`, а на Arch —
пакет и репозиторий pacman. Пакетам, собранным на дистрибутиве с плавающими обновлениями, нужна его glibc;
выпускаемые пакеты собираются на Ubuntu 24.04. Сценарии для AUR — в [`packaging/aur`](packaging/aur).

**Вторая копия для разработки.** `BOXPILOT_INSTANCE=dev` даёт копии собственную блокировку единственного
экземпляра, а `XDG_CONFIG_HOME` (Linux) — свою папку данных, и она работает рядом с обычным BoxPilot.

**CI и выпуски.**

- [`ci.yml`](.github/workflows/ci.yml) собирает и тестирует на Windows, macOS (arm64, x64) и Linux:
  модульные тесты, проверка каждого сгенерированного конфига через `sing-box check`, живой тест движка с
  настоящим ядром, проверка запуска интерфейса и установка пакетов Linux на Ubuntu и Arch.
- [`live.yml`](.github/workflows/live.yml) проверяет системный прокси и TUN на настоящих машинах.
- [`release.yml`](.github/workflows/release.yml) публикует выпуск по тегу версии. Задайте версию в
  `Cargo.toml`, затем:

  ```bash
  git tag v1.2.3
  git push origin v1.2.3
  ```

  К выпуску прикладываются zip для Windows, `.dmg` и архивы для macOS, архив, `.deb`, `.rpm` и репозиторий
  Arch для Linux. С секретом `AUR_SSH_PRIVATE_KEY` обновляется и AUR.

## Структура проекта

| Путь | Что там |
| --- | --- |
| `crates/core` | Всё без интерфейса: конвертер подписок (`convert/`), генератор конфигурации (`config.rs`), преобразование наборов правил (`rules.rs`), движок с жизненным циклом процесса, планировщиком и обновлениями (`engine.rs`), клиент Clash API, загрузчик ядра, интеграция с системой (`sys.rs`). |
| `crates/app` | Приложение на GPUI: страницы (`src/ui/`), пружины (`ui/anim.rs`), переводы (`i18n.rs`), трей, единственный экземпляр, окна. |
| `packaging/` | Пункт меню и сценарии пакетов для Linux, сценарии AUR, `Info.plist` для macOS. |
| `docs/` | Инструкция на трёх языках. |

## Лицензия

MIT — см. [LICENSE](LICENSE).
