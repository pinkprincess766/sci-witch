@echo off
setlocal
cd /d "%~dp0"

powershell.exe -NoProfile -STA -WindowStyle Hidden -ExecutionPolicy Bypass -File "%~dp0SciWhisper-Settings.ps1"
if errorlevel 1 (
  echo Не удалось открыть настройки SciWhisper.
  pause
)
