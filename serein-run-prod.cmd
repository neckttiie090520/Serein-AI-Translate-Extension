@echo off
setlocal
set "SEREIN_TRANSLATE_HOST=127.0.0.1:5050"
set "SEREIN_TRANSLATE_TARGET=th"
cd /d C:\Users\nextzus\Projects\Serein
start "" "target\release\serein.exe"
