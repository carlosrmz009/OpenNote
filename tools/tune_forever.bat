@echo off
rem Keep improving every part of the engine, for as long as this window is open.
rem
rem Each target is searched for a few hours and then the next one, round and round.
rem Everything a search needs to carry on is written after every generation, so the
rem handover costs nothing but the minute it takes to read the scores back in, and the
rem whole thing can be stopped at any moment — closing the window, a power cut — and
rem started again.
rem
rem     tools\tune_forever.bat D:\PDMX\PDMX\data
rem
rem To leave it running without it getting in the way, start it at low priority:
rem
rem     start "opennote tune" /low /min tools\tune_forever.bat D:\PDMX\PDMX\data
setlocal
if "%~1"=="" (
    echo usage: tune_forever.bat ^<PDMX\data folder^> [extra opennote arguments...]
    exit /b 1
)
set SCORES=%~1
shift
:loop
target\release\opennote.exe tune --target hands --scores "%SCORES%" --hours 4 %2 %3 %4 %5 %6
target\release\opennote.exe tune --target play --scores "%SCORES%" --hours 4 %2 %3 %4 %5 %6
goto loop
