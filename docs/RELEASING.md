# Порядок выпуска

## Обычная проверка изменений

Предложение изменений → автоматические проверки на Windows → проверка интерфейса вручную → включение в `main` → версия и тег → черновик релиза → публикация.

GitHub Actions запускает форматирование, сборку встроенного DWG-конвертера, публичные тесты, Clippy и сборку EXE при изменениях `main`/`dev`, в предложениях изменений и по тегам `v*`. Частные эталоны в CI не используются. Версии действий закреплены идентификаторами коммитов; обычная сборка имеет только право чтения. Право записи выделено отдельному шагу подготовки релиза.

## Новый релиз

1. Обновите версию в `Cargo.toml` и выполните `cargo check`, чтобы обновить `Cargo.lock`.
2. Добавьте запись в `CHANGELOG.md` и описание `docs/releases/vX.Y.Z.md`. При изменении интерфейса обновите скриншоты.
3. Проверьте приложение вручную и убедитесь, что автоматические проверки проходят.
4. Зафиксируйте изменения, отправьте их, затем создайте аннотированный тег той же версии и отправьте его:

   ```powershell
   git tag -a vX.Y.Z -m "DXF Холст X.Y.Z"
   git push origin vX.Y.Z
   ```

5. Дождитесь успешной сборки. Она создаст **черновик** GitHub Release с EXE, `SHA256SUMS.txt` и `THIRD-PARTY-LICENSES.txt`.
6. Проверьте описание и файл, затем **опубликуйте** черновик в разделе Releases или командой `gh release edit vX.Y.Z --draft=false --latest`. Черновик не виден обычным посетителям и не считается завершённым выпуском. Старые релизы и теги не перезаписывайте.

Для первого публичного выпуска 0.5.3 допускается загрузка локально проверенного EXE; последующие теги используют тот же формат файлов. Тег обязан совпадать с версией в `Cargo.toml`.

## Упаковка локальной сборки

```powershell
./scripts/build-dwg-converter.ps1
cargo build --locked --release
./scripts/package-release.ps1
```

В `dist/release` появятся `DXF-Canvas-X.Y.Z-windows-x64.exe`, `SHA256SUMS.txt` и `THIRD-PARTY-LICENSES.txt`. Лицензии также встроены в сам EXE. Каталог `dist` не хранится в Git: исполняемые файлы размещаются в Releases, а не в истории исходников.

## Проверка скачанного EXE

Откройте PowerShell в папке скачанного файла:

```powershell
Get-FileHash -LiteralPath './DXF-Canvas-0.8.0-windows-x64.exe' -Algorithm SHA256
```

Сравните значение `Hash` с записью из `SHA256SUMS.txt` того же релиза. Контрольная сумма помогает проверить целостность файла, но не заменяет цифровую подпись.

## Ссылки на документацию GitHub

- [Безопасное оформление действий сборки](https://docs.github.com/en/actions/reference/security/secure-use)
- [Создание релиза через GitHub CLI](https://cli.github.com/manual/gh_release_create)
- [Тематические теги репозитория](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/classifying-your-repository-with-topics)
