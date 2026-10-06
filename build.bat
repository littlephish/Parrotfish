@echo off
setlocal
cd /d "%~dp0"

where cargo >nul 2>nul
if errorlevel 1 (
    echo cargo was not found on PATH. Install Rust from https://rustup.rs and run this again.
    exit /b 1
)

echo Building PhishSpeak in release mode. The first build takes several minutes.
cargo build --release -p ps-app
if errorlevel 1 (
    echo.
    echo Build failed. The compiler output above says why.
    exit /b 1
)

echo.
echo Done: %~dp0target\release\ps-app.exe
endlocal
