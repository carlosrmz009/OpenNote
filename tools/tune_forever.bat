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
rem The two targets are given different numbers because a score costs about three
rem hundred times as much to finger as it does to divide between the hands. The hand
rem target reads whole pieces and judges a generation on six hundred of them; the
rem playing target reads a passage from each and judges on two hundred.
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
set EXTRA=%2 %3 %4 %5 %6
:loop
target\release\opennote.exe tune --target hands --scores "%SCORES%" --hours 4 %EXTRA%
target\release\opennote.exe tune --target play --scores "%SCORES%" --hours 4 --limit 20000 --notes 120 --batch 200 %EXTRA%
goto loop
