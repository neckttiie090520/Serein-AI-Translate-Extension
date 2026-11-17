@echo off
setlocal
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cd /d C:\Users\nextzus\Projects\Serein\extensions
rustup run 1.99.1 cargo %*
