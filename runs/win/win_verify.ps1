# dedupe - Windows verification run. Builds the release binary, runs the
# suite (with the real-format files and the Ctrl-C test), then exercises
# every command on runs\win\sample and logs it all, UTF-8, under
# runs\win\log. Run from anywhere; everything is anchored below.
# -Pulled: started by an older copy of this script after its pull (below).
param([switch]$Pulled)
$ErrorActionPreference = 'Continue'
$repo = 'C:\github\dedupify'
$log = Join-Path $repo 'runs\win\log'
$sample = Join-Path $repo 'runs\win\sample'
$model = Join-Path $repo 'runs\win\models\sscd_disc_mixup.onnx'
$formats = Join-Path $repo 'runs\win\formats'
New-Item -ItemType Directory -Force -Path $log | Out-Null
Set-Location $repo
$host.UI.RawUI.WindowTitle = 'dedupe - Windows verification'
$exe = Join-Path $repo 'target\release\dedupe.exe'

# dedupe writes UTF-8; read native output as UTF-8 and write every log as
# UTF-8 without a BOM (PowerShell 5.1's Tee-Object and Out-File write
# UTF-16, Set-Content the ANSI code page)
$utf8 = New-Object System.Text.UTF8Encoding $false
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8
function Save($name, $lines) {
    [System.IO.File]::WriteAllLines((Join-Path $log "$name.txt"), [string[]]@($lines), $utf8)
}
function Add($name, $line) {
    [System.IO.File]::AppendAllText((Join-Path $log "$name.txt"), "$line`r`n", $utf8)
}
function Show($name, $lines) { $lines | ForEach-Object { Write-Host $_ }; Save $name $lines }

function Step($name) { Write-Host ""; Write-Host "==== $name" -ForegroundColor Cyan }
# native command through cmd so stderr merges as plain text (PowerShell 5.1
# would wrap every stderr line in an error record); each line is shown and
# logged as it arrives
function Run($name, $cmd) {
    $path = Join-Path $log "$name.txt"
    [System.IO.File]::WriteAllText($path, '', $utf8)
    $t0 = Get-Date
    cmd /c "$cmd 2>&1" | ForEach-Object { Write-Host $_; Add $name $_ }
    Add $name "exit $LASTEXITCODE  $([int]((Get-Date) - $t0).TotalSeconds) s"
}
function Tree($name) {
    Save $name (Get-ChildItem $sample -Recurse -File | Sort-Object FullName |
        ForEach-Object { "$($_.FullName.Substring($sample.Length))  $($_.Length)" })
}
function Snapshot() {
    Get-ChildItem $sample -Recurse -File | Where-Object { $_.FullName -notmatch '\\_milked\\' } | Sort-Object FullName |
        ForEach-Object { "$($_.FullName.Substring($sample.Length))  $($_.Length)  $((Get-FileHash $_.FullName -Algorithm SHA256).Hash)" }
}

# a picture of this console window, as it is on screen (QOL-10)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class DedupeWin {
    [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
function Shot($name) {
    [void][DedupeWin]::SetProcessDPIAware()
    # in Windows Terminal the console window is hidden; the terminal is the
    # foreground window
    $h = if ($env:WT_SESSION) { [DedupeWin]::GetForegroundWindow() } else { [DedupeWin]::GetConsoleWindow() }
    [void][DedupeWin]::SetForegroundWindow($h)
    Start-Sleep -Milliseconds 400
    $r = New-Object DedupeWin+RECT
    [void][DedupeWin]::GetWindowRect($h, [ref]$r)
    $w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
    if ($w -lt 50 -or $hh -lt 50) { Save $name "no window to capture ($w x $hh)"; return }
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size)
    $bmp.Save((Join-Path $log "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Save $name "captured $w x $hh  host=$($Host.Name)  WT_SESSION=$env:WT_SESSION"
}

Step 'environment'
Show 'env' @(
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

Step 'git pull'
if ($Pulled) {
    Write-Host 'pulled by the copy of this script that started the run'
} else {
    $self = (Get-FileHash $PSCommandPath -Algorithm SHA256).Hash
    # local edits under runs\win are this script's own earlier copies; the repo's version wins
    Run 'git_checkout' 'git checkout -- runs/win'
    Run 'git_pull' 'git pull --ff-only origin main'
    # PowerShell runs the copy it read at the start; when the checkout or
    # the pull changed this script, the new copy runs the rest of the run
    if ((Get-FileHash $PSCommandPath -Algorithm SHA256).Hash -ne $self) {
        Write-Host 'the pull changed this script; the new copy runs the rest' -ForegroundColor Yellow
        & powershell -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath -Pulled
        exit $LASTEXITCODE
    }
}

Step 'cargo build --release'
Run 'build' 'cargo build --release'
if (-not (Test-Path $exe)) { Write-Host 'no binary; stopping' -ForegroundColor Red; Save 'DONE' 'NO BINARY'; Start-Sleep 600; exit 1 }
Add 'build' "binary    $((Get-Item $exe).Length) bytes"

Step 'cargo test --release (includes the Ctrl-C test: CTRL_BREAK to a child milk)'
Run 'test' 'cargo test --release'

if (Test-Path $model) {
    Step 'cargo test --release --test e2e (with the model)'
    $env:DEDUPE_MODEL = $model
    Run 'test_model' 'cargo test --release --test e2e'
    Remove-Item Env:\DEDUPE_MODEL
}

Step 'cargo test -- --ignored purge_then_undo_through_the_bin (the real Recycle Bin)'
Run 'test_recycle' 'cargo test --release --test e2e -- --ignored purge_then_undo_through_the_bin'

Step 'formats: real phone and camera files (fetched once into runs\win\formats)'
# a small part of two public test-image sets: iPhone and Sony HEIC/HEIF,
# AVIF transform files, one RAW per maker; about 90 MB, kept for next time
if (-not (Test-Path (Join-Path $formats 'mei\heic'))) {
    New-Item -ItemType Directory -Force -Path $formats | Out-Null
    Run 'formats_fetch_mei' "git clone -q --depth 1 --filter=blob:none --sparse https://github.com/drewnoakes/metadata-extractor-images `"$formats\mei`""
    Run 'formats_sparse_mei' "git -C `"$formats\mei`" sparse-checkout set --no-cone /heic/IMG_1034.heic /heic/IMG_2927.HEIC `"/heic/Issue 263 dotnet.heic`" /heic/Issue*487.heic `"/heif/Sony ILCE-7M4 (A7M4).heif`" `"/avif/Issue 649.avif`" `"/cr3/Canon EOS R3.cr3`" `"/nef/Nikon D70.nef`" `"/arw/Sony NEX-5N.arw`" `"/dng/Leica T (Typ 701).dng`" `"/raf/Fujifilm X-E1.raf`" `"/orf/Olympus E-M5.orf`" `"/rw2/Panasonic DMC-GF7.rw2`""
    Run 'formats_fetch_heif' "git clone -q --depth 1 --filter=blob:none --sparse https://github.com/strukturag/libheif `"$formats\strukturag_libheif`""
    Run 'formats_sparse_heif' "git -C `"$formats\strukturag_libheif`" sparse-checkout set examples"
    Run 'formats_fetch_avif' "git clone -q --depth 1 --filter=blob:none --sparse https://github.com/AOMediaCodec/av1-avif `"$formats\AOMediaCodec_av1-avif`""
    Run 'formats_sparse_avif' "git -C `"$formats\AOMediaCodec_av1-avif`" sparse-checkout set --no-cone /testFiles/Link-U/ /testFiles/Microsoft/Summer_in_Tomsk_720p_5x4_grid.avif /testFiles/Microsoft/Ronda_rotate90.avif /testFiles/Microsoft/Chimera_8bit_cropped_480x256.avif /testFiles/Microsoft/Chimera_10bit_cropped_to_1920x1008.avif /testFiles/Microsoft/Monochrome.avif"
}
$env:DEDUPE_FORMATS = $formats
Run 'test_formats' 'cargo test --release --test formats_corpus -- --ignored --nocapture'
Remove-Item Env:\DEDUPE_FORMATS

Step 'doctor'
Run 'doctor' "`"$exe`" --doctor"
Step 'find'
Run 'find_all' "`"$exe`" find $sample ."
Run 'find_img' "`"$exe`" find $sample . img"
Run 'find_vid' "`"$exe`" find $sample . vid"
Run 'find_photos_flat' "`"$exe`" find $sample\photos"
Run 'find_quick' "`"$exe`" find $sample . -q"
Run 'find_formats' "`"$exe`" find `"$formats`" . img"
if (Test-Path $model) {
    $env:DEDUPE_MODEL = $model
    Run 'find_img_model' "`"$exe`" find $sample . img"
    Remove-Item Env:\DEDUPE_MODEL
}

Step 'milk, then undo'
$before = Snapshot
Save 'tree_before' $before
Run 'milk_dry' "`"$exe`" milk $sample . -n"
Run 'milk' "`"$exe`" milk $sample ."
Tree 'tree_after_milk'
Run 'milk_undo' "`"$exe`" --undo $sample"
$after = Snapshot
Save 'tree_after_milk_undo' $after
$msg = if (Compare-Object $before $after) { "TREE DIFFERS after milk+undo" } else { "tree identical after milk+undo: $($after.Count) files, every SHA-256 equal" }
Show 'milk_roundtrip' $msg
$left = if (Test-Path (Join-Path $sample '_milked')) { Get-ChildItem (Join-Path $sample '_milked') -Recurse -Force | ForEach-Object { $_.FullName } } else { '_milked gone' }
Save 'milked_dir_after_undo' $left

Step 'purge, then undo (Recycle Bin)'
Run 'purge_dry' "`"$exe`" purge $sample . -n"
Run 'purge' "`"$exe`" purge $sample ."
Tree 'tree_after_purge'
Run 'purge_undo' "`"$exe`" --undo $sample"
$after2 = Snapshot
Save 'tree_after_purge_undo' $after2
$msg = if (Compare-Object $before $after2) { "TREE DIFFERS after purge+undo" } else { "tree identical after purge+undo: $($after2.Count) files, every SHA-256 equal" }
Show 'purge_roundtrip' $msg
Run 'undo_nothing' "`"$exe`" --undo $sample"

Step 'help and errors'
Run 'help' "`"$exe`" --help"
Run 'err_two_kinds' "`"$exe`" find $sample img vid"
Run 'err_missing' "`"$exe`" find C:\no\such\folder"
$j = Join-Path $env:LOCALAPPDATA 'dedupe'
Save 'journal_dir' (@("app dir $j") + $(if (Test-Path $j) { Get-ChildItem $j -Recurse | ForEach-Object { "$($_.FullName)  $($_.Length)" } } else { 'absent' }))

Step 'colour on this console: the doctor, live, and a picture of the window'
Clear-Host
& $exe --doctor
Shot 'colour'

Save 'DONE' "DONE $(Get-Date -Format s)"
Step 'done - a live find on the sample opens the workspace: space select, k keeper, v view, o reveal, m/p act, u undo, q quit'
Start-Sleep -Seconds 2
Clear-Host
& $exe find $sample .
Write-Host ""
Write-Host "logs in runs\win\log - press a key to close" -ForegroundColor DarkGray
$null = $Host.UI.RawUI.ReadKey('NoEcho,IncludeKeyDown')
