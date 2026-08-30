@echo off
chcp 65001 > nul
setlocal
cd /d "%~dp0"

set "SCIWHISPER_BIN=%~dp0sciwhisper.exe"
if not exist "%SCIWHISPER_BIN%" set "SCIWHISPER_BIN=%~dp0..\..\target\release\sciwhisper.exe"

if not exist "%SCIWHISPER_BIN%" (
  echo Не найден sciwhisper.exe.
  echo Используйте официальный release-архив или поместите launcher рядом с бинарником.
  pause
  exit /b 1
)

echo === SciWhisper: локальная проверка ===
"%SCIWHISPER_BIN%" self-test
if errorlevel 1 goto failed

echo.
echo === Диагностика локального Whisper ===
"%SCIWHISPER_BIN%" doctor

echo.
choice /C YN /N /M "Проверить микрофон в течение 6 секунд? [Y/N] "
if errorlevel 2 goto done

echo Произнесите: гидроксид меди два превращается в оксид меди два плюс вода
"%SCIWHISPER_BIN%" rec --seconds 6 --domain chemistry --renderer all
if errorlevel 1 goto failed

:done
echo.
echo Проверка завершена. Нажмите любую клавишу.
pause > nul
exit /b 0

:failed
echo.
echo Проверка завершилась с ошибкой. Скопируйте сообщение по инструкции docs\TESTING_RU.md.
pause
exit /b 1
