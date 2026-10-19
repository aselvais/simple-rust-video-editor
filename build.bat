@echo off
REM Build QuickEdit for Windows (cmd fallback) and copy the release binary into
REM .\builds\ as quickedit-windows-x64.exe.
REM
REM FFmpeg is a runtime dependency, so a missing FFmpeg only warns here.

setlocal
cd /d "%~dp0"

where ffmpeg >nul 2>nul
if errorlevel 1 (
  echo warning: 'ffmpeg' not found on PATH - the editor will build, but won't work until FFmpeg is installed. 1>&2
)

echo -^> cargo build --release
call cargo build --release
if errorlevel 1 goto :error

if not exist builds mkdir builds
copy /Y target\release\quickedit.exe builds\quickedit-windows-x64.exe
echo Built: builds\quickedit-windows-x64.exe
echo Run:   builds\quickedit-windows-x64.exe

endlocal
exit /b 0

:error
echo Build failed. See the output above.
endlocal
exit /b 1
