@echo off
setlocal
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
set "PATH=%LOCALAPPDATA%\bin\NASM;%USERPROFILE%\.cargo\bin;%PATH%"
cd /d C:\Users\nextzus\Projects\Serein
rustup run 1.99.1 cargo %*
