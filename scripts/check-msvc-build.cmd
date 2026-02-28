@echo off
setlocal
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat" -arch=amd64
if errorlevel 1 exit /b 1
where link
if errorlevel 1 exit /b 1
cargo check --workspace
exit /b %errorlevel%
