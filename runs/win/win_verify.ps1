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
$title = 'dedupe - Windows verification'
$host.UI.RawUI.WindowTitle = $title
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

# a picture of the window this console is shown in, as drawn (QOL-10)
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
public static class DedupeWin {
    [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr p);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

    public static string Text(IntPtr h) {
        StringBuilder s = new StringBuilder(512);
        GetWindowText(h, s, s.Capacity);
        return s.ToString();
    }
    public static string Class(IntPtr h) {
        StringBuilder s = new StringBuilder(256);
        GetClassName(h, s, s.Capacity);
        return s.ToString();
    }
    // the frame as drawn: DWM's bounds leave out the invisible resize border
    public static RECT Frame(IntPtr h) {
        RECT r;
        if (DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))) != 0) GetWindowRect(h, out r);
        return r;
    }
    public static string Size(IntPtr h) {
        RECT r = Frame(h);
        return (r.Right - r.Left) + " x " + (r.Bottom - r.Top);
    }
    static bool Sized(IntPtr h) {
        RECT r = Frame(h);
        return r.Right - r.Left >= 50 && r.Bottom - r.Top >= 50;
    }
    // Windows Terminal's window, or the classic console's
    public static bool Terminal(IntPtr h) {
        string c = Class(h);
        return c == "CASCADIA_HOSTING_WINDOW_CLASS" || c == "ConsoleWindowClass";
    }
    static IntPtr ByTitle(string part) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate (IntPtr h, IntPtr p) {
            if (IsWindowVisible(h) && Terminal(h) && Text(h).IndexOf(part, StringComparison.OrdinalIgnoreCase) >= 0) {
                found = h;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    // The window that shows this console. A classic console has its own
    // window. In Windows Terminal - and wherever Windows hands a console to
    // its default terminal, which leaves no WT_SESSION - the console's
    // window is a hidden 0 x 0 stand-in owned by the terminal's window.
    // Then: that owner; a terminal window carrying this run's title; the
    // terminal in front. Never a window that is not a terminal.
    public static IntPtr Pick(string title, out string how) {
        IntPtr con = GetConsoleWindow();
        if (con != IntPtr.Zero && IsWindowVisible(con) && (Sized(con) || IsIconic(con)) && Terminal(con)) {
            how = "this console's window";
            return con;
        }
        if (con != IntPtr.Zero) {
            IntPtr own = GetAncestor(con, 3); // GA_ROOTOWNER
            if (own != IntPtr.Zero && own != con && IsWindowVisible(own) && Terminal(own)) {
                how = "the terminal window that owns this console";
                return own;
            }
        }
        IntPtr t = ByTitle(title);
        if (t != IntPtr.Zero) {
            how = "the terminal window titled '" + title + "'";
            return t;
        }
        IntPtr f = GetForegroundWindow();
        if (f != IntPtr.Zero && Terminal(f)) {
            how = "the terminal in front";
            return f;
        }
        how = null;
        return IntPtr.Zero;
    }

    // why no window was found, for the log
    public static string Why(string title) {
        IntPtr con = GetConsoleWindow();
        string s = "console window ";
        if (con == IntPtr.Zero) {
            s += "none";
        } else {
            s += Class(con) + " " + Size(con) + (IsWindowVisible(con) ? " shown" : " hidden");
            IntPtr own = GetAncestor(con, 3);
            s += ", owner " + (own == IntPtr.Zero || own == con ? "none" : Class(own) + " " + Size(own));
        }
        IntPtr f = GetForegroundWindow();
        s += ", no terminal titled '" + title + "', in front " + (f == IntPtr.Zero ? "nothing" : Class(f));
        return s;
    }

    static bool Blank(Bitmap b) {
        Color first = b.GetPixel(0, 0);
        for (int y = 0; y < 24; y++) {
            for (int x = 0; x < 24; x++) {
                Color c = b.GetPixel(x * (b.Width - 1) / 23, y * (b.Height - 1) / 23);
                if (c.R != first.R || c.G != first.G || c.B != first.B) return false;
            }
        }
        return true;
    }

    // The window as a picture of its frame, saved to path; what was done,
    // or null. The window draws itself (PrintWindow with
    // PW_RENDERFULLCONTENT, composed content included), so one behind
    // another comes out whole; a blank drawing falls back to the screen.
    public static string Capture(IntPtr h, string path) {
        if (IsIconic(h)) {
            ShowWindow(h, 9); // SW_RESTORE
            Thread.Sleep(600);
        }
        RECT w;
        GetWindowRect(h, out w);
        RECT f = Frame(h);
        int ww = w.Right - w.Left, wh = w.Bottom - w.Top;
        int fw = f.Right - f.Left, fh = f.Bottom - f.Top;
        if (fw < 50 || fh < 50) return null;
        string method = "drawn by the window";
        // no alpha channel: GDI leaves it unset, which would read as transparent
        using (Bitmap full = new Bitmap(ww, wh, PixelFormat.Format32bppRgb)) {
            using (Graphics g = Graphics.FromImage(full)) {
                IntPtr hdc = g.GetHdc();
                bool drawn = PrintWindow(h, hdc, 2); // PW_RENDERFULLCONTENT
                g.ReleaseHdc(hdc);
                if (!drawn || Blank(full)) {
                    SetForegroundWindow(h);
                    Thread.Sleep(400);
                    g.CopyFromScreen(w.Left, w.Top, 0, 0, full.Size);
                    method = "copied from the screen" + (GetForegroundWindow() == h ? "" : ", another window may be over it");
                }
            }
            Rectangle crop = new Rectangle(f.Left - w.Left, f.Top - w.Top, fw, fh);
            if (!new Rectangle(0, 0, ww, wh).Contains(crop)) crop = new Rectangle(0, 0, ww, wh);
            using (Bitmap pic = full.Clone(crop, PixelFormat.Format24bppRgb)) {
                pic.Save(path, ImageFormat.Png);
            }
            return crop.Width + " x " + crop.Height + "   " + method;
        }
    }

    // ---- keys into this console's own input, and pictures between them
    // (Phase 16, TECH-13): what a person would press, queued while a live
    // `find` runs in the same console and reads them as typed keys
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr GetStdHandle(int n);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool WriteConsoleInputW(IntPtr h, KeyRecord[] recs, uint n, out uint written);
    [StructLayout(LayoutKind.Explicit, CharSet = CharSet.Unicode)]
    public struct KeyRecord {
        [FieldOffset(0)] public ushort EventType;
        [FieldOffset(4)] public int KeyDown;
        [FieldOffset(8)] public ushort RepeatCount;
        [FieldOffset(10)] public ushort VirtualKeyCode;
        [FieldOffset(12)] public ushort VirtualScanCode;
        [FieldOffset(14)] public char UnicodeChar;
        [FieldOffset(16)] public uint ControlKeyState;
    }
    static KeyRecord Rec(ushort vk, char c, bool down) {
        KeyRecord r = new KeyRecord();
        r.EventType = 1; // KEY_EVENT
        r.KeyDown = down ? 1 : 0;
        r.RepeatCount = 1;
        r.VirtualKeyCode = vk;
        r.UnicodeChar = c;
        return r;
    }
    static bool Press(ushort vk, char c) {
        uint n;
        KeyRecord[] recs = { Rec(vk, c, true), Rec(vk, c, false) };
        return WriteConsoleInputW(GetStdHandle(-10), recs, 2, out n) && n == 2;
    }
    static bool Type(string s) {
        foreach (char c in s) {
            ushort vk = (ushort)char.ToUpperInvariant(c);
            if (c == '/') vk = 0xBF;
            if (c == '_') vk = 0xBD;
            if (!Press(vk, c)) return false;
            Thread.Sleep(80);
        }
        return true;
    }
    public static string Driven = "";
    static Thread driver;
    // a plan of steps, one per line: "wait 6000", "type castle",
    // "enter", "esc", "shot <path>"; run on a thread of its own while the
    // caller runs `find` in this console. What it did lands in Driven.
    public static void Drive(string plan, IntPtr window) {
        Driven = "";
        driver = new Thread(delegate () {
            StringBuilder log = new StringBuilder();
            foreach (string raw in plan.Split('\n')) {
                string step = raw.Trim();
                if (step.Length == 0) continue;
                string arg = step.IndexOf(' ') > 0 ? step.Substring(step.IndexOf(' ') + 1) : "";
                bool ok = true;
                string said = "";
                if (step.StartsWith("wait ")) { Thread.Sleep(int.Parse(arg)); continue; }
                else if (step.StartsWith("type ")) ok = Type(arg);
                else if (step == "enter") ok = Press(0x0D, '\r');
                else if (step == "esc") ok = Press(0x1B, (char)27);
                else if (step.StartsWith("shot ")) { said = Capture(window, arg) ?? "no picture"; ok = said != "no picture"; }
                log.AppendLine(step + (ok ? "   ok" : "   FAILED") + (said.Length > 0 ? "   " + said : ""));
            }
            Driven = log.ToString();
        });
        driver.IsBackground = true;
        driver.Start();
    }
    public static void Join() { if (driver != null) driver.Join(60000); }
}
"@
function Shot($name) {
    [void][DedupeWin]::SetProcessDPIAware()
    # this run's title again (a command since may have changed it); the
    # terminal shows it, and Pick can find the window by it
    $host.UI.RawUI.WindowTitle = $title
    Start-Sleep -Milliseconds 500
    $how = $null
    $h = [DedupeWin]::Pick($title, [ref]$how)
    if ($h -eq [IntPtr]::Zero) { Save $name "no window to capture: $([DedupeWin]::Why($title))"; return }
    $got = [DedupeWin]::Capture($h, (Join-Path $log "$name.png"))
    if (-not $got) { Save $name "no window to capture: $how is $([DedupeWin]::Size($h))"; return }
    Save $name "captured $got   $how ($([DedupeWin]::Class($h)))   host=$($Host.Name)  WT_SESSION=$env:WT_SESSION"
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

# the workspace and v, photographed (Phase 16, TECH-13): keys queued into
# this console's own input while a live find runs in it, so the sixel
# pictures Windows Terminal draws are seen without a key pressed by hand
Step 'the workspace and v on a near pair, photographed (keys queued into this console)'
Clear-Host
[void][DedupeWin]::SetProcessDPIAware()
$host.UI.RawUI.WindowTitle = $title
Start-Sleep -Milliseconds 500
$how = $null
$h = [DedupeWin]::Pick($title, [ref]$how)
if ($h -eq [IntPtr]::Zero) {
    Save 'view' "no window to capture: $([DedupeWin]::Why($title))"
} else {
    $plan = @(
        'wait 7000', "shot $(Join-Path $log 'workspace.png')",
        'type /castle', 'enter', 'wait 800', 'type v', 'wait 4000',
        "shot $(Join-Path $log 'view.png')", 'esc', 'wait 800', 'type q'
    ) -join "`n"
    [DedupeWin]::Drive($plan, $h)
    & $exe find (Join-Path $sample 'photos') . img
    [DedupeWin]::Join()
    Save 'view' (@("window: $how ($([DedupeWin]::Class($h)))   WT_SESSION=$env:WT_SESSION") + ([DedupeWin]::Driven -split "`r?`n" | Where-Object { $_ }))
}

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
    $v = Join-Path $log 'view.txt'
    $vt = if (Test-Path $v) { Get-Content $v -Encoding UTF8 } else { @('not run') }
    $vShot = $vt | Where-Object { $_ -match '^shot .*view\.png' } | Select-Object -First 1
    $vOk = if ($vShot -match '   ok   ') { $true } elseif ($vt[0] -eq 'not run') { $null } else { $false }
    Line 'v picture' $vOk $(if ($vOk) { (($vShot -split '   ok   ')[1]) + '   runs\win\log\view.png, workspace.png' } else { ($vt | Select-Object -Last 1) })
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
