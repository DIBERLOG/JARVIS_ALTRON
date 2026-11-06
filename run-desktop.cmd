@echo off
chcp 65001 >nul
title JARVIS ALTRON - сборка и запуск
cd /d "%~dp0frontend"
echo [1/3] Сборка интерфейса (npm run build)...
call npm run build || goto :fail
cd /d "%~dp0"
echo [2/3] Сборка окна и голосового ядра...
cargo build --locked --release -p jarvis-app -p jarvis-gui || goto :fail
rem Voice scripts run from the release folder: copy edited ones next to the binaries.
xcopy /D /Y /Q "%~dp0resources\tts\*.py" "%~dp0target\release\resources\tts\" >nul
echo [3/3] Запуск JARVIS ALTRON...
start "" /d "%~dp0target\release" "%~dp0target\release\jarvis-gui-sync-v2-lab.exe"
timeout /t 5 >nul 2>&1
exit /b 0
:fail
echo.
echo Сборка остановилась с ошибкой - смотрите вывод выше.
pause
