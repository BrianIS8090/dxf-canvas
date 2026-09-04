DXF Холст — сторонние компоненты

Приложение использует открытые библиотеки Rust и встроенный конвертер DWG на ACadSharp 3.7.1 (MIT). Конвертер собран в автономный Windows x64 EXE с NativeAOT .NET 10.0.10: установка .NET и CAD-системы не нужна. Точные версии закреплены в Cargo.lock и tools/dwg-converter/packages.lock.json; лицензия самого приложения — MIT.

Основные компоненты:
- egui / eframe — MIT OR Apache-2.0, интерфейс и отрисовка.
- dxf — MIT, чтение DXF.
- rfd — MIT, выбор файлов.
- thiserror — MIT OR Apache-2.0, обработка ошибок.
- encoding_rs — (Apache-2.0 OR MIT) AND BSD-3-Clause, декодирование текста.
- earcutr — ISC, построение заливок с отверстиями.
- tempfile — MIT OR Apache-2.0, временные файлы конвертации.
- serde_json — MIT OR Apache-2.0, чтение отчёта конвертера.
- windows-sys — MIT OR Apache-2.0, управление дочерним процессом Windows.
- ACadSharp / включённая CSUtilities — MIT, чтение DWG и запись DXF.
- .NET NativeAOT — MIT и уведомления о сторонних компонентах из пакета компилятора.

Полные уведомления для DWG-компонента находятся в docs/licenses/ACADSHARP-LICENSE.txt, DOTNET-LICENSE.txt и DOTNET-NATIVE-NOTICES.txt. Они также встроены в меню версии программы. LibreDWG и коммерческие CAD-конвертеры не используются.

Ниже приведено уведомление новой зависимости earcutr 0.5.0. Полные сведения об остальных прямых и транзитивных зависимостях доступны в их пакетах по версиям из Cargo.lock. Этот файл не заменяет их лицензии.

ISC License

Copyright (c) 2016, Mapbox
Copyright (c) 2018, Tree Cricket

Permission to use, copy, modify, and/or distribute this software for any purpose
with or without fee is hereby granted, provided that the above copyright notice
and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH
REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND
FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT,
INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS
OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER
TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF
THIS SOFTWARE.
