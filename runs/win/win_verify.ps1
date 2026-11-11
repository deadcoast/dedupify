# dedupe - Windows verification run. Builds the release binary, runs the
# suite (with the real-format files and the Ctrl-C test), then exercises
# every command on runs\win\sample and logs it all, UTF-8, under
# runs\win\log - each step in its own file and the whole run, in order,
# in RUN.txt. Ends on a summary, then the live workspace once a key is
# pressed. Run from anywhere; everything is anchored below.
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
# RUN.txt: every step's lines in the order they ran, one file to attach
[System.IO.File]::WriteAllText((Join-Path $log 'RUN.txt'), '', $utf8)
function Save($name, $lines) {
    [System.IO.File]::WriteAllLines((Join-Path $log "$name.txt"), [string[]]@($lines), $utf8)
    [System.IO.File]::AppendAllText((Join-Path $log 'RUN.txt'), ((@("---- $name") + @($lines)) -join "`r`n") + "`r`n", $utf8)
}
function Add($name, $line) {
    [System.IO.File]::AppendAllText((Join-Path $log "$name.txt"), "$line`r`n", $utf8)
    [System.IO.File]::AppendAllText((Join-Path $log 'RUN.txt'), "$line`r`n", $utf8)
}
function Show($name, $lines) { $lines | ForEach-Object { Write-Host $_ }; Save $name $lines }

function Step($name) {
    Write-Host ""; Write-Host "==== $name" -ForegroundColor Cyan
    [System.IO.File]::AppendAllText((Join-Path $log 'RUN.txt'), "`r`n==== $name`r`n", $utf8)
}
# each Run's exit code and time, for the summary
$script:ran = [ordered]@{}
# native command through cmd so stderr merges as plain text (PowerShell 5.1
# would wrap every stderr line in an error record); each line is shown and
# logged as it arrives
function Run($name, $cmd) {
    $path = Join-Path $log "$name.txt"
    [System.IO.File]::WriteAllText($path, '', $utf8)
    [System.IO.File]::AppendAllText((Join-Path $log 'RUN.txt'), "---- $name   $cmd`r`n", $utf8)
    $t0 = Get-Date
    cmd /c "$cmd 2>&1" | ForEach-Object { Write-Host $_; Add $name $_ }
    $code = $LASTEXITCODE
    $secs = [int]((Get-Date) - $t0).TotalSeconds
    Add $name "exit $code  $secs s"
    $script:ran[$name] = @{ code = $code; secs = $secs }
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
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
function Shot($name) {
    [void][DedupeWin]::SetProcessDPIAware()
    # the console's own window, unless it is hidden: in Windows Terminal,
    # and wherever Windows hands a console to its default terminal (no
    # WT_SESSION then, a hidden 0 x 0 console window), the terminal is the
    # foreground window
    $h = [DedupeWin]::GetConsoleWindow()
    if ($env:WT_SESSION -or $h -eq [IntPtr]::Zero -or -not [DedupeWin]::IsWindowVisible($h)) {
        $h = [DedupeWin]::GetForegroundWindow()
    }
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
    "ffmpeg    on PATH: $(cmd /c 'ffmpeg -version 2>&1' | Select-Object -First 1)   (dedupe also looks where installers put it: see doctor)",
    "ffprobe   on PATH: $(cmd /c 'ffprobe -version 2>&1' | Select-Object -First 1)",
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

Step 'the Ctrl-C test again, its own words shown (sent, or skipped without a console)'
Run 'test_ctrlc' 'cargo test --release --test e2e interrupted_milk_leaves_a_journal_undo_can_use -- --nocapture'

Step 'a Windows pseudo console: the workspace driven by keys (a milk and its undo), and colour'
Run 'test_tty' 'cargo test --release --test e2e -- --nocapture find_on_a_terminal_acts_from_the_result a_terminal_gets_colour'

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

# the run on one screen: each part, what it gave, and where the rest is
function Line($label, $ok, $text) {
    $tag = if ($ok -eq $true) { 'PASS' } elseif ($ok -eq $false) { 'FAIL' } else { '    ' }
    $colour = if ($ok -eq $true) { 'Green' } elseif ($ok -eq $false) { 'Red' } else { 'Gray' }
    Write-Host ' ' -NoNewline
    Write-Host $tag -ForegroundColor $colour -NoNewline
    Write-Host ("  " + $label.PadRight(13) + $text)
    Add 'SUMMARY' (" $tag  " + $label.PadRight(13) + $text)
}
function Count($name) {
    $p = Join-Path $log "$name.txt"
    if (-not (Test-Path $p)) { return $null }
    $pass = 0; $fail = 0
    Get-Content $p -Encoding UTF8 | ForEach-Object {
        if ($_ -match 'test result: \w+\. (\d+) passed; (\d+) failed') { $pass += [int]$Matches[1]; $fail += [int]$Matches[2] }
    }
    "$pass passed, $fail failed"
}
function Ok($name) { if ($script:ran.Contains($name)) { $script:ran[$name].code -eq 0 } else { $null } }
function Secs($name) { if ($script:ran.Contains($name)) { "$($script:ran[$name].secs) s" } else { '' } }
function Grep($name, $pattern) {
    $p = Join-Path $log "$name.txt"
    if (Test-Path $p) { Get-Content $p -Encoding UTF8 | Where-Object { $_ -match $pattern } | Select-Object -First 1 } else { $null }
}
function Summary() {
    $rule = ' ' + ([string][char]0x2500) * 76
    [System.IO.File]::WriteAllText((Join-Path $log 'SUMMARY.txt'), '', $utf8)
    Write-Host ''
    Write-Host " dedupe   Windows run   $(Get-Date -Format 'yyyy-MM-dd HH:mm')   $(git rev-parse --short HEAD)"
    Write-Host $rule -ForegroundColor DarkGray
    $size = if (Test-Path $exe) { "{0:N1} MB" -f ((Get-Item $exe).Length / 1MB) } else { 'no binary' }
    Line 'build' (Ok 'build') "$(Secs 'build')   $size"
    Line 'tests' (Ok 'test') "$(Count 'test')   $(Secs 'test')"
    $c = Grep 'test_ctrlc' 'CTRL_BREAK sent|could not be sent|skipped'
    $cOk = if ((Ok 'test_ctrlc') -and $c -match 'CTRL_BREAK sent') { $true } elseif (Ok 'test_ctrlc') { $null } else { $false }
    Line 'Ctrl-C' $cOk $(if ($c) { $c.Trim() } else { 'no verdict in the log' })
    $t = Grep 'test_tty' 'pseudo-terminal'
    $tOk = if ((Ok 'test_tty') -and $t -match 'driven through') { $true } elseif (Ok 'test_tty') { $null } else { $false }
    Line 'workspace' $tOk $(if ($t) { $t.Trim() } else { 'no verdict in the log' })
    $k = Grep 'test_tty' 'colour on a terminal'
    $kOk = if ((Ok 'test_tty') -and $k) { $true } elseif (Ok 'test_tty') { $null } else { $false }
    Line 'colour codes' $kOk $(if ($k) { $k.Trim() } else { 'no verdict in the log' })
    if ($script:ran.Contains('test_model')) { Line 'with model' (Ok 'test_model') "$(Count 'test_model')" }
    Line 'Recycle Bin' (Ok 'test_recycle') "$(Count 'test_recycle')"
    Line 'formats' (Ok 'test_formats') "$(Count 'test_formats')"
    $ff = Grep 'doctor' 'ffmpeg  '
    $ffOk = if ($ff -match 'PASS') { $true } elseif ($ff) { $false } else { $null }
    Line 'ffmpeg' $ffOk $(if ($ff) { ($ff -replace '^\s*\w+\s+ffmpeg\s+', '').Trim() } else { 'no doctor line' })
    foreach ($r in @('milk_roundtrip', 'purge_roundtrip')) {
        $p = Join-Path $log "$r.txt"
        $t = if (Test-Path $p) { (Get-Content $p -Encoding UTF8 | Select-Object -First 1) } else { 'not run' }
        Line ($r -replace '_roundtrip', ' + undo') ($t -match '^tree identical') $t
    }
    $shot = Join-Path $log 'colour.txt'
    $s = if (Test-Path $shot) { (Get-Content $shot -Encoding UTF8 | Select-Object -First 1) } else { 'not run' }
    Line 'colour' $(if ($s -match '^captured') { $true } else { $false }) $(if ($s -match '^captured') { "$s   runs\win\log\colour.png" } else { $s })
    Write-Host $rule -ForegroundColor DarkGray
    Write-Host " every step, in order: runs\win\log\RUN.txt   this summary: runs\win\log\SUMMARY.txt" -ForegroundColor DarkGray
}

Step 'summary'
Summary
Write-Host ''
Write-Host ' press a key to open the live workspace on the sample (q leaves it)' -ForegroundColor Cyan
$null = $Host.UI.RawUI.ReadKey('NoEcho,IncludeKeyDown')
& $exe find $sample .
Summary
Write-Host ''
Write-Host ' press a key to close' -ForegroundColor DarkGray
$null = $Host.UI.RawUI.ReadKey('NoEcho,IncludeKeyDown')
