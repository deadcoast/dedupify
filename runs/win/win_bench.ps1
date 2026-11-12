# dedupe - the bench on this machine (Phase 14, TECH-10): the stages of a
# scan on the generated corpus, with the model when it is in
# runs\win\models; the 100,000-file run; the deep tier's batch measure.
# Real folders join the corpus when listed below (only read, never
# changed). Everything is written to runs\win\log\BENCH.txt.
$ErrorActionPreference = 'Continue'
$repo = 'C:\github\dedupify'
# real photographs and videos to add, e.g. "$env:USERPROFILE\Pictures"
$photos = @()
$videos = @()

$log = Join-Path $repo 'runs\win\log'
$out = Join-Path $log 'BENCH.txt'
$model = Join-Path $repo 'runs\win\models\sscd_disc_mixup.onnx'
New-Item -ItemType Directory -Force -Path $log | Out-Null
Set-Location $repo
$utf8 = New-Object System.Text.UTF8Encoding $false
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8
[System.IO.File]::WriteAllText($out, '', $utf8)
function Run($label, $cmd) {
    Write-Host "==== $label"
    [System.IO.File]::AppendAllText($out, "`r`n==== $label   $cmd`r`n", $utf8)
    $t = Get-Date
    cmd /c "$cmd 2>&1" | ForEach-Object {
        Write-Host $_
        [System.IO.File]::AppendAllText($out, "$_`r`n", $utf8)
    }
    $secs = [int]((Get-Date) - $t).TotalSeconds
    [System.IO.File]::AppendAllText($out, "(exit $LASTEXITCODE, $secs s)`r`n", $utf8)
}

Run 'pull' 'git pull --ff-only'
$cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
$mem = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)
[System.IO.File]::AppendAllText($out, "machine   $cpu   $mem GB   $env:NUMBER_OF_PROCESSORS threads`r`n", $utf8)
if (Test-Path $model) { $env:DEDUPE_MODEL = $model }
if ($photos.Count) { $env:DEDUPE_PHOTO_DIRS = $photos -join ';' }
if ($videos.Count) { $env:DEDUPE_VIDEO_DIRS = $videos -join ';' }
Run 'stages' 'cargo test --release --test bench stages_on_a_fixed_corpus -- --ignored --nocapture'
Run 'scale' 'cargo test --release --test bench scale_run -- --ignored --nocapture'
if (Test-Path $model) {
    Run 'deep batch' 'cargo test --release --test deep_batch -- --ignored --nocapture'
}
Write-Host ""
Write-Host "Record written to $out"
