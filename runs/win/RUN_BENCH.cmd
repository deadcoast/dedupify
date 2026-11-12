@echo off
rem dedupe - the bench on this machine. Double-click to run; the record lands in log\BENCH.txt.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0win_bench.ps1"
pause
