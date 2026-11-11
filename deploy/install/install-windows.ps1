# Lighthouse installer for Windows.
#
# Download this file, then right-click it and choose "Run with PowerShell". It:
#   1. downloads Lighthouse into C:\Lighthouse (checked against the release's checksums),
#   2. asks you a few questions (your game servers, your Discord bot, a dashboard password),
#   3. makes Lighthouse start with Windows and keep running in the background,
#   4. opens the dashboard in your browser.
#
# Already installed? Running it again updates Lighthouse and keeps all your settings and data.
# Other folder: -Folder "D:\Lighthouse".  Remove: -Uninstall (keeps your settings and data).
param(
    [string]$Folder = "C:\Lighthouse",
    [ValidateSet("stable", "bleeding-edge")][string]$Channel = "stable",
    [switch]$Uninstall
)
$ErrorActionPreference = "Stop"
$Repo = "Hailey-Ross/hails.Lighthouse"
$TaskName = "Lighthouse"
$Asset = "lighthouse-x86_64-pc-windows-msvc.exe"
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

function Write-Step($text) { Write-Host ""; Write-Host $text -ForegroundColor Cyan }
function Pause-End { Write-Host ""; Read-Host "Press Enter to close this window" | Out-Null }

# Administrator rights are needed to make Lighthouse start with Windows.
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) {
    $pass = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$PSCommandPath`"", "-Folder", "`"$Folder`"", "-Channel", $Channel)
    if ($Uninstall) { $pass += "-Uninstall" }
    Start-Process powershell -Verb RunAs -ArgumentList $pass
    exit
}

try {
    if ($Uninstall) {
        if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
            Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
            Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
        }
        Get-Process lighthouse -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$Folder\*" } | Stop-Process -Force
        Write-Host "Lighthouse is stopped and won't start with Windows any more."
        Write-Host "Your settings and data are still in $Folder. Delete that folder to remove them too."
        Pause-End
        exit
    }

    $exe = Join-Path $Folder "lighthouse.exe"
    $config = Join-Path $Folder "lighthouse.toml"
    $base = "https://github.com/$Repo/releases/download/$Channel"

    if (Test-Path $exe) {
        # ---------- already installed: update, keeping everything ----------
        Write-Step "Lighthouse is already installed in $Folder. Updating it (your settings and data stay)."
        $updater = Join-Path ([IO.Path]::GetTempPath()) "lighthouse-update-windows.ps1"
        Invoke-WebRequest -UseBasicParsing "$base/update-windows.ps1" -OutFile $updater
        & powershell -NoProfile -ExecutionPolicy Bypass -File $updater -Folder $Folder -Channel $Channel
        Remove-Item -Force $updater -ErrorAction SilentlyContinue
    } else {
        # ---------- download ----------
        Write-Step "Downloading Lighthouse"
        New-Item -ItemType Directory -Force $Folder | Out-Null
        $tmp = Join-Path ([IO.Path]::GetTempPath()) ("lighthouse-install-" + [Guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory $tmp | Out-Null
        Invoke-WebRequest -UseBasicParsing "$base/$Asset" -OutFile "$tmp\$Asset"
        Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS" -OutFile "$tmp\SHA256SUMS"
        $want = (Get-Content "$tmp\SHA256SUMS" | Where-Object { $_ -match [regex]::Escape($Asset) + '$' } |
            Select-Object -First 1) -split '\s+' | Select-Object -First 1
        if (-not $want -or (Get-FileHash -Algorithm SHA256 "$tmp\$Asset").Hash -ne $want.ToUpper()) {
            throw "The download didn't match the release's checksum. Please try again."
        }
        Copy-Item "$tmp\$Asset" $exe
        Remove-Item -Recurse -Force $tmp
        Write-Host "   $((& $exe --version) -join ' ')"
    }

    # ---------- questions (first install only) ----------
    Set-Location $Folder
    if (-not (Test-Path $config)) {
        Write-Step "A few questions to set things up"
        Write-Host "Type your answer and press Enter. Use the arrow keys where you're given a list."
        Write-Host "Not sure about something? Pick the quick setup: you can do the rest in the dashboard."
        & $exe --config $config setup
        if (-not (Test-Path $config)) { throw "Setup was not finished. Run this installer again to start over." }
    }
    & $exe --config $config check-config
    if ($LASTEXITCODE -ne 0) { throw "The settings have a problem (see above). Fix it on the dashboard's Setup page or run this installer again." }
    # The settings can hold your bot token: only administrators and Windows itself may read them.
    icacls $config /inheritance:r /grant:r "*S-1-5-32-544:F" "*S-1-5-18:F" | Out-Null

    # ---------- keep it running ----------
    Write-Step "Making Lighthouse start with Windows"
    $runner = Join-Path $Folder "run-lighthouse.ps1"
    @'
# Keeps Lighthouse running (started by the "Lighthouse" Windows task). When Lighthouse exits to apply
# new settings or an update, it starts again a few seconds later. Its log is lighthouse.log here.
$Folder = Split-Path -Parent $PSCommandPath
Set-Location $Folder
$env:LIGHTHOUSE_SUPERVISED = "1"
while ($true) {
    if (Test-Path "$Folder\lighthouse.log") { Move-Item -Force "$Folder\lighthouse.log" "$Folder\lighthouse.old.log" }
    $p = Start-Process "$Folder\lighthouse.exe" -ArgumentList "--config", "`"$Folder\lighthouse.toml`"", "run" `
        -NoNewWindow -PassThru -RedirectStandardOutput "$Folder\lighthouse.log" -RedirectStandardError "$Folder\lighthouse.err.log"
    try { $p.PriorityClass = 'BelowNormal' } catch {}
    $p.WaitForExit()
    Start-Sleep -Seconds 5
}
'@ | Set-Content -Encoding utf8 $runner
    if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
        Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    }
    $action = New-ScheduledTaskAction -Execute "powershell.exe" `
        -Argument "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$runner`"" -WorkingDirectory $Folder
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) -Priority 7
    $principal = New-ScheduledTaskPrincipal -UserId "SYSTEM" -LogonType ServiceAccount -RunLevel Highest
    Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger (New-ScheduledTaskTrigger -AtStartup) `
        -Settings $settings -Principal $principal -Force | Out-Null
    Start-ScheduledTask -TaskName $TaskName

    # ---------- done ----------
    Write-Host ""
    Write-Host "All done. Lighthouse is running and starts by itself when the computer starts." -ForegroundColor Green
    Write-Host "Its folder: $Folder   (its log: lighthouse.log)"
    $text = Get-Content $config -Raw
    if ($text -match '(?ms)^\[dashboard\].*?^enabled\s*=\s*true') {
        $bind = if ($text -match '(?ms)^\[dashboard\].*?^bind\s*=\s*"([^"]+)"') { $Matches[1] } else { "127.0.0.1:8650" }
        $url = "http://" + ($bind -replace '^0\.0\.0\.0', '127.0.0.1')
        Write-Host "Opening the dashboard: $url"
        Start-Sleep -Seconds 4
        Start-Process $url
    }
} catch {
    Write-Host ""
    Write-Host "Something went wrong: $($_.Exception.Message)" -ForegroundColor Red
}
Pause-End
