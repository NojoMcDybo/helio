@echo off
rem Installiert den aktuellen Release-Build von Helio und startet ihn.
rem Per Doppelklick im Explorer starten - NICHT aus der Claude-App heraus:
rem Prozesse, die die Claude-App startet, schreiben in einen umgeleiteten AppData-Ordner.
rem Nimmt den neuesten Installer im Build-Ordner (Versionsnummer egal).
set "SETUP="
for /f "delims=" %%f in ('dir /b /o:d "%~dp0..\src-tauri\target\release\bundle\nsis\Helio_*_x64-setup.exe" 2^>nul') do set "SETUP=%~dp0..\src-tauri\target\release\bundle\nsis\%%f"
if not defined SETUP (echo Kein Installer gefunden. Erst "npm run tauri build". & pause & exit /b 1)
taskkill /im helio.exe /f >nul 2>&1
"%SETUP%" /S
timeout /t 2 /nobreak >nul
start "" "%LOCALAPPDATA%\Helio\helio.exe"
