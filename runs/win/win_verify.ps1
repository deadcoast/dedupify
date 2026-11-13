# dedupe - Windows verification (RUN_VERIFY.cmd). Pulls, builds the
# release binary, runs the suite, exercises every command on
# runs\win\sample and photographs the live workspace. On screen: one line
# per workflow, live while it runs, folded to its mark and result once it
# is checked (docs/CLI.md 3a). Every command's own output is logged,
# UTF-8, under runs\win\log: each step in its own file, the whole run in
# order in RUN.txt, the lines of this screen in SUMMARY.txt. Ends on the
# live workspace once a key is pressed. Run from anywhere.
# -Pulled: started by the copy of this script that pulled a newer one;
# -Started: when that copy started.
param([switch]$Pulled, [string]$Started = '')
$ErrorActionPreference = 'Continue'
# no progress bars drawn over the lines
$ProgressPreference = 'SilentlyContinue'
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
$t0 = if ($Started) { New-Object DateTime ([long]$Started) } else { Get-Date }
$dot = ' ' + [char]0x00B7 + ' '
$arrow = ' ' + [char]0x2192 + ' '
$inv = [System.Globalization.CultureInfo]::InvariantCulture

# dedupe writes UTF-8; read native output as UTF-8 and write every log as
# UTF-8 without a BOM (PowerShell 5.1's Tee-Object and Out-File write
# UTF-16, Set-Content the ANSI code page)
$utf8 = New-Object System.Text.UTF8Encoding $false
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8
# the copy that pulled a newer one started RUN.txt and SUMMARY.txt
if (-not $Pulled) {
    [System.IO.File]::WriteAllText((Join-Path $log 'RUN.txt'), '', $utf8)
    [System.IO.File]::WriteAllText((Join-Path $log 'SUMMARY.txt'), '', $utf8)
}
function Save($name, $lines) {
    [System.IO.File]::WriteAllLines((Join-Path $log "$name.txt"), [string[]]@($lines), $utf8)
    [System.IO.File]::AppendAllText((Join-Path $log 'RUN.txt'), ((@("---- $name") + @($lines)) -join "`r`n") + "`r`n", $utf8)
}
function Add($name, $line) {
    [System.IO.File]::AppendAllText((Join-Path $log "$name.txt"), "$line`r`n", $utf8)
    [System.IO.File]::AppendAllText((Join-Path $log 'RUN.txt'), "$line`r`n", $utf8)
}
# each Run's exit code
$script:ran = @{}
# a native command through cmd, so stderr merges as plain text (PowerShell
# 5.1 would wrap every stderr line in an error record); each line logged as
# it arrives and handed to $each, which reads progress out of it
function Run($name, $cmd, [scriptblock]$each) {
    $path = Join-Path $log "$name.txt"
    [System.IO.File]::WriteAllText($path, '', $utf8)
    [System.IO.File]::AppendAllText((Join-Path $log 'RUN.txt'), "---- $name   $cmd`r`n", $utf8)
    $t = Get-Date
    cmd /c "$cmd 2>&1" | ForEach-Object {
        Add $name $_
        if ($each) { $null = & $each $_ }
    }
    $code = $LASTEXITCODE
    Add $name "exit $code  $([int]((Get-Date) - $t).TotalSeconds) s"
    $script:ran[$name] = $code
    $code
}
function Lines($name) {
    $p = Join-Path $log "$name.txt"
    if (Test-Path $p) { @(Get-Content $p -Encoding UTF8) } else { @() }
}
function Grep($name, $pattern) { Lines $name | Where-Object { $_ -match $pattern } | Select-Object -First 1 }
function Snapshot() {
    Get-ChildItem $sample -Recurse -File | Where-Object { $_.FullName -notmatch '\\_milked\\' } | Sort-Object FullName |
        ForEach-Object { "$($_.FullName.Substring($sample.Length))  $($_.Length)  $((Get-FileHash $_.FullName -Algorithm SHA256).Hash)" }
}
function Tree($name) {
    Save $name (Get-ChildItem $sample -Recurse -File | Sort-Object FullName |
        ForEach-Object { "$($_.FullName.Substring($sample.Length))  $($_.Length)" })
}
function MB($bytes) { ([math]::Round($bytes / 1MB, 1)).ToString($inv) + ' MB' }

# a picture of the window this console is shown in, as drawn (QOL-10); the
# keys a person would press, queued into this console (TECH-13); the
# console's text (Phase 17); the fold
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Globalization;
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
                else if (step.StartsWith("text ")) { string s = Screen(); System.IO.File.WriteAllText(arg, s, new UTF8Encoding(false)); ok = s.Trim().Length > 0; said = s.Split('\n').Length + " lines"; }
                log.AppendLine(step + (ok ? "   ok" : "   FAILED") + (said.Length > 0 ? "   " + said : ""));
            }
            Driven = log.ToString();
        });
        driver.IsBackground = true;
        driver.Start();
    }
    public static void Join() { if (driver != null) driver.Join(60000); }
    [DllImport("kernel32.dll")] static extern bool FlushConsoleInputBuffer(IntPtr h);
    // a key queued for a prompt that did not come stays in the input; gone
    public static void Flush() { FlushConsoleInputBuffer(GetStdHandle(-10)); }

    // ---- what the console shows, as text (Phase 17): the active screen
    // buffer (CONOUT$ is the alternate screen while a program holds it),
    // the rows of its window, so a step can be checked by what it drew
    [StructLayout(LayoutKind.Sequential)] struct Coord { public short X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct Rect16 { public short Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct BufferInfo { public Coord Size; public Coord Cursor; public ushort Attributes; public Rect16 Window; public Coord Max; }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern bool GetConsoleScreenBufferInfo(IntPtr h, out BufferInfo info);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern bool ReadConsoleOutputCharacterW(IntPtr h, [Out] char[] buf, uint length, Coord at, out uint read);
    public static string Screen() {
        IntPtr h = CreateFileW("CONOUT$", 0xC0000000, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
        if (h == new IntPtr(-1)) return "";
        try {
            BufferInfo info;
            if (!GetConsoleScreenBufferInfo(h, out info)) return "";
            int w = info.Window.Right - info.Window.Left + 1;
            StringBuilder s = new StringBuilder();
            for (int y = info.Window.Top; y <= info.Window.Bottom; y++) {
                char[] row = new char[w];
                uint n;
                Coord at;
                at.X = info.Window.Left;
                at.Y = (short)y;
                ReadConsoleOutputCharacterW(h, row, (uint)w, at, out n);
                s.Append(new string(row, 0, (int)n).TrimEnd()).Append('\n');
            }
            return s.ToString();
        } finally {
            CloseHandle(h);
        }
    }

    // the share of a picture's pixels in colour: lit and saturated, not grey
    public static double Colourful(string path) {
        using (Bitmap b = new Bitmap(path)) {
            int n = 0, hit = 0;
            for (int y = 0; y < b.Height; y += 2) {
                for (int x = 0; x < b.Width; x += 2) {
                    Color c = b.GetPixel(x, y);
                    int hi = Math.Max(c.R, Math.Max(c.G, c.B)), lo = Math.Min(c.R, Math.Min(c.G, c.B));
                    n++;
                    if (hi > 90 && hi - lo > 60) hit++;
                }
            }
            return n == 0 ? 0 : (double)hit / n;
        }
    }
}

// The fold (docs/CLI.md 3a), the launcher's own: one line per workflow,
// live while it runs - an orbit, a sheen across the label, a track toward
// the count or a comet when there is none, what is in hand, the time -
// then folded to its mark and result. A thread draws it 25 times a second
// so it moves while a step is silent.
public static class Fold {
    static readonly object gate = new object();
    static string label = "", say = "";
    static int done = -1, total = -1;
    static DateTime opened = DateTime.Now;
    static bool active, held;
    static Thread ticker;
    const int LABEL = 13;
    static readonly string[] ORBIT = { "\u2881", "\u2848", "\u2814", "\u2822" };
    struct Rgb { public int R, G, B; public Rgb(int r, int g, int b) { R = r; G = g; B = b; } }
    static readonly Rgb AMBER = new Rgb(255, 176, 0), WHITE = new Rgb(255, 255, 255), PLATE = new Rgb(232, 228, 216),
        DIM = new Rgb(138, 138, 138), GROOVE = new Rgb(58, 58, 58), GREEN = new Rgb(124, 217, 146),
        RED = new Rgb(255, 77, 77), BLUE = new Rgb(95, 179, 255);
    const string ESC = "\u001b[";

    [DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int n);
    [DllImport("kernel32.dll")] static extern bool GetConsoleMode(IntPtr h, out uint mode);
    [DllImport("kernel32.dll")] static extern bool SetConsoleMode(IntPtr h, uint mode);

    // escapes on (virtual-terminal processing), the drawing thread started
    public static void Init() {
        IntPtr h = GetStdHandle(-11);
        uint mode;
        if (GetConsoleMode(h, out mode)) SetConsoleMode(h, mode | 4);
        if (ticker == null) {
            ticker = new Thread(Tick);
            ticker.IsBackground = true;
            ticker.Start();
        }
    }

    static Rgb Lerp(Rgb a, Rgb b, double k) {
        k = Math.Max(0, Math.Min(1, k));
        return new Rgb((int)Math.Round(a.R + (b.R - a.R) * k), (int)Math.Round(a.G + (b.G - a.G) * k), (int)Math.Round(a.B + (b.B - a.B) * k));
    }
    static string Fg(Rgb c, bool bold, string s) {
        return ESC + (bold ? "1;" : "") + "38;2;" + c.R + ";" + c.G + ";" + c.B + "m" + s + ESC + "0m";
    }
    static double Wave(double secs, double period) { return 0.5 + 0.5 * Math.Sin(secs / period * 2 * Math.PI); }
    static int Width() { try { return Math.Max(40, Console.WindowWidth); } catch { return 100; } }
    static void Put(string s) { Console.Out.Write(s); Console.Out.Flush(); }
    static string Cut(string s, int max) {
        if (max <= 0) return "";
        if (s.Length <= max) return s;
        return max == 1 ? "\u2026" : s.Substring(0, max - 1) + "\u2026";
    }

    // 4.2 s, 57 s, 4 m 57 s
    public static string Clock(TimeSpan d) {
        double s = d.TotalSeconds;
        if (s < 10) return s.ToString("0.0", CultureInfo.InvariantCulture) + " s";
        if (s < 600) return ((int)Math.Floor(s)) + " s";
        int m = (int)(s / 60);
        return m + " m " + (int)Math.Floor(s - m * 60) + " s";
    }

    // the run's plate, what it checks, the commit, the machine
    public static void Header(string what, string sha, string machine) {
        int room = Width() - 1 - 14 - 2 - what.Length - 3 - sha.Length - 3;
        Put(" " + ESC + "1;38;2;24;22;18;48;2;255;176;0m RUN_VERIFY " + ESC + "0m  " + Fg(AMBER, true, what) + "   "
            + Fg(BLUE, false, sha) + "   " + Fg(DIM, false, Cut(machine, room)) + "\n\n");
    }

    public static void Open(string l) {
        lock (gate) {
            label = l;
            say = "";
            done = total = -1;
            opened = DateTime.Now;
            active = true;
            Put(ESC + "?25l");
        }
    }
    public static void Say(string s) { lock (gate) { say = s; } }
    public static void Count(int d, int t) { lock (gate) { done = d; total = t; } }
    // a step that draws on the console itself: the line off, then back
    public static void Hold() { lock (gate) { held = true; Put("\r" + ESC + "K"); } }
    public static void Release() { lock (gate) { held = false; } }
    public static void Raw(string s) { lock (gate) { Put(s); } }
    // back to row top of the main screen and everything under it erased:
    // what a program printed there after leaving the alternate screen
    public static void Restore(int top) {
        lock (gate) {
            try { Console.SetCursorPosition(0, top); } catch { }
            Put("\r" + ESC + "J");
        }
    }

    static void Tick() {
        while (true) {
            Thread.Sleep(40);
            lock (gate) {
                if (!active || held) continue;
                try { Put("\r" + Live(DateTime.Now) + ESC + "K"); } catch { }
            }
        }
    }

    static string Live(DateTime now) {
        int w = Width();
        double secs = (now - opened).TotalSeconds;
        StringBuilder b = new StringBuilder(" ");
        b.Append(Fg(Lerp(AMBER, WHITE, 0.15 + 0.35 * Wave(secs, 1.1)), true, ORBIT[(int)(secs / 0.09) % 4])).Append(' ');
        string lab = label.PadRight(LABEL).Substring(0, LABEL);
        double pos = (secs % 1.8) / 1.8 * (LABEL + 8) - 4;
        for (int i = 0; i < lab.Length; i++) {
            double lit = Math.Max(0, 1 - Math.Abs(i - pos) / 2.2);
            b.Append(Fg(Lerp(AMBER, WHITE, 0.8 * lit), true, lab[i].ToString()));
        }
        b.Append(' ');
        int cells = w < 90 ? 0 : Math.Min(28, Math.Max(12, w / 5));
        if (cells > 0) {
            if (total > 0 && done >= 0) {
                double f = Math.Min(1.0, (double)done / total) * cells;
                int full = (int)Math.Floor(f);
                bool half = f - full >= 0.5;
                double gloss = (secs * 16) % (cells + 10) - 5;
                for (int i = 0; i < cells; i++) {
                    if (i < full) {
                        Rgb c = Lerp(AMBER, GROOVE, 0.4 - 0.4 * i / cells);
                        b.Append(Fg(Lerp(c, WHITE, Math.Max(0, 1 - Math.Abs(i - gloss) / 2.5) * 0.55), false, "\u2501"));
                    } else if (i == full && half) {
                        b.Append(Fg(AMBER, false, "\u2578"));
                    } else {
                        b.Append(Fg(GROOVE, false, "\u2500"));
                    }
                }
            } else {
                double head = (secs * 20) % (cells + 8) - 1;
                for (int i = 0; i < cells; i++) {
                    double d = head - i;
                    b.Append(d >= 0 && d < 7 ? Fg(Lerp(GROOVE, AMBER, 1 - d / 7), false, "\u2501") : Fg(GROOVE, false, "\u2500"));
                }
            }
            b.Append("  ");
        }
        string clock = Clock(now - opened);
        int used = 1 + 1 + 1 + LABEL + 1 + (cells > 0 ? cells + 2 : 0);
        int room = Math.Max(0, w - used - 2 - clock.Length - 1);
        string t = Cut(say, room);
        b.Append(Fg(PLATE, false, t)).Append(' ', room - t.Length + 2).Append(Fg(DIM, false, clock));
        return b.ToString();
    }

    // a result: figures lit, words dim, the dots between parts darker
    static string Paint(string s) {
        StringBuilder b = new StringBuilder();
        string[] parts = s.Split(new string[] { " \u00b7 " }, StringSplitOptions.None);
        for (int p = 0; p < parts.Length; p++) {
            if (p > 0) b.Append(Fg(Lerp(GROOVE, DIM, 0.5), false, " \u00b7 "));
            int last = 0;
            foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(parts[p], @"(?<![\w.])\d[\d,.]*( ?(MB|GB|KB|s|%))?(?!\w)")) {
                if (m.Index > last) b.Append(Fg(DIM, false, parts[p].Substring(last, m.Index - last)));
                b.Append(Fg(PLATE, true, m.Value));
                last = m.Index + m.Length;
            }
            if (last < parts[p].Length) b.Append(Fg(DIM, false, parts[p].Substring(last)));
        }
        return b.ToString();
    }

    // the line folded: 0 verified, 1 done with something to read, 2 failed.
    // Returns the line as plain text, for SUMMARY.txt.
    public static string Settle(int mark, string result) {
        lock (gate) {
            TimeSpan d = DateTime.Now - opened;
            active = false;
            int w = Width();
            string clock = Clock(d);
            int room = Math.Max(0, w - (1 + 1 + 1 + LABEL + 1) - 2 - clock.Length - 1);
            string r = Cut(result, room);
            string glyph = mark == 0 ? Fg(GREEN, true, "\u2714") : mark == 1 ? Fg(AMBER, true, "\u25b2") : Fg(RED, true, "\u2716");
            string lab = label.PadRight(LABEL).Substring(0, LABEL);
            Put("\r " + glyph + " " + Fg(mark == 2 ? RED : PLATE, false, lab) + " " + Paint(r) + new string(' ', room - r.Length + 2)
                + Fg(DIM, false, clock) + ESC + "K\n" + ESC + "?25h");
            return (mark == 0 ? "OK  " : mark == 1 ? "WARN" : "FAIL") + "  " + lab + " " + result + "   " + clock;
        }
    }

    // the run in one line: how many verified, how long, where the rest is
    public static void Final(int ok, int warn, int fail, string clock, string where) {
        lock (gate) {
            int n = ok + warn + fail;
            string head = fail > 0
                ? Fg(RED, true, " \u2716 " + fail + " of " + n + " failed")
                : Fg(GREEN, true, " \u2714 " + (ok + warn) + " of " + n + " verified");
            string rest = (warn > 0 ? Fg(AMBER, false, " \u00b7 " + warn + " to read") : "") + Fg(DIM, false, " \u00b7 " + clock);
            Put("\n" + head + rest + "\n" + Fg(DIM, false, "   every step, in order: " + where) + "\n");
        }
    }
}
"@
[Fold]::Init()

# ---- the lines: open, say what is in hand, fold with a mark (0 verified,
# 1 done with something to read, 2 failed); each also in RUN.txt and
# SUMMARY.txt
function Open($label) {
    [System.IO.File]::AppendAllText((Join-Path $log 'RUN.txt'), "`r`n==== $label`r`n", $utf8)
    [Fold]::Open($label)
}
function Settle($mark, $text) {
    $line = [Fold]::Settle($mark, $text)
    [System.IO.File]::AppendAllText((Join-Path $log 'SUMMARY.txt'), "$line`r`n", $utf8)
    [System.IO.File]::AppendAllText((Join-Path $log 'RUN.txt'), "== $line`r`n", $utf8)
}
# cargo test's lines, read live: the binary, its count, the test in hand;
# the totals from cargo's own `test result` lines
function TestCounter() {
    $script:bin = ''; $script:n = 0; $script:of = 0
    $script:passed = 0; $script:failed = 0; $script:ignored = 0; $script:firstFail = ''
}
$testLine = {
    param($l)
    if ($l -match '^\s*Compiling (\S+)') {
        [Fold]::Say("compiling $($Matches[1])")
    } elseif ($l -match '^\s*Running (?:unittests )?(\S+)') {
        $script:bin = [System.IO.Path]::GetFileNameWithoutExtension($Matches[1])
        $script:n = 0; $script:of = 0
        [Fold]::Count(0, 0)
        [Fold]::Say($script:bin)
    } elseif ($l -match '^running (\d+) tests?') {
        $script:of = [int]$Matches[1]
        [Fold]::Count(0, $script:of)
    } elseif ($l -match '^test (\S+) \.\.\. (ok|FAILED|ignored)') {
        $name = $Matches[1]; $how = $Matches[2]
        $script:n++
        if ($how -eq 'FAILED' -and -not $script:firstFail) { $script:firstFail = $name }
        [Fold]::Count($script:n, $script:of)
        [Fold]::Say("$($script:bin)  $($script:n) / $($script:of)$dot$name")
    } elseif ($l -match '^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored') {
        $script:passed += [int]$Matches[1]; $script:failed += [int]$Matches[2]; $script:ignored += [int]$Matches[3]
    }
}
function TestResult($code, $extra) {
    $text = "$($script:passed) passed$dot$($script:failed) failed"
    if ($script:ignored -gt 0) { $text += "$dot$($script:ignored) ignored" }
    if ($extra) { $text += "$dot$extra" }
    if ($script:firstFail) { $text += "${dot}first: $($script:firstFail)" }
    Settle $(if ($code -eq 0 -and $script:failed -eq 0 -and $script:passed -gt 0) { 0 } else { 2 }) $text
}
function Verdict($name, $pattern) {
    $v = Grep $name $pattern
    if ($v) { $v.Trim() } else { '' }
}
# a program that draws on this console itself (the doctor, the workspace)
# runs between these two, at the top level of the script, where its output
# goes to the console and not into a pipe: the line held, the program on
# the alternate screen, then what it printed on the main screen after
# leaving it erased, so these lines stay as they were
function ConsoleTake() {
    [Fold]::Hold()
    # room under the line first: what the program prints back on the main
    # screen then never scrolls it, so the row noted here stays this line
    [Fold]::Raw(("`n" * 8) + [string][char]27 + '[8A')
    $script:top = [Console]::CursorTop
    [Fold]::Raw([string][char]27 + '[?1049h' + [string][char]27 + '[H')
}
function ConsoleGive() {
    [Fold]::Raw([string][char]27 + '[?1049l')
    [Fold]::Restore($script:top)
    [Fold]::Release()
}

# ---- environment
if (-not $Pulled) {
    $os = Get-CimInstance Win32_OperatingSystem
    $cs = Get-CimInstance Win32_ComputerSystem
    $cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
    $ram = ([math]::Round($cs.TotalPhysicalMemory / 1GB, 1)).ToString($inv)
    [Fold]::Header('dedupe on Windows', (git rev-parse --short HEAD), "$cpu$dot$($cs.NumberOfLogicalProcessors) threads$dot$ram GB")
    Open 'environment'
    [void][DedupeWin]::SetProcessDPIAware()
    $how = $null
    $hw = [DedupeWin]::Pick($title, [ref]$how)
    $term = if ($hw -eq [IntPtr]::Zero) { 'no terminal window found' } elseif ([DedupeWin]::Class($hw) -eq 'CASCADIA_HOSTING_WINDOW_CLASS') { 'Windows Terminal' } else { 'the console host' }
    [Fold]::Say('rustc, cargo, ffmpeg')
    $rustc = (cmd /c 'rustc -V 2>&1') -replace '^rustc (\S+).*$', '$1'
    $ffmpeg = cmd /c 'ffmpeg -version 2>&1' | Select-Object -First 1
    $cols = $Host.UI.RawUI.WindowSize.Width
    Save 'env' @(
        "date      $(Get-Date -Format s)",
        "windows   $([System.Environment]::OSVersion.VersionString)  $($os.Caption)",
        "cpu       $cpu  $($cs.NumberOfLogicalProcessors) threads",
        "ram       $ram GB",
        "terminal  $term  WT_SESSION=$env:WT_SESSION  host=$($Host.Name)  psversion=$($PSVersionTable.PSVersion)  cols=$cols",
        "codepage  $([Console]::OutputEncoding.WebName)  $(cmd /c chcp)",
        "rustc     $(cmd /c 'rustc -V 2>&1')",
        "cargo     $(cmd /c 'cargo -V 2>&1')",
        "ffmpeg    on PATH: $ffmpeg   (dedupe also looks where installers put it: see doctor)",
        "git       $(git rev-parse --short HEAD)  $((git status --porcelain | Measure-Object).Count) changed paths"
    )
    Settle 0 "$($os.Caption -replace '^Microsoft ', '') $($os.BuildNumber)${dot}rustc $rustc$dot$term$dot$cols columns"

    # ---- update: the repo's runs\win over any local edit, then the pull
    Open 'update'
    $self = (Get-FileHash $PSCommandPath -Algorithm SHA256).Hash
    $before = git rev-parse --short HEAD
    [Fold]::Say('git checkout -- runs/win')
    $null = Run 'git_checkout' 'git checkout -- runs/win'
    $restored = (Get-FileHash $PSCommandPath -Algorithm SHA256).Hash -ne $self
    $mid = (Get-FileHash $PSCommandPath -Algorithm SHA256).Hash
    [Fold]::Say('git pull --ff-only origin main')
    $code = Run 'git_pull' 'git pull --ff-only origin main'
    $after = git rev-parse --short HEAD
    $updated = (Get-FileHash $PSCommandPath -Algorithm SHA256).Hash -ne $mid
    $text = if ($before -eq $after) { "already at $after" } else { "$before$arrow$after$dot$((git diff --name-only $before $after | Measure-Object).Count) files changed" }
    if ($code -ne 0) { $text = "git pull failed (exit $code), running $after" }
    if ($updated) { $text += "${dot}win_verify.ps1 updated" } elseif ($restored) { $text += "${dot}win_verify.ps1 restored from the repo" }
    Settle $(if ($code -eq 0) { 0 } else { 1 }) $text
    # PowerShell runs the copy it read at the start: a changed script runs
    # on in a fresh copy, under these lines
    if ($updated -or $restored) {
        & powershell -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath -Pulled -Started $t0.Ticks
        exit $LASTEXITCODE
    }
}

# ---- build
Open 'build'
$script:crates = 0
$code = Run 'build' 'cargo build --release' {
    param($l)
    if ($l -match '^\s*Compiling (\S+)') {
        $script:crates++
        [Fold]::Say("compiling $($Matches[1])$dot$($script:crates) crates")
    }
}
if ($code -ne 0 -or -not (Test-Path $exe)) {
    Settle 2 "no binary${dot}cargo exit $code${dot}runs\win\log\build.txt"
    Save 'DONE' 'NO BINARY'
    [Fold]::Final(0, 0, 1, [Fold]::Clock((Get-Date) - $t0), 'runs\win\log\RUN.txt')
    $null = $Host.UI.RawUI.ReadKey('NoEcho,IncludeKeyDown')
    exit 1
}
Add 'build' "binary    $((Get-Item $exe).Length) bytes"
Settle 0 "release$dot$(MB (Get-Item $exe).Length)$dot$($script:crates) crates compiled"

# ---- the suite
Open 'tests'
TestCounter
$code = Run 'test' 'cargo test --release' $testLine
TestResult $code

Open 'Ctrl-C'
$code = Run 'test_ctrlc' 'cargo test --release --test e2e interrupted_milk_leaves_a_journal_undo_can_use -- --nocapture' $testLine
$v = Verdict 'test_ctrlc' 'CTRL_BREAK sent|could not be sent|skipped'
Settle $(if ($code -ne 0) { 2 } elseif ($v -match 'CTRL_BREAK sent') { 0 } else { 1 }) $(if ($v) { $v } else { "no verdict in the log${dot}exit $code" })

Open 'terminal'
$code = Run 'test_tty' 'cargo test --release --test e2e -- --nocapture find_on_a_terminal_acts_from_the_result a_terminal_gets_colour' $testLine
$w = Verdict 'test_tty' 'driven through a pseudo-terminal'
$k = Verdict 'test_tty' 'colour on a terminal'
$parts = @($w, $k) | Where-Object { $_ }
Settle $(if ($code -ne 0) { 2 } elseif ($w -and $k) { 0 } else { 1 }) $(if ($parts) { ($parts -join $dot) -replace 'the workspace driven through a pseudo-terminal: ', 'workspace by keys: ' } else { "no verdict in the log${dot}exit $code" })

if (Test-Path $model) {
    Open 'model'
    TestCounter
    $env:DEDUPE_MODEL = $model
    $code = Run 'test_model' 'cargo test --release --test e2e' $testLine
    Remove-Item Env:\DEDUPE_MODEL
    TestResult $code
}

Open 'Recycle Bin'
TestCounter
$code = Run 'test_recycle' 'cargo test --release --test e2e -- --ignored purge_then_undo_through_the_bin' $testLine
TestResult $code

Open 'formats'
# a small part of two public test-image sets: iPhone and Sony HEIC/HEIF,
# AVIF transform files, one RAW per maker; about 90 MB, kept for next time
if (-not (Test-Path (Join-Path $formats 'mei\heic'))) {
    New-Item -ItemType Directory -Force -Path $formats | Out-Null
    [Fold]::Say('fetching the phone and camera files, about 90 MB, once')
    $null = Run 'formats_fetch_mei' "git clone -q --depth 1 --filter=blob:none --sparse https://github.com/drewnoakes/metadata-extractor-images `"$formats\mei`""
    $null = Run 'formats_sparse_mei' "git -C `"$formats\mei`" sparse-checkout set --no-cone /heic/IMG_1034.heic /heic/IMG_2927.HEIC `"/heic/Issue 263 dotnet.heic`" /heic/Issue*487.heic `"/heif/Sony ILCE-7M4 (A7M4).heif`" `"/avif/Issue 649.avif`" `"/cr3/Canon EOS R3.cr3`" `"/nef/Nikon D70.nef`" `"/arw/Sony NEX-5N.arw`" `"/dng/Leica T (Typ 701).dng`" `"/raf/Fujifilm X-E1.raf`" `"/orf/Olympus E-M5.orf`" `"/rw2/Panasonic DMC-GF7.rw2`""
    $null = Run 'formats_fetch_heif' "git clone -q --depth 1 --filter=blob:none --sparse https://github.com/strukturag/libheif `"$formats\strukturag_libheif`""
    $null = Run 'formats_sparse_heif' "git -C `"$formats\strukturag_libheif`" sparse-checkout set examples"
    $null = Run 'formats_fetch_avif' "git clone -q --depth 1 --filter=blob:none --sparse https://github.com/AOMediaCodec/av1-avif `"$formats\AOMediaCodec_av1-avif`""
    $null = Run 'formats_sparse_avif' "git -C `"$formats\AOMediaCodec_av1-avif`" sparse-checkout set --no-cone /testFiles/Link-U/ /testFiles/Microsoft/Summer_in_Tomsk_720p_5x4_grid.avif /testFiles/Microsoft/Ronda_rotate90.avif /testFiles/Microsoft/Chimera_8bit_cropped_480x256.avif /testFiles/Microsoft/Chimera_10bit_cropped_to_1920x1008.avif /testFiles/Microsoft/Monochrome.avif"
}
TestCounter
$env:DEDUPE_FORMATS = $formats
$code = Run 'test_formats' 'cargo test --release --test formats_corpus -- --ignored --nocapture' $testLine
Remove-Item Env:\DEDUPE_FORMATS
TestResult $code (Verdict 'test_formats' '^checked \d+ of \d+ files')

# ---- the commands, on the sample; logged in full (-v), every group and file
Open 'doctor'
$code = Run 'doctor' "`"$exe`" --doctor"
$checks = @(Lines 'doctor' | Where-Object { $_ -match '^\s*(PASS|WARN|FAIL)\s+\S' })
$bad = @($checks | Where-Object { $_ -notmatch '^\s*PASS' })
$ff = Verdict 'doctor' '^\s*\w+\s+ffmpeg\s'
$ffText = if (-not $ff) { 'ffmpeg not checked' } elseif ($ff -match 'not on PATH') { 'ffmpeg found outside PATH' } elseif ($ff -match '^\s*PASS') { 'ffmpeg on PATH' } else { 'ffmpeg missing' }
$deep = Verdict 'doctor' '^\s*\w+\s+deep\s'
$deepText = if ($deep -match 'no model') { 'no model' } else { 'model installed' }
$text = "$($checks.Count - $bad.Count) of $($checks.Count) checks pass$dot$ffText$dot$deepText"
if ($bad.Count -gt 0) { $text += $dot + (($bad | ForEach-Object { ($_.Trim() -split '\s+')[1] }) -join ', ') }
Settle $(if ($code -ne 0) { 2 } elseif ($bad.Count -gt 0) { 1 } else { 0 }) $text

Open 'find'
$runs = @(
    @('find_all', "`"$exe`" find $sample . -v", 'all'),
    @('find_img', "`"$exe`" find $sample . img -v", 'img'),
    @('find_vid', "`"$exe`" find $sample . vid -v", 'vid'),
    @('find_photos_flat', "`"$exe`" find $sample\photos -v", 'flat'),
    @('find_quick', "`"$exe`" find $sample . -q -v", 'quick'),
    @('find_formats', "`"$exe`" find `"$formats`" . img -v", 'formats')
)
if (Test-Path $model) { $runs += , @('find_img_model', "`"$exe`" find $sample . img -v", 'model') }
$failedRuns = @()
$i = 0
foreach ($r in $runs) {
    $i++
    [Fold]::Count($i - 1, $runs.Count)
    [Fold]::Say("find $($r[2])  $i / $($runs.Count)")
    if ($r[2] -eq 'model') { $env:DEDUPE_MODEL = $model }
    $code = Run $r[0] $r[1]
    if ($r[2] -eq 'model') { Remove-Item Env:\DEDUPE_MODEL }
    if ($code -ne 0) { $failedRuns += $r[2] }
}
$names = ($runs | ForEach-Object { $_[2] }) -join ', '
Settle $(if ($failedRuns.Count -gt 0) { 2 } else { 0 }) $(if ($failedRuns.Count -gt 0) { "$($failedRuns.Count) of $($runs.Count) runs failed: $($failedRuns -join ', ')" } else { "$($runs.Count) runs$dot$names" })

# milk then undo, purge then undo: the tree, every file's SHA-256, the same
$before = Snapshot
Save 'tree_before' $before
function RoundTrip($act, $label) {
    Open $label
    [Fold]::Say("$act, dry run")
    $null = Run "$($act)_dry" "`"$exe`" $act $sample . -n -v"
    [Fold]::Say($act)
    $c1 = Run $act "`"$exe`" $act $sample . -v"
    Tree "tree_after_$act"
    $moved = (Lines $act | Where-Object { $_ -match '^\s*(moved|binned)' }).Count
    [Fold]::Say("undo$dot$moved to put back")
    $c2 = Run "$($act)_undo" "`"$exe`" --undo $sample -v"
    $after = Snapshot
    Save "tree_after_$($act)_undo" $after
    $diff = @(Compare-Object $before $after)
    if ($diff.Count -eq 0) {
        Save "$($act)_roundtrip" "tree identical after $act+undo: $($after.Count) files, every SHA-256 equal"
        Settle $(if ($c1 -eq 0 -and $c2 -eq 0) { 0 } else { 1 }) "$moved $(if ($act -eq 'milk') { 'moved' } else { 'binned' }) and put back$dot$($after.Count) files, every SHA-256 equal to before"
    } else {
        Save "$($act)_roundtrip" (@("TREE DIFFERS after $act+undo") + ($diff | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }))
        Settle 2 "the tree differs after $act and undo$dot$($diff.Count) lines${dot}runs\win\log\$($act)_roundtrip.txt"
    }
}
RoundTrip 'milk' 'milk + undo'
$left = if (Test-Path (Join-Path $sample '_milked')) { Get-ChildItem (Join-Path $sample '_milked') -Recurse -Force | ForEach-Object { $_.FullName } } else { '_milked gone' }
Save 'milked_dir_after_undo' $left
RoundTrip 'purge' 'purge + undo'

Open 'help, errors'
$checks = @(
    @('help', "`"$exe`" --help", 0, 'dedupe', 'help'),
    @('err_two_kinds', "`"$exe`" find $sample img vid", 1, 'one kind at a time', 'two kinds refused'),
    @('err_missing', "`"$exe`" find C:\no\such\folder", 1, 'folder not found', 'missing folder refused'),
    @('undo_nothing', "`"$exe`" --undo $sample", 0, 'nothing to undo', 'nothing to undo said')
)
$said = @(); $wrong = @()
foreach ($c in $checks) {
    [Fold]::Say($c[4])
    $code = Run $c[0] $c[1]
    if ($code -eq $c[2] -and (Grep $c[0] ([regex]::Escape($c[3])))) { $said += $c[4] } else { $wrong += "$($c[4]) (exit $code)" }
}
$j = Join-Path $env:LOCALAPPDATA 'dedupe'
Save 'journal_dir' (@("app dir $j") + $(if (Test-Path $j) { Get-ChildItem $j -Recurse | ForEach-Object { "$($_.FullName)  $($_.Length)" } } else { 'absent' }))
Settle $(if ($wrong.Count -gt 0) { 2 } else { 0 }) $(if ($wrong.Count -gt 0) { "not as expected: $($wrong -join ', ')" } else { $said -join $dot })

# ---- pictures: the doctor in colour, the workspace, a filter, v; each
# checked by what the console shows (its text) and the picture's pixels
Open 'pictures'
[void][DedupeWin]::SetProcessDPIAware()
$host.UI.RawUI.WindowTitle = $title
$how = $null
$h = [DedupeWin]::Pick($title, [ref]$how)
$found = @(); $missing = @()
if ($h -eq [IntPtr]::Zero) {
    Save 'colour' "no window to capture: $([DedupeWin]::Why($title))"
    Settle 2 "no window to photograph$dot$([DedupeWin]::Why($title))"
} else {
    [Fold]::Say('the doctor, live')
    # esc, queued, answers the model offer the doctor ends on when no model
    # is installed (Phase 16, NF-09); with a model there is no offer
    ConsoleTake
    [DedupeWin]::Flush()
    [DedupeWin]::Drive("wait 4000`nesc", $h)
    & $exe --doctor
    [DedupeWin]::Join()
    [DedupeWin]::Flush()
    Start-Sleep -Milliseconds 400
    $got = [DedupeWin]::Capture($h, (Join-Path $log 'colour.png'))
    ConsoleGive
    Save 'colour' "captured $got   $how ($([DedupeWin]::Class($h)))"
    $share = if (Test-Path (Join-Path $log 'colour.png')) { [DedupeWin]::Colourful((Join-Path $log 'colour.png')) } else { 0 }
    Add 'colour' ("colour pixels {0:P2}" -f $share)
    if ($share -gt 0.001) { $found += 'the doctor in colour' } else { $missing += 'no colour in colour.png' }

    [Fold]::Say('the workspace, a filter to castle, v on the pair')
    $plan = @(
        'wait 7000', "shot $(Join-Path $log 'workspace.png')", "text $(Join-Path $log 'screen_workspace.txt')",
        'type /castle', 'enter', 'wait 800', "text $(Join-Path $log 'screen_filter.txt')",
        'type v', 'wait 4000', "shot $(Join-Path $log 'view.png')", "text $(Join-Path $log 'screen_view.txt')",
        'esc', 'wait 800', 'type q'
    ) -join "`n"
    ConsoleTake
    [DedupeWin]::Flush()
    [DedupeWin]::Drive($plan, $h)
    & $exe find (Join-Path $sample 'photos') . img
    [DedupeWin]::Join()
    [DedupeWin]::Flush()
    ConsoleGive
    Save 'view' (@("window: $how ($([DedupeWin]::Class($h)))   WT_SESSION=$env:WT_SESSION") + ([DedupeWin]::Driven -split "`r?`n" | Where-Object { $_ }))
    $ws = (Lines 'screen_workspace') -join "`n"
    $fl = (Lines 'screen_filter') -join "`n"
    $vw = (Lines 'screen_view') -join "`n"
    if (-not ($ws + $fl + $vw).Trim()) {
        $missing += 'the console text could not be read'
    } else {
        if ($ws -match 'GROUP 001' -and $ws -match 'keep') { $found += 'workspace' } else { $missing += 'no workspace on screen' }
        if ($fl -match 'castle_q60' -and $fl -notmatch 'starry_night') { $found += 'the filter shows castle only' } else { $missing += 'the filter shows other groups' }
        if ($vw -match 'VIEW' -and $vw -notmatch 'no inline graphics') { $found += 'v drew the pair inline' }
        elseif ($vw -match 'no inline graphics') { $missing += 'v opened the system viewer: no inline graphics found' }
        else { $missing += 'no view on screen' }
    }
    # what failed first: a line cut at the edge keeps it
    if ($missing.Count -gt 0) {
        Settle 2 ((@($missing) + @($found)) -join $dot)
    } else {
        Settle 0 "$($found -join $dot)${dot}colour.png, workspace.png, view.png"
    }
}

Save 'DONE' "DONE $(Get-Date -Format s)"

# ---- the run in one line, then the live workspace
$lines = Lines 'SUMMARY'
$ok = @($lines | Where-Object { $_ -match '^OK ' }).Count
$warn = @($lines | Where-Object { $_ -match '^WARN' }).Count
$fail = @($lines | Where-Object { $_ -match '^FAIL' }).Count
[Fold]::Final($ok, $warn, $fail, [Fold]::Clock((Get-Date) - $t0), 'runs\win\log\RUN.txt')
Add 'SUMMARY' "$ok verified, $warn to read, $fail failed   $([Fold]::Clock((Get-Date) - $t0))   $(git rev-parse --short HEAD)"
[Fold]::Raw("`n   " + [string][char]27 + '[38;2;255;176;0mpress a key' + [string][char]27 + '[0m to open the live workspace on the sample (q leaves it)')
$null = $Host.UI.RawUI.ReadKey('NoEcho,IncludeKeyDown')
[Fold]::Raw("`r" + [string][char]27 + '[K')
ConsoleTake
& $exe find $sample .
ConsoleGive
[Fold]::Raw('   press a key to close')
$null = $Host.UI.RawUI.ReadKey('NoEcho,IncludeKeyDown')
