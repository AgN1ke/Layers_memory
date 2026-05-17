# Local Development Environment

Цей документ потрібен для моделей або розробників, які відкривають проєкт без історії чату. Тут зафіксовано, що вже встановлено на цій машині, де воно лежить і якими командами перевіряти локальний цикл.

Документ описує поточну Windows-машину користувача для `C:\Python_projects\Layers_memory`.

## Що Встановлено

### Rust Toolchain

Встановлено через `winget`:

```powershell
winget install --id Rustlang.Rustup --source winget
```

Поточний стан:

- `rustup 1.29.0`
- `rustc 1.95.0`
- `cargo 1.95.0`
- active toolchain: `stable-x86_64-pc-windows-msvc`
- installed target: `x86_64-pc-windows-msvc`

Основні шляхи:

- Cargo binaries: `C:\Users\AgNike\.cargo\bin`
- Rustup home: `C:\Users\AgNike\.rustup`
- Cargo cache/registry: `C:\Users\AgNike\.cargo`

У `C:\Users\AgNike\.cargo\bin` лежать:

- `cargo.exe`
- `rustc.exe`
- `rustup.exe`
- `rustfmt.exe`
- `cargo-fmt.exe`
- `cargo-clippy.exe`
- `clippy-driver.exe`
- `rust-analyzer.exe`

### Visual Studio Build Tools

Встановлено через `winget`:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --source winget --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --norestart"
```

Поточний стан:

- package: `Microsoft.VisualStudio.2022.BuildTools`
- version: `17.14.32`
- installed path: `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools`
- discovery tool: `C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe`

Навіщо це потрібно:

- Rust target `x86_64-pc-windows-msvc` потребує MSVC linker `link.exe`.
- Без Visual Studio Build Tools `cargo test` падає з помилкою `linker link.exe not found`.
- У звичайній PowerShell команді `where.exe link` може нічого не показати, але `cargo` після встановлення Build Tools уже вміє знайти MSVC linker.

## Як Перевірити Середовище

Спочатку перейти в корінь проєкту:

```powershell
cd C:\Python_projects\Layers_memory
```

Перевірити Rust:

```powershell
rustup --version
rustup show
rustc --version
cargo --version
```

Перевірити встановлені Windows-пакети:

```powershell
winget list --id Rustlang.Rustup --source winget
winget list --id Microsoft.VisualStudio.2022.BuildTools --source winget
```

Якщо поточна PowerShell-сесія не бачить `cargo`, тимчасово додати Cargo binaries у PATH:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
```

Це потрібно тільки для старої вже відкритої сесії. User PATH уже містить `C:\Users\AgNike\.cargo\bin`, тому в новому терміналі `cargo` має бути доступний без ручного додавання.

## Як Запускати Проєкт Локально

Підтягнути залежності:

```powershell
cargo fetch
```

Перевірити форматування:

```powershell
cargo fmt --check
```

Запустити тести:

```powershell
cargo test
```

Запустити clippy як строгий quality gate:

```powershell
cargo clippy --all-targets -- -D warnings
```

Повний локальний цикл для моделі:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo fetch
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

## Як Запустити Живий Термінал Памʼяті

Інтерактивний runner:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo run --bin memory_terminal -- memory
```

Якщо термінал некоректно показує кирилицю, перед запуском увімкнути UTF-8:

```powershell
chcp 65001
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new()
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo run --bin memory_terminal -- memory
```

Команди всередині runner:

```text
/help              показати команди
/where             показати активну теку памʼяті і сесію
/session <id>      перемкнути активну сесію
/sleep             стиснути поточну сесію в preliminary archive memory
/recall <text>     пошукати archive memory
/tasks             показати pending LLM tasks
/exit              вийти
```

Plain text без `/` записується як `IngestEvent` у поточну сесію.

## Поточний Очікуваний Результат

На момент запису цього документа локальний цикл проходить:

- `cargo fmt --check` проходить;
- `cargo test` проходить;
- `cargo test` запускає 6 тестів;
- `cargo clippy --all-targets -- -D warnings` проходить.

## Що Не Треба Шукати Вручну

Не треба вручну шукати Rust по системі. Основна точка входу:

```text
C:\Users\AgNike\.cargo\bin
```

Не треба вручну шукати Visual Studio Build Tools. Основна точка входу:

```text
C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools
```

Якщо треба знайти встановлення Visual Studio програмно:

```powershell
& "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
```

## Правило Для Наступних Моделей

Перед роботою з кодом перевірити:

```powershell
git status --short --branch
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo test
```

Після змін у Rust-коді запускати:

```powershell
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
```

Якщо змінюються залежності в `Cargo.toml`, треба оновити і комітити `Cargo.lock`.

`target/` не комітити: це локальний build output і він ігнорується через `.gitignore`.
