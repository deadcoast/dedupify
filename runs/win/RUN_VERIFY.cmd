@echo off
rem dedupe - Windows verification. Double-click to run; logs land in log\.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0win_verify.ps1"
