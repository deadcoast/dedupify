# dedupe - Windows verification run. Builds the release binary, runs the
# suite, then exercises every command on runs\win\sample and logs it all
# under runs\win\log. Run from anywhere; everything is anchored below.
$ErrorActionPreference = 'Continue'
$repo = 'C:\github\dedupify'
$log = Join-Path $repo 'runs\win\log'
$sample = Join-Path $repo 'runs\win\sample'
$model = Join-Path $repo 'runs\win\models\sscd_disc_mixup.onnx'
New-Item -ItemType Directory -Force -Path $log | Out-Null
Set-Location $repo
$host.UI.RawUI.WindowTitle = 'dedupe - Windows verification'
$exe = Join-Path $repo 'target\release\dedupe.exe'

function Step($name) { Write-Host ""; Write-Host "==== $name" -ForegroundColor Cyan }
# native command through cmd so stderr merges as plain text (PowerShell 5.1
# would wrap every stderr line in an error record)
function Run($name, $cmd) {
    $path = Join-Path $log "$name.txt"
    $t0 = Get-Date
    cmd /c "$cmd 2>&1" | Tee-Object -FilePath $path
    "exit $LASTEXITCODE  $([int]((Get-Date) - $t0).TotalSeconds) s" | Tee-Object -Append -FilePath $path
}
function Tree($name) {
    Get-ChildItem $sample -Recurse -File | Sort-Object FullName |
        ForEach-Object { "$($_.FullName.Substring($sample.Length))  $($_.Length)" } |
        Set-Content (Join-Path $log "$name.txt")
}
function Snapshot() {
    Get-ChildItem $sample -Recurse -File | Where-Object { $_.FullName -notmatch '\\_milked\\' } | Sort-Object FullName |
        ForEach-Object { "$($_.FullName.Substring($sample.Length))  $($_.Length)  $((Get-FileHash $_.FullName -Algorithm SHA256).Hash)" }
}

Step 'environment'
$env_lines = @(
    "date      $(Get-Date -Format s)",
    "windows   $([System.Environment]::OSVersion.VersionString)  $((Get-CimInstance Win32_OperatingSystem).Caption)",
    "cpu       $((Get-CimInstance Win32_Processor).Name)  $((Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors) threads",
    "ram       $([math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory/1GB,1)) GB",
    "terminal  WT_SESSION=$env:WT_SESSION  host=$($Host.Name)  psversion=$($PSVersionTable.PSVersion)  cols=$($Host.UI.RawUI.WindowSize.Width)",
    "codepage  $([Console]::OutputEncoding.WebName)  $(cmd /c chcp)",
    "rustc     $(cmd /c 'rustc -V 2>&1')",
    "cargo     $(cmd /c 'cargo -V 2>&1')",
    "ffmpeg    $(cmd /c 'ffmpeg -version 2>&1' | Select-Object -First 1)",
    "ffprobe   $(cmd /c 'ffprobe -version 2>&1' | Select-Object -First 1)",
    "git       $(git rev-parse --short HEAD)  $((git status --porcelain | Measure-Object).Count) changed paths"
)
$env_lines | Tee-Object -FilePath (Join-Path $log 'env.txt')

Step 'git pull'
Run 'git_pull' 'git pull --ff-only origin main'

Step 'cargo build --release'
Run 'build' 'cargo build --release'
if (-not (Test-Path $exe)) { Write-Host 'no binary; stopping' -ForegroundColor Red; "NO BINARY" | Set-Content (Join-Path $log 'DONE.txt'); Start-Sleep 600; exit 1 }
"binary    $((Get-Item $exe).Length) bytes" | Tee-Object -Append -FilePath (Join-Path $log 'build.txt')

Step 'cargo test --release'
Run 'test' 'cargo test --release'

if (Test-Path $model) {
    Step 'cargo test --release --test e2e (with the model)'
    $env:DEDUPE_MODEL = $model
    Run 'test_model' 'cargo test --release --test e2e'
    Remove-Item Env:\DEDUPE_MODEL
}

Step 'cargo test -- --ignored purge_then_undo_through_the_bin (the real Recycle Bin)'
Run 'test_recycle' 'cargo test --release --test e2e -- --ignored purge_then_undo_through_the_bin'

Step 'doctor'
Run 'doctor' "`"$exe`" --doctor"
Step 'find'
Run 'find_all' "`"$exe`" find $sample ."
Run 'find_img' "`"$exe`" find $sample . img"
Run 'find_vid' "`"$exe`" find $sample . vid"
Run 'find_photos_flat' "`"$exe`" find $sample\photos"
Run 'find_quick' "`"$exe`" find $sample . -q"
if (Test-Path $model) {
    $env:DEDUPE_MODEL = $model
    Run 'find_img_model' "`"$exe`" find $sample . img"
    Remove-Item Env:\DEDUPE_MODEL
}

Step 'milk, then undo'
$before = Snapshot
$before | Set-Content (Join-Path $log 'tree_before.txt')
Run 'milk_dry' "`"$exe`" milk $sample . -n"
Run 'milk' "`"$exe`" milk $sample ."
Tree 'tree_after_milk'
Run 'milk_undo' "`"$exe`" --undo $sample"
$after = Snapshot
$after | Set-Content (Join-Path $log 'tree_after_milk_undo.txt')
$msg = if (Compare-Object $before $after) { "TREE DIFFERS after milk+undo" } else { "tree identical after milk+undo: $($after.Count) files, every SHA-256 equal" }
$msg | Tee-Object -FilePath (Join-Path $log 'milk_roundtrip.txt')
$left = if (Test-Path (Join-Path $sample '_milked')) { Get-ChildItem (Join-Path $sample '_milked') -Recurse -Force | ForEach-Object { $_.FullName } } else { '_milked gone' }
$left | Set-Content (Join-Path $log 'milked_dir_after_undo.txt')

Step 'purge, then undo (Recycle Bin)'
Run 'purge_dry' "`"$exe`" purge $sample . -n"
Run 'purge' "`"$exe`" purge $sample ."
Tree 'tree_after_purge'
Run 'purge_undo' "`"$exe`" --undo $sample"
$after2 = Snapshot
$after2 | Set-Content (Join-Path $log 'tree_after_purge_undo.txt')
$msg = if (Compare-Object $before $after2) { "TREE DIFFERS after purge+undo" } else { "tree identical after purge+undo: $($after2.Count) files, every SHA-256 equal" }
$msg | Tee-Object -FilePath (Join-Path $log 'purge_roundtrip.txt')
Run 'undo_nothing' "`"$exe`" --undo $sample"

Step 'help and errors'
Run 'help' "`"$exe`" --help"
Run 'err_two_kinds' "`"$exe`" find $sample img vid"
Run 'err_missing' "`"$exe`" find C:\no\such\folder"
$j = Join-Path $env:LOCALAPPDATA 'dedupe'
@("app dir $j") + $(if (Test-Path $j) { Get-ChildItem $j -Recurse | ForEach-Object { "$($_.FullName)  $($_.Length)" } } else { 'absent' }) | Set-Content (Join-Path $log 'journal_dir.txt')

Step 'done - leaving a live find on screen'
"DONE $(Get-Date -Format s)" | Set-Content (Join-Path $log 'DONE.txt')
Start-Sleep -Seconds 2
Clear-Host
& $exe find $sample . img
Write-Host ""
Write-Host "logs in runs\win\log - this window closes itself in 20 minutes" -ForegroundColor DarkGray
Start-Sleep -Seconds 1200
