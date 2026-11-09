@echo off
rem dedupe on the sample tree: exact copies, then images, then videos. Window stays open.
set EXE=%~dp0..\..\target\release\dedupe.exe
"%EXE%" find "%~dp0sample" .
echo.
"%EXE%" find "%~dp0sample" . img
echo.
"%EXE%" find "%~dp0sample" . vid
echo.
pause
