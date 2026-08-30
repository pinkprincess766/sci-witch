@echo off
setlocal
cd /d "%~dp0"

if not exist "%~dp0sciwhisper.exe" (
  echo Не найден sciwhisper.exe рядом с launcher.
  pause
  exit /b 1
)
powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "%~dp0SciWhisper-App.ps1"
if errorlevel 1 (
  echo Не удалось запустить SciWhisper.
  pause
  exit /b 1
)
