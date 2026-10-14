# Dev tool: screenshot every workspace tab at several window sizes for layout review.
# Usage: cargo build; .\scripts\UI-Survey.ps1 [-Sizes max,1280x800] [-Skill Beginner] [-Wizard]
# Output: %TEMP%\<prefix>-<size>-<tab>.png. Restores target\debug\wideband-ux-prefs.json afterwards.
param(
    [string[]]$Sizes = @("max", "1600x900", "1280x800", "1024x720"),
    [string[]]$Tabs = @("Calibration", "Telemetry", "Strategy", "Transmission", "Flash & Diag", "Switchboard"),
    [string]$Prefix = "sv",
    [string]$Skill = "Advanced",
    [switch]$Wizard
)
# Screenshots every tab at every window size. Tabs are clicked through UI Automation
# (egui exposes widgets via AccessKit), so it survives header reflow at small widths.
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes
if (-not ("WbWin" -as [type])) {
Add-Type @'
using System; using System.Runtime.InteropServices;
public class WbWin {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int hh, uint f);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
}
'@
}
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$prefs = "$root\target\debug\wideband-ux-prefs.json"
$first = if ($Wizard) { "false" } else { "true" }
"{ `"first_run_complete`": $first, `"skill_level`": `"$Skill`" }" | Set-Content -Encoding ascii $prefs
Get-Process wideband -ErrorAction SilentlyContinue | Stop-Process -Force
$p = Start-Process -FilePath "$root\target\debug\wideband.exe" -WorkingDirectory $root -PassThru
Start-Sleep 7; $p.Refresh(); $h = $p.MainWindowHandle

function Shot($name) {
    [WbWin]::SetCursorPos(2, 2) | Out-Null; Start-Sleep -Milliseconds 700
    $r = New-Object WbWin+RECT; [WbWin]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $hh = $r.B - $r.T
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen((New-Object System.Drawing.Point $r.L, $r.T), [System.Drawing.Point]::Empty, (New-Object System.Drawing.Size $w, $hh))
    $bmp.Save("$env:TEMP\$name.png")
}
function ClickNamed($name) {
    $rootEl = [System.Windows.Automation.AutomationElement]::FromHandle($h)
    $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $name)
    $el = $rootEl.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
    if (-not $el) { return $false }
    try {
        $el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    } catch {
        $b = $el.Current.BoundingRectangle
        [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point ([int]($b.X + $b.Width / 2)), ([int]($b.Y + $b.Height / 2))
        Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern void mouse_event(uint f,uint x,uint y,uint d,UIntPtr e);' -Name M -Namespace W -ErrorAction SilentlyContinue
        [W.M]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 60; [W.M]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
    }
    Start-Sleep -Milliseconds 900
    return $true
}
try {
    foreach ($size in $Sizes) {
        if ($size -eq "max") {
            [WbWin]::ShowWindow($h, 3) | Out-Null
        } else {
            $wh = $size -split "x"
            [WbWin]::ShowWindow($h, 1) | Out-Null; Start-Sleep -Milliseconds 300
            [WbWin]::SetWindowPos($h, [IntPtr]::Zero, 40, 20, [int]$wh[0], [int]$wh[1], 0x0040) | Out-Null
        }
        [WbWin]::SetForegroundWindow($h) | Out-Null; Start-Sleep 1
        foreach ($tab in $Tabs) {
            $ok = ClickNamed $tab
            $safe = ($tab -replace '[^A-Za-z]', '')
            Shot "$Prefix-$size-$safe"
            "{0,-10} {1,-14} {2}" -f $size, $tab, $(if ($ok) { "ok" } else { "NOT FOUND" })
        }
    }
} finally {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    "{`n  `"first_run_complete`": false,`n  `"skill_level`": `"Beginner`"`n}" | Set-Content -Encoding ascii $prefs
}
